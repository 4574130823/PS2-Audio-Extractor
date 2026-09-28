"""Test files for EA SCHl/BNK/ABK/HDR+DAT/MPF+MUS/SWVR, Eurocom MUSX, Ubisoft HX/SB.

Run from anywhere: python tools/fixtures/ea_ubi.py
"""
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import *  # noqa: F401,F403,E402

D = out_dir("ea_ubi")


# ---------------------------------------------------------------- EA-XA encoders
XA_C1 = [0, 240, 460, 392]
XA_C2 = [0, 0, -208, -220]


def clamp16(v):
    return max(-32768, min(32767, v))


def _xa_best(fr, h1, h2, v2):
    """Best (coef, shift, nibbles, h1, h2) for 28 samples."""
    rnd = 0 if v2 else 128
    best = None
    for c in range(4):
        for s in range(13):
            a, b, err, nibs = h1, h2, 0, []
            for x in fr:
                pred = XA_C1[c] * a + XA_C2[c] * b + rnd
                # sample = ((nib << (20 - s)) + pred) >> 8  ~= nib * 2^(12-s) + pred/256
                q = round((x * 256 - pred) / (1 << (20 - s)))
                q = max(-8, min(7, q))
                y = clamp16(((q << (20 - s)) + pred) >> 8)
                err += (y - x) ** 2
                b, a = a, y
                nibs.append(q & 0xF)
            if best is None or err < best[0]:
                best = (err, c, s, nibs, a, b)
    return best[1:]


def eaxa_mono(samples, v2=False, pcm_frames=()):
    """EA-XA v1 (0x0f frames) or v2 (with PCM 0xEE frames at indexes `pcm_frames`)."""
    out = bytearray()
    h1 = h2 = 0
    for fi in range(0, max(len(samples), 1), 28):
        fr = (samples[fi:fi + 28] + [0] * 28)[:28]
        if v2 and fi // 28 in pcm_frames:
            out += b"\xEE" + struct.pack(">hh", fr[27], fr[26]) + b"".join(struct.pack(">h", x) for x in fr)
            h1, h2 = fr[27], fr[26]
            continue
        c, s, nibs, h1, h2 = _xa_best(fr, h1, h2, v2)
        out += bytes([(c << 4) | s]) + bytes((nibs[i] << 4) | nibs[i + 1] for i in range(0, 28, 2))
    return bytes(out)


def eaxa_stereo(left, right):
    """EA-XA v1 stereo: one 0x1e frame for both channels."""
    out = bytearray()
    hl = [0, 0]
    hr = [0, 0]
    n = max(len(left), len(right))
    for fi in range(0, n, 28):
        fl = (left[fi:fi + 28] + [0] * 28)[:28]
        frr = (right[fi:fi + 28] + [0] * 28)[:28]
        cl, sl, nl, hl[0], hl[1] = _xa_best(fl, hl[0], hl[1], False)
        cr, sr, nr, hr[0], hr[1] = _xa_best(frr, hr[0], hr[1], False)
        out += bytes([(cl << 4) | cr, (sl << 4) | sr]) + bytes((nl[i] << 4) | nr[i] for i in range(28))
    return bytes(out)


# ---------------------------------------------------------------- EA headers
def patch(t, v, n=None):
    if n is None:
        n = 1 if v < 0x100 else 2 if v < 0x10000 else 3 if v < 0x1000000 else 4
    return bytes([t, n]) + v.to_bytes(n, "big") if n else bytes([t, 0])


def pt_header(platform, fields, gstr=False):
    """PT header: platform + patches (list of (type, value)), ends with 0xFF, padded to 4."""
    h = bytearray(b"GSTR" + bytes(4) if gstr else b"PT" + struct.pack("<H", platform))
    h += b"\xFD"
    for t, v in fields:
        h += patch(t, v)
    h += b"\xFF"
    return bytes(h)


def schl(platform, fields, blocks, lang=None, be_sizes=False):
    """SCHl stream: header, SCCl, SCDl blocks (already built bodies), SCEl."""
    hdr = pt_header(platform, fields)
    hid = b"SH" + lang if lang else b"SCHl"
    cid, did, eid = (b"SC" + lang, b"SD" + lang, b"SE" + lang) if lang else (b"SCCl", b"SCDl", b"SCEl")
    pk = (lambda v: struct.pack(">I", v)) if be_sizes else (lambda v: struct.pack("<I", v))
    hsize = (8 + len(hdr) + 3) // 4 * 4
    out = bytearray(hid + pk(hsize) + hdr)
    out = out.ljust(hsize, b"\0")
    out += cid + pk(12) + struct.pack("<I", len(blocks))
    for body in blocks:
        body = pad(body, 4)
        out += did + pk(8 + len(body)) + body
    out += eid + pk(8)
    return bytes(out)


def split(chans, frame_samples, per_block):
    """Channel samples cut into blocks of `per_block` samples."""
    n = len(chans[0])
    return [[c[i:i + per_block] for c in chans] for i in range(0, n, per_block)]


def schl_psx_v1(chans, rate, per_block=28 * 40, loop=None, platform=5, extra=()):
    """PS2 (v1, offsets per channel) PS-ADPCM stream."""
    ch = len(chans)
    blocks = []
    for part in split(chans, 28, per_block):
        datas = [psx_encode(p + [0] * ((-len(p)) % 28), lead=False, end=False) for p in part]
        samples = len(datas[0]) // 16 * 28
        offs, rel = b"", 0
        for d in datas:
            offs += struct.pack("<I", rel)
            rel += len(d)
        # PS-ADPCM block samples come from the size (vgmstream ignores the field)
        blocks.append(struct.pack("<I", samples) + offs + b"".join(datas))
    n = len(chans[0])
    fields = [(0x82, ch), (0x84, rate), (0x85, n)] + list(extra)
    if loop:
        fields += [(0x86, loop[0]), (0x87, loop[1] - 1)]
    return schl(platform, fields, blocks)


def schl_psx_v0(chans, rate, per_block=28 * 30):
    """PS1-style (v0) PS-ADPCM stream: interleaved halves after a 0x10 block header."""
    blocks = []
    for part in split(chans, 28, per_block):
        datas = [psx_encode(p + [0] * ((-len(p)) % 28), lead=False, end=False) for p in part]
        blocks.append(struct.pack("<II", len(datas[0]) // 16 * 28, 0) + b"".join(datas))
    return schl(1, [(0x82, len(chans)), (0x84, rate), (0x85, len(chans[0]))], blocks)


def schl_eaxa(chans, rate, version, per_block=28 * 50, platform=5, codec2=0x0A, v2_pcm=()):
    """EA-XA split mono channels with offsets (v1+: EA_XA_int with 4-byte hist; v3: EA-XA v2)."""
    ch = len(chans)
    v2 = version == 3
    blocks = []
    for bi, part in enumerate(split(chans, 28, per_block)):
        samples = len(part[0])
        datas = []
        for p in part:
            d = eaxa_mono(p, v2=v2, pcm_frames=v2_pcm if bi == 0 else ())
            datas.append((b"\0" * 4 if not v2 else b"") + d)  # v1: hist before each channel
        offs, rel = b"", 0
        for d in datas:
            offs += struct.pack("<I", rel)
            rel += len(d)
        blocks.append(struct.pack("<I", samples) + offs + b"".join(datas))
    fields = [(0x80, version), (0x82, ch), (0x84, rate), (0x85, len(chans[0])), (0xA0, codec2)]
    return schl(platform, fields, blocks)


def schl_eaxa_stereo_v0(left, right, rate, per_block=28 * 40):
    """PC v0 EA-XA stereo (codec1 EAXA on PC -> shared stereo frames), hists then data."""
    blocks = []
    for part in split([left, right], 28, per_block):
        blocks.append(struct.pack("<I", len(part[0])) + bytes(8) + eaxa_stereo(part[0], part[1]))
    return schl(0, [(0x82, 2), (0x84, rate), (0x85, len(left)), (0x83, 7)], blocks)


def schl_pcm16(chans, rate, per_block=500):
    """PS2 v1 PCM16LE split (codec2 S16LE) with offsets."""
    blocks = []
    for part in split(chans, 1, per_block):
        datas = [pcm16le(p) for p in part]
        offs, rel = b"", 0
        for d in datas:
            offs += struct.pack("<I", rel)
            rel += len(d)
        blocks.append(struct.pack("<I", len(part[0])) + offs + b"".join(datas))
    return schl(5, [(0x82, len(chans)), (0x84, rate), (0x85, len(chans[0])), (0xA0, 0x08)], blocks)


def bnk_ps2(sounds, version=4, big=False):
    """BNKl bank; sounds: list of (chans, rate, loop, flag100) or None for an empty slot."""
    e = ">" if big else "<"
    n = len(sounds)
    table_off = 0x14 if version >= 4 else 0x0c
    headers, datas = [], []
    for s in sounds:
        if s is None:
            headers.append(None)
            datas.append(None)
            continue
        chans, rate, loop, iop = s
        chd = [psx_encode(c, lead=False, end=False) for c in chans]
        if iop:
            chd = [bytes(0x10) + d for d in chd]
        datas.append(chd)
        headers.append((chans, rate, loop, iop))
    # layout: header area (fixed size per entry), then data
    hdr_area = table_off + 4 * n
    ent_size = 0x40
    data_start = (hdr_area + ent_size * n + 0x7F) // 0x80 * 0x80
    out = bytearray(data_start)
    pos = data_start
    body = bytearray()
    for i, h in enumerate(headers):
        if h is None:
            continue
        chans, rate, loop, iop = h
        offs = []
        for d in datas[i]:
            offs.append(pos + len(body))
            body += d
            body += bytes((-len(body)) % 0x10)
        fields = [(0x82, len(chans)), (0x84, rate), (0x85, len(chans[0])), (0x88, offs[0])]
        if len(offs) > 1:
            fields.append((0x89, offs[1]))
        if iop:
            fields.append((0x8C, 0x100))
        if loop:
            fields += [(0x86, loop[0]), (0x87, loop[1] - 1)]
        ph = pt_header(5, fields)
        at = hdr_area + ent_size * i
        out[at:at + len(ph)] = ph
        entry = table_off + 4 * i
        out[entry:entry + 4] = struct.pack(e + "I", at - entry)
    out[0:8] = (b"BNKb" if big else b"BNKl") + bytes([version, 0]) + struct.pack(e + "H", n)
    out[8:12] = struct.pack(e + "I", len(out) + len(body))
    return bytes(out + body)


def build_ea():
    r = 22050
    a = tone(r, 0.35, [440, 660])
    b = sweep(r, 0.35, 300, 3000)
    c = tone(r, 0.3, [220])
    write(D, "schl/ps2_psx_mono.asf", schl_psx_v1([a], r, loop=(1000, 7000)))
    write(D, "schl/ps2_psx_stereo.sng", schl_psx_v1([a, b], 32000, per_block=28 * 64))
    write(D, "schl/ps1_psx_v0.asf", schl_psx_v0([c, a[:len(c)]], r))
    write(D, "schl/ps2_eaxa_v1.asf", schl_eaxa([a, b], r, 1))
    write(D, "schl/eaxa_v2_pcmblocks.asf", schl_eaxa([c], r, 3, v2_pcm=(0, 3)))
    write(D, "schl/pc_eaxa_stereo_v0.asf", schl_eaxa_stereo_v0(a, b, r))
    write(D, "schl/ps2_pcm16.asf", schl_pcm16([c[:3000], a[:3000]], 16000))
    # multi-language video audio (SHEN/SHFR headers, then SCxx/SDxx blocks)
    en = schl_psx_v1([a], r)
    write(D, "schl/lang.asf", en.replace(b"SCHl", b"SHEN", 1).replace(b"SCCl", b"SCEN").replace(b"SCDl", b"SDEN").replace(b"SCEl", b"SEEN"))
    write(D, "bnk/ps2.bnk", bnk_ps2([([a], r, (0, len(a)), False), None, ([b, c + [0] * (len(b) - len(c))], 24000, None, False),
                                     ([c], 11025, None, True)]))
    write(D, "bnk/ps2_v2.bnk", bnk_ps2([([c], r, None, False), ([a], 16000, (28 * 10, 28 * 100), False)], version=2))


def abk_file(bnk, entries_by_table, big=False):
    """ABKC: one module whose players point at sample tables (lists of (type, a, b)); the BNK
    goes after the tables."""
    e = ">" if big else "<"
    tables = list(entries_by_table)
    players = [0, 1, 0]  # the third player repeats table 0 (vgmstream skips it)
    mt = 0x40
    module = bytearray(0x3c + 4 * len(players))
    module[0x24] = len(players)
    module[0x27] = 0
    data_base = mt + len(module)
    # module_data = data_base; player structs: 8 bytes each, +4 = samples table offset
    pl = bytearray(8 * len(players))
    tbl_base = data_base + len(pl)
    tbl = bytearray()
    toffs = []
    for t in tables:
        toffs.append(tbl_base + len(tbl))
        tbl += struct.pack(e + "I", len(t))
        for (ty, a, b) in t:
            tbl += bytes([ty, 0, 0, 0]) + struct.pack(e + "II", a, b)
    for j, ti in enumerate(players):
        module[0x3c + 4 * j:0x40 + 4 * j] = struct.pack(e + "I", 8 * j)
        pl[8 * j + 4:8 * j + 8] = struct.pack(e + "I", toffs[ti])
    module[0x2c:0x30] = struct.pack(e + "I", data_base)
    body = bytes(module) + bytes(pl) + bytes(tbl)
    bnk_off = (mt + len(body) + 0xF) // 0x10 * 0x10
    hdr = bytearray(0x40)
    hdr[0:4] = b"ABKC"
    hdr[0x0a:0x0c] = struct.pack(e + "H", 1)
    hdr[0x1c:0x20] = struct.pack(e + "I", mt)
    hdr[0x20:0x24] = struct.pack(e + "I", bnk_off)
    out = bytes(hdr) + body
    return out + bytes(bnk_off - len(out)) + bnk


def build_abk():
    r = 22050
    a = tone(r, 0.25, [500])
    b = sweep(r, 0.25, 400, 2000)
    c = tone(r, 0.2, [300, 900])
    bnk = bnk_ps2([([c], r, None, False), ([a], r, None, False), ([b], 16000, (0, 2000), False)])
    s1 = schl_psx_v1([a], r)
    s2 = schl_psx_v1([b], r)
    s3 = schl_psx_v1([c], r, loop=(100, 4000))
    ast = s1 + s2 + s3
    o1, o2, o3 = 0, len(s1), len(s1) + len(s2)
    t0 = [(0, 1, 0), (0, 0, 0), (1, o3, 0), (2, o1, o2), (0, 2, 0)]
    t1 = [(0, 1, 0), (1, o1, 0)]
    abk = abk_file(bnk, [t0, t1])
    write(D, "abk/bank.abk", abk)
    write(D, "abk/bank.ast", ast)
    # .AMB container (007: From Russia with Love): v8 header, ABKC at 0x60
    amb = bytearray(0x60)
    amb[0x08:0x0c] = struct.pack("<I", 0x20)
    amb[0x20:0x24] = struct.pack("<I", 8)
    amb[0x24:0x28] = struct.pack("<I", len(abk))
    write(D, "amb/level.amb", bytes(amb) + abk + b"MOIR" + bytes(12))
    write(D, "amb/level.ast", ast)


def hdr_dat_v1(sounds, mult_code=0, params=(7,)):
    mult = mult_code * 0x100 + 0x100
    dat = bytearray()
    table = bytearray()
    for i, s in enumerate(sounds):
        dat += bytes((-len(dat)) % mult)
        table += struct.pack(">H", len(dat) // mult) + bytes([params[0] + i] * len(params))
        dat += s
    dat += bytes((-len(dat)) % mult)
    h = struct.pack(">HH", 0x1234, 0) + bytes([len(params), len(sounds), 0, mult_code]) + struct.pack("<H", len(dat) // mult)
    h += struct.pack(">H", 0) + table
    return h, bytes(dat)


def hdr_dat_v2(sounds, mult_code=1, params=(3, 4)):
    mult = mult_code * 0x100 + 0x100
    dat = bytearray()
    table = bytearray()
    for i, s in enumerate(sounds):
        dat += bytes((-len(dat)) % mult)
        table += struct.pack(">H", len(dat) // mult) + bytes(p + i for p in params)
        dat += s
    dat += bytes((-len(dat)) % mult)
    h = struct.pack(">H", 0x55) + bytes([len(params), len(sounds)]) + struct.pack(">I", 1) + bytes([0, mult_code])
    h += struct.pack("<H", len(dat) // mult) + struct.pack(">I", 0) + table
    return h, bytes(dat)


def build_hdr_dat():
    r = 22050
    a = tone(r, 0.2, [600])
    b = sweep(r, 0.2, 500, 2500)
    h, d = hdr_dat_v1([schl_psx_v1([a], r), schl_psx_v1([b], r), schl_eaxa([a], r, 1)])
    write(D, "hdrdat/speech1.hdr", h)
    write(D, "hdrdat/speech1.dat", d)
    # Need for Speed: Hot Pursuit 2 (PS2): VAGp files in the .DAT (found by the VAG format)
    vag = lambda smp: b"VAGp" + struct.pack(">IIII", 0x20, 0, len(psx_encode(smp)), r) + bytes(28) + psx_encode(smp)
    h, d = hdr_dat_v1([vag(a), vag(b)], mult_code=1)
    write(D, "hdrdat/nfs.hdr", h)
    write(D, "hdrdat/nfs.dat", d)
    write(D, "hdrdat/speech1.hdr", h)
    write(D, "hdrdat/speech1.dat", d)
    h, d = hdr_dat_v2([schl_psx_v1([b], r), schl_psx_v1([a, b], r)])
    write(D, "hdrdat/speech2.hdr", h)
    write(D, "hdrdat/speech2.dat", d)


def build_mpf():
    r = 22050
    a = tone(r, 0.2, [440])
    b = sweep(r, 0.2, 300, 1500)
    c = tone(r, 0.15, [800])
    # MPF v3.1 (SSX Tricky), little endian, one track, 2 samples (offsets in 4-byte units)
    s1, s2 = schl_psx_v1([a], r), schl_psx_v1([b, a], r)
    mus = s1 + bytes((-len(s1)) % 4) + s2
    o2 = (len(s1) + 3) // 4
    m = bytearray(0x3c)
    m[0:4] = b"xDFP"
    m[4], m[5] = 3, 1
    m[0x0d] = 1
    m[0x12:0x14] = struct.pack("<H", 1)
    m[0x24:0x26] = struct.pack("<H", 0x28 // 4)
    m[0x28 + 0x0b] = 0
    m[0x34:0x38] = struct.pack("<I", 0x38 // 4)
    m[0x38:0x3c] = struct.pack("<I", 0x3c // 4)
    m += struct.pack("<II", 0, 100) + struct.pack("<II", o2, 100)
    write(D, "mpf/tricky.mpf", bytes(m))
    write(D, "mpf/tricky.mus", mus)
    # .MSB container: PFDx at 0x50, names its .MUS
    msb = bytearray(0x50)
    msb[0x08:0x0c] = struct.pack("<I", 0x20)
    msb[0x20:0x24] = struct.pack("<I", 5)
    msb[0x24:0x28] = struct.pack("<I", len(m))
    msb[0x30:0x30 + len(b"tricky.mus")] = b"tricky.mus"
    write(D, "mpf/level.msb", bytes(msb) + bytes(m))
    # MPF v5 (SSX On Tour style names), little endian: track 0 streamed in moments0.mus,
    # track 1 RAM (BNK at 0x100 of main.mus, samples made of 2 and 1 bank sounds)
    t0 = schl_psx_v1([a], r) 
    t0b = schl_psx_v1([c], r)
    mus0 = t0 + bytes((-len(t0)) % 0x80) + t0b
    bnk = bnk_ps2([([a], r, None, False), ([b], r, None, False), ([c], r, None, False)])
    mus1 = bytes(0x100) + bnk
    hdr = bytearray(0x40)
    hdr[0:4] = b"xDFP"
    hdr[4], hdr[5] = 5, 1
    hdr[0x0d] = 2
    tracks_table = 0x40
    tracks_data = 0x48
    # track entries: 0x18 + 0x10 bytes each
    e0 = bytearray(0x28)
    e1 = bytearray(0x28)
    e0[0:4] = struct.pack("<I", 0)
    e1[0:4] = struct.pack("<I", 2)
    e1[4:6] = struct.pack("<H", 1)
    samples_table = tracks_data + len(e0) + len(e1)
    samples = [((len(t0) + 0x7f) // 0x80) * 0 + 0, (len(t0) + 0x7f) // 0x80, (0 << 16) | 0, (0 << 16) | 2]
    hdr[0x2c:0x30] = struct.pack("<I", tracks_table)
    hdr[0x30:0x34] = struct.pack("<I", tracks_data)
    hdr[0x34:0x38] = struct.pack("<I", samples_table)
    hdr[0x38:0x3c] = struct.pack("<I", samples_table + 8 * len(samples))
    body = bytes(hdr) + struct.pack("<II", tracks_data // 4, (tracks_data + len(e0)) // 4) + bytes(e0) + bytes(e1)
    body += b"".join(struct.pack("<II", s, 100) for s in samples)
    write(D, "mpf/SSX4.mpf", body)
    write(D, "mpf/moments0.mus", mus0)
    write(D, "mpf/main.mus", mus1)
    # MPF v4 (SSX 3 / NFS Underground 2), little endian, one track, offsets in 0x80 units
    s1, s2 = schl_psx_v1([c], r), schl_psx_v1([a, b], r)
    mus = s1 + bytes((-len(s1)) % 0x80) + s2
    m = bytearray(0x64)
    m[0:4] = b"xDFP"
    m[4] = 4
    m[0x0d] = 1
    m[0x0f] = 1
    m[0x12:0x14] = struct.pack("<H", 1)
    m[0x20:0x22] = struct.pack("<H", 0x24 // 4)
    m[0x34:0x36] = struct.pack("<H", 0x38 // 4)
    m[0x48:0x4c] = struct.pack("<I", 0x4c // 4)
    m[0x4c:0x54] = struct.pack("<II", 0x54 // 4, 0x64 // 4)
    m[0x54:0x64] = struct.pack("<IIII", 0, 50, (len(s1) + 0x7f) // 0x80, 50)
    write(D, "mpf/ssx3.mpf", bytes(m))
    write(D, "mpf/ssx3.mus", mus)
    # MAP (NFS/SSX era, big endian PFDx v1)
    s1, s2 = schl_psx_v1([b], r), schl_psx_v1([c], r)
    mus = s1 + s2
    mp = bytearray(b"PFDx" + bytes([1, 0, 2, 1, 0, 0, 0, 1]))
    mp += bytes(0x1c * 2) + bytes(1 * 1) + struct.pack(">II", 0, len(s1))
    write(D, "map/track.map", bytes(mp))
    write(D, "map/track.mus", mus)


def swvr_block(bid, chans_data, header, extra=None):
    """One SWVR block: id (LE), size, header fields, then each channel's data."""
    body = b"".join(chans_data)
    h = bytearray(header)
    h[0:4] = bid[::-1]
    h[4:8] = struct.pack("<I", header + len(body))
    for at, v in (extra or {}).items():
        h[at:at + len(v)] = v
    return bytes(h) + body


def swvr_stream(chans, per_block_frames, bid, header, extra=None, fill_every=0):
    datas = [psx_encode(c, lead=False, end=False) for c in chans]
    step = per_block_frames * 16
    out = bytearray()
    n = 0
    for i in range(0, len(datas[0]), step):
        out += swvr_block(bid, [d[i:i + step].ljust(step, b"\0") for d in datas], header, extra)
        n += 1
        if fill_every and n % fill_every == 0:
            out += b"LLIF" + struct.pack("<I", 0x20) + bytes(0x18)
    return bytes(out)


def build_swvr():
    a = tone(22050, 0.3, [500])
    b = sweep(22050, 0.3, 300, 2000)
    # PS1 style: RVWS + VAGM (0x1c header), ~14254 Hz, loop from the 2nd audio block
    body = swvr_stream([a, b], 40, b"VAGM", 0x1c, fill_every=2)
    write(D, "swvr/music.stream", b"RVWS" + struct.pack("<III", 0x20, 0, 1) + bytes(0x10) + body)
    # PS2 mono VAGB with the 0x6400 flag (22050 Hz, 0x40 header)
    body = swvr_stream([b], 50, b"VAGB", 0x40, {0x1a: struct.pack("<H", 0x6400)})
    write(D, "swvr/voice.stream", b"RVWS" + struct.pack("<III", 0x40, 0, 0) + bytes(0x30) + body)
    # Freekstyle raw movie audio: VAGM blocks of two subsongs alternating (0x24 flag, 0x40 header)
    s1 = [psx_encode(c, lead=False, end=False) for c in (a, b)]
    s2 = [psx_encode(c, lead=False, end=False) for c in (b[:4000], a[:4000])]
    step = 30 * 16
    out = bytearray()
    for i in range(0, max(len(s1[0]), len(s2[0])), step):
        for k, s in ((0, s1), (1, s2)):
            if i < len(s[0]):
                ex = {0x0c: struct.pack("<I", 1 - k if i == 0 and k == 0 else k), 0x1a: struct.pack("<H", 0x24)}
                if i == 0 and k == 0:
                    ex[0x0c] = struct.pack("<I", 1)  # first block tells the subsong count (index 1 -> 2 subsongs)
                out += swvr_block(b"VAGM", [d[i:i + step].ljust(step, b"\0") for d in s], 0x40, ex)
    write(D, "swvr/movie.str", bytes(out))


def musx_hdr(version, platform, file_size):
    h = bytearray(b"MUSX" + struct.pack("<III", 0x1234, version, file_size))
    if version in (4, 5, 6, 10):
        h += platform + bytes(0xC)
    return h


def build_musx():
    r = 32000
    a = tone(r, 0.3, [440, 880])
    b = sweep(r, 0.3, 200, 3000)
    c = tone(22050, 0.2, [700])
    # v4 PS2 MFX stereo (0x80 interleave), loop from the cue table
    data = interleave([psx_encode(a, lead=False, end=False), psx_encode(b, lead=False, end=False)], 0x80)
    info_at, data_at = 0x40, 0x100
    info = struct.pack("<IIIII", 0, 1, 0x14, 0x14, 80) + struct.pack("<IIIII", 0, len(data) - 0x100, 6, 0x400, 0)
    h = musx_hdr(4, b"PS2_", data_at + len(data))
    h += struct.pack("<IIIIIIII", info_at, len(info), data_at, len(data), 0, 0, 0, 0)
    h = h.ljust(info_at, b"\0") + info
    write(D, "musx/music_v4.sfx", bytes(h.ljust(data_at, b"\0")) + data)
    # v201 (Sphinx) PS2 sfx bank, mono sounds, platform guessed from the data
    sounds = [psx_encode(c, lead=False, end=False), psx_encode(a[:5000], lead=False, end=False)]
    head = struct.pack("<I", len(sounds))
    body = b""
    for i, sd in enumerate(sounds):
        head += struct.pack("<IIIIIIIIII", 0, len(body), len(sd), 22050 if i == 0 else 16000, 0, 1, 4, 0, 0, i)
        body += sd
    tables_at = 0x10
    head_at = 0x40
    data_at = (head_at + len(head) + 0x7f) // 0x80 * 0x80
    h = bytearray(b"MUSX" + struct.pack("<III", 0x77, 201, data_at + len(body)))
    h += struct.pack("<IIIIIIII", 0x30, 4, head_at, len(head), head_at, 0, data_at, len(body))
    h = h.ljust(0x30, b"\0") + struct.pack("<I", 0)
    h = h.ljust(head_at, b"\0") + head
    write(D, "musx/sfx_v201.sfx", bytes(h.ljust(data_at, b"\0")) + body)
    # v10 PS2 MFX: stream at 0x800, loop info table at 0x30, 0xAB padding at the end
    d = interleave([psx_encode(b, lead=False, end=False), psx_encode(a, lead=False, end=False)], 0x80)
    n = len(d) // 2 // 16 * 28
    h = musx_hdr(10, b"PS2_", 0)
    h = h.ljust(0x30, b"\0") + struct.pack("<IIIIIIII", 1, 1, 0, 0, n - 100, 2000, len(d), 0x800)
    total = 0x800 + len(d) + 0x40
    h[0x0c:0x10] = struct.pack("<I", total)
    write(D, "musx/music_v10.musx", bytes(h.ljust(0x800, b"\0")) + d + b"\xAB" * 0x40)
    # v5 PS2 MFX bank (mono 22050 Hz streams with offsets list), cue loops on the 2nd
    streams = [psx_encode(c, lead=False, end=False), psx_encode(b[:6000], lead=False, end=False), psx_encode(a[:3000], lead=False, end=False)]
    offs = b""
    ents = b""
    body = b""
    for i, sd in enumerate(streams):
        offs += struct.pack("<I", len(ents))
        loops = struct.pack("<IIII", 0, 1 if i == 1 else 0, 0x14, 0x14) + struct.pack("<I", 90)
        if i == 1:
            loops += struct.pack("<IIIII", 0, len(sd) - 0x200, 7, 0x100, 0)
        ents += struct.pack("<III", i, len(body), len(sd)) + loops
        body += sd
    t1_at = 0x40
    data_rel = ents
    tbl_at = t1_at + len(offs)
    data_at = 0x100
    # entries live in the data area (offsets relative to it): put them first, streams after
    ents2 = b""
    shift = (len(ents) + 0x7f) // 0x80 * 0x80
    body_all = bytearray(ents.ljust(shift, b"\0"))
    # rebuild entries with stream offsets past the entries
    ents2 = b""
    pos = shift
    for i, sd in enumerate(streams):
        loops = struct.pack("<IIII", 0, 1 if i == 1 else 0, 0x14, 0x14) + struct.pack("<I", 90)
        if i == 1:
            loops += struct.pack("<IIIII", 0, len(sd) - 0x200, 7, 0x100, 0)
        ents2 += struct.pack("<III", i, pos, len(sd)) + loops
        pos += len(sd)
    body_all = bytearray(ents2.ljust(shift, b"\0")) + b"".join(streams)
    h = musx_hdr(5, b"PS2_", data_at + len(body_all))
    h += struct.pack("<IIIIIIII", t1_at, len(offs), data_at, len(body_all), 0, 0, 0, 0)
    h = h.ljust(t1_at, b"\0") + offs
    write(D, "musx/bank_v5.sfx", bytes(h.ljust(data_at, b"\0")) + bytes(body_all))


# ---------------------------------------------------------------- Ubisoft
def ubi_adpcm(bps, channels, codes_per_subframe, codes_last, subframes, seed=5):
    """Ubi ADPCM stream: 0x30 header + frames with plausible states and random codes (the
    decode only has to match vgmstream's)."""
    rng = random.Random(seed)
    total = 0
    frames = bytearray()
    sub = 0
    while sub < subframes:
        if sub + 1 == subframes:
            ca, cb = codes_last, 0
        elif sub + 2 == subframes:
            ca, cb = codes_per_subframe, codes_last
        else:
            ca, cb = codes_per_subframe, codes_per_subframe
        for c in range(channels):
            st = struct.pack("<iiii", 2, rng.randint(300, 2000), rng.randint(-50, 50), rng.randint(-50, 50))
            st += struct.pack("<hhhh", rng.randint(-300, 300), rng.randint(-200, 200), 0, 0)
            st += struct.pack("<hhhh", *[rng.randint(-100, 100) for _ in range(4)])
            st += struct.pack("<hhhh", rng.randint(-2000, 2000), rng.randint(-2000, 2000), 0, 0)
            st += struct.pack("<hhhhhh", *[rng.randint(-500, 500) for _ in range(5)], 0)
            frames += st
        for n in (ca, cb):
            if n:
                size = bps * n // 8 + 1
                frames += bytes(rng.randrange(256) for _ in range(size))
        total += ca + cb
        sub += 2
    h = struct.pack("<IIIIIIIIIIII", 8, total, subframes, codes_last, codes_per_subframe, 2, 22050, 0, 0, bps, 1, channels)
    return h + bytes(frames)


def hx_riff(codec, channels, rate, data=None, datx=None):
    fmt = struct.pack("<HHIIHH", codec, channels, rate, rate * 2 * channels, 2 * channels, 16)
    body = b"WAVE" + b"fmt " + struct.pack("<I", len(fmt)) + fmt
    if data is not None:
        body += b"data" + struct.pack("<I", len(data)) + data
    if datx is not None:
        body += b"datx" + struct.pack("<I", 8) + struct.pack("<II", *datx)
    return b"RIFF" + struct.pack("<I", len(body)) + body


def hx_bank(objects):
    """HXx bank (LE): objects are (class, cuuid, body, links). Index at the end."""
    out = bytearray(struct.pack("<I", 0) + struct.pack(">I", 5) + b"test\0".ljust(8, b"\0"))
    index = []
    for cls, cuuid, body, links in objects:
        at = len(out)
        hdr = struct.pack("<I", len(cls)) + cls.encode() + struct.pack("<II", *cuuid) + body
        out += hdr
        index.append((cls, cuuid, at, len(hdr), links))
    idx_at = len(out)
    idx = bytearray(b"INDX" + struct.pack("<Ii", 2, len(index)))
    for cls, cuuid, at, size, links in index:
        idx += struct.pack("<I", len(cls)) + cls.encode() + struct.pack("<IIII", cuuid[0], cuuid[1], at, size)
        idx += struct.pack("<i", 0) + struct.pack("<i", len(links))
        for l in links:
            idx += struct.pack("<II", *l)
        idx += struct.pack("<i", 0)
    out += idx
    out[0:4] = struct.pack("<I", idx_at)
    return bytes(out)


def hx_wave(cls, riff, external=None, mode=None):
    """WaveFileIdObj body after class+cuuid: flag type 3, parent id, stream mode, [resource], RIFF."""
    if external:
        return struct.pack("<II", 3, 0) + bytes([3]) + struct.pack("<I", len(external)) + external.encode() + riff
    return struct.pack("<II", 3, 0) + bytes([mode or 0]) + riff


def hx_wavres(cls, name=b""):
    """WavResData body after class+cuuid: flags, internal name size + name."""
    return struct.pack("<I", 3) + struct.pack("<I", len(name)) + name


def build_hx():
    r = 22050
    a = tone(r, 0.25, [520])
    b = sweep(r, 0.25, 300, 2500)
    # PS2 (.hx2): internal mono and stereo PS-ADPCM (0x10 interleave), external stereo stream
    st = interleave([psx_encode(a, lead=False, end=False), psx_encode(b, lead=False, end=False)], 0x10)
    ext = interleave([psx_encode(b, lead=False, end=False), psx_encode(a, lead=False, end=False)], 0x10)
    stream_file = bytes(0x100) + ext
    objs = [
        ("CPS2WaveFileIdObj", (0x03000001, 0x11), hx_wave("", hx_riff(3, 1, r, psx_encode(a, lead=False, end=False))), []),
        ("CPS2WaveFileIdObj", (0x03000001, 0x12), hx_wave("", hx_riff(3, 2, 24000, st)), []),
        ("CPS2WaveFileIdObj", (0x03000001, 0x13), hx_wave("", hx_riff(3, 2, 32000, datx=(len(ext), 0x100)), external="MUSIC01.BIN"), []),
        ("CPS2WavResData", (0x01000001, 0x13), hx_wavres("CPS2WavResData", b"music01"), [(0x03000001, 0x13)]),
        ("CEventResData", (0x01000001, 0x20), bytes(8), [(0x01000001, 0x13)]),
    ]
    write(D, "hx/level.hx2", hx_bank(objs))
    write(D, "hx/MUSIC01.BIN", stream_file)
    # PC (.hxc): Ubi ADPCM 4-bit stereo and 6-bit mono, PCM16
    objs = [
        ("CPCWaveFileIdObj", (0x03000002, 1), hx_wave("", hx_riff(2, 2, r, ubi_adpcm(4, 2, 1024, 512, 5))), []),
        ("CPCWaveFileIdObj", (0x03000002, 2), hx_wave("", hx_riff(2, 1, r, ubi_adpcm(6, 1, 1536, 770, 3, seed=9))), []),
        ("CPCWaveFileIdObj", (0x03000002, 3), hx_wave("", hx_riff(1, 1, 16000, pcm16le(a[:3000]))), []),
    ]
    write(D, "hx/pc.hxc", hx_bank(objs))


def put(buf, at, fmt, *vals):
    data = struct.pack(fmt, *vals)
    if len(buf) < at + len(data):
        buf.extend(bytes(at + len(data) - len(buf)))
    buf[at:at + len(data)] = data


def psx_raw(samples, loop=None):
    return psx_encode(samples, lead=False, end=False, loop=loop)


def build_sb():
    r = 22050
    a = tone(r, 0.2, [500])
    b = sweep(r, 0.2, 300, 2500)
    c = tone(r, 0.25, [700, 350])
    # ---- .SB1 bank, version 0x000A0007 (Prince of Persia: Sands of Time / Rainbow Six 3 style)
    E1, E2 = 0x48, 0x6c
    sub0 = bytearray()  # hardware sounds (subblock 0)
    sub1 = bytearray()  # software sounds (subblock 1)
    sub3 = bytearray()  # hardware module sounds (subblock 3)
    da = psx_raw(a)
    off_a = len(sub0); sub0 += da
    dst = psx_raw(b) + psx_raw(c[:len(b)])
    off_st = len(sub3); sub3 += dst
    dpcm = pcm16le(a[:2000]) + pcm16le(b[:2000])
    dpcm_i = b"".join(struct.pack("<hh", a[i], b[i]) for i in range(2000))
    off_pcm = len(sub1); sub1 += dpcm_i
    ext = bytearray(0x200)
    ext_l = psx_raw(c, loop=(40, 150))
    ext_r = psx_raw(a + [0] * (len(c) - len(a)))
    ext_off = len(ext)
    ext += ext_l + ext_r
    entries = []
    # (id, type, fields dict of offset -> (fmt, value))
    ent = bytearray(E2); put(ent, 0, "<II", 0x00010001, 1); put(ent, 0x08, "<IIi", len(da), 0, off_a)
    put(ent, 0x18, "<I", 0); put(ent, 0x20, "<II", 1, r); put(ent, 0x30, "<I", len(da) // 16 * 28); put(ent, 0x68, "<I", 0x55)
    entries.append(ent)
    ent = bytearray(E2); put(ent, 0, "<II", 0x00010002, 1); put(ent, 0x08, "<IIi", len(dst), 0, off_st)
    put(ent, 0x18, "<I", (1 << 5) | (1 << 4)); put(ent, 0x20, "<II", 2, 24000)
    n = len(dst) // 2 // 16 * 28
    put(ent, 0x30, "<I", 500); put(ent, 0x38, "<I", n - 500)
    entries.append(ent)
    ent = bytearray(E2); put(ent, 0, "<II", 0x00010003, 1); put(ent, 0x08, "<IIi", len(ext_l) * 2, 0, ext_off)
    put(ent, 0x18, "<I", (1 << 2) | (1 << 4)); put(ent, 0x20, "<II", 2, 32000); put(ent, 0x30, "<II", 0, 0)
    ent[0x40:0x40 + 9] = b"MUSIC.SS1"
    entries.append(ent)
    ent = bytearray(E2); put(ent, 0, "<II", 0x00010004, 1); put(ent, 0x08, "<IIi", len(dpcm_i), 0, off_pcm)
    put(ent, 0x18, "<I", 1 << 3); put(ent, 0x20, "<II", 2, 16000); put(ent, 0x30, "<I", 2000); put(ent, 0x68, "<I", 1)
    entries.append(ent)
    # silence (type 8), 0.1 s
    ent = bytearray(E2); put(ent, 0, "<II", 0x00010005, 8); put(ent, 0x18, "<I", 6554)
    entries.append(ent)
    # sequence: sound 0, silence, sound 0; loops from the 2nd part
    sx = bytearray()
    seq_at = len(sx)
    for idx in (0, 4, 0):
        sx += struct.pack("<IIII", idx, 0, 0, 0)
    ent = bytearray(E2); put(ent, 0, "<II", 0x00020001, 5); put(ent, 0x0c, "<I", seq_at)
    put(ent, 0x18, "<iI", 1, 0); put(ent, 0x28, "<I", 3)
    entries.append(ent)
    # layered stream (type 6): layer 0 stereo + layer 1 mono, v4 blocked layout in LAYERS.SS1
    lay_n = 28 * 150
    l0 = psx_raw(b[:lay_n]) + psx_raw(c[:lay_n])
    l1 = psx_raw(a[:lay_n] + [0] * max(0, lay_n - len(a)))
    lay_hdr_at = len(sx)
    sx += struct.pack("<IHHII", 22050, 0, 2, 0, 0) + struct.pack("<I", lay_n)
    sx += struct.pack("<IHHII", 22050, 0, 1, 0, 0) + struct.pack("<I", lay_n)
    blk = 0x800
    lay = bytearray(struct.pack("<IIIIIII", 4, 2, 0, 0, 0x14, blk, 0) + struct.pack("<III", 8, 0, 0))
    p0 = p1 = 0
    bno = 1
    while p0 < len(l0) or p1 < len(l1):
        room = blk - 0x14
        c0 = min(len(l0) - p0, (room * 2 // 3) // 16 * 16)
        c1 = min(len(l1) - p1, room - c0)
        body = struct.pack("<IIIII", bno, len(lay), 3, c0, c1) + l0[p0:p0 + c0] + l1[p1:p1 + c1]
        lay += body.ljust(blk, b"\0")
        p0 += c0; p1 += c1; bno += 1
    lay_file = bytes(0x100) + bytes(lay)
    put(lay, 0x08, "<I", len(lay))
    lay_file = bytes(0x100) + bytes(lay)
    ent = bytearray(E2); put(ent, 0, "<II", 0x00030001, 6); put(ent, 0x0c, "<I", lay_hdr_at); put(ent, 0x18, "<I", 0)
    put(ent, 0x20, "<I", 2); put(ent, 0x58, "<I", 0x100); put(ent, 0x60, "<I", len(lay))
    ent[0x30:0x30 + 10] = b"LAYERS.SS1"
    entries.append(ent)
    write(D, "sb/LAYERS.SS1", lay_file)
    s1 = bytearray(E1); put(s1, 0, "<I", 0x00010001)
    s3 = struct.pack("<II", 0, len(sub0)) + struct.pack("<II", 1, len(sub1)) + struct.pack("<II", 3, len(sub3))
    hdr = bytearray(0x1c)
    put(hdr, 0, "<IIIIIII", 0x000A0007, 1, len(entries), 3, len(sx), 0, 0)
    sb = bytes(hdr) + bytes(s1) + b"".join(bytes(e) for e in entries) + bytes(sx) + s3 + bytes(sub0) + bytes(sub1) + bytes(sub3)
    write(D, "sb/POP_BANK.SB1", sb)
    write(D, "sb/MUSIC.SS1", bytes(ext))

    # ---- .SM1 map, version 0x00160002 (Splinter Cell: Double Agent style), two submaps
    E1, E2 = 0x48, 0x54
    out = bytearray(0x100)
    put(out, 0, "<III", 0x00160002, 0x10, 2)
    ext2 = bytearray(0x800)
    subs = []
    for m in range(2):
        body_sounds = bytearray()
        sa = psx_raw(a if m == 0 else b)
        sbd = psx_raw(c)
        # sounds: internal 0 (subblock 0), internal 1 (subblock 3), external streamed
        ents = []
        e = bytearray(E2); put(e, 0, "<II", 0x10000 * (m + 1) + 1, 1); put(e, 0x08, "<IIi", len(sa), 0, 0)
        put(e, 0x20, "<I", 0); put(e, 0x28, "<II", 1, 22050); put(e, 0x34, "<I", len(sa) // 16 * 28); put(e, 0x44, "<i", -1)
        ents.append(e)
        e = bytearray(E2); put(e, 0, "<II", 0x10000 * (m + 1) + 2, 1); put(e, 0x08, "<IIi", len(sbd), 0, 0)
        put(e, 0x20, "<I", 1 << 5); put(e, 0x28, "<II", 1, 11025); put(e, 0x34, "<I", len(sbd) // 16 * 28); put(e, 0x44, "<i", -1)
        ents.append(e)
        xl = psx_raw(b if m == 0 else a)
        xoff = len(ext2); ext2 += xl
        e = bytearray(E2); put(e, 0, "<II", 0x10000 * (m + 1) + 3, 1); put(e, 0x08, "<IIi", len(xl), 0, xoff)
        put(e, 0x20, "<I", 1 << 2); put(e, 0x28, "<II", 1, 22050); put(e, 0x34, "<I", len(xl) // 16 * 28); put(e, 0x44, "<I", 0)
        ents.append(e)
        sxb = b"STREAMS.SS1\0".ljust(0x28, b"\0")
        subs.append((ents, sxb, sa, sbd))
    pos = 0x100
    submap_blobs = []
    for m, (ents, sxb, sa, sbd) in enumerate(subs):
        base = pos
        s1 = bytearray(E1)
        s2 = b"".join(bytes(e) for e in ents)
        # section3: one entry with table1 (index -> offset in subblock) and table2 (subblocks)
        hdr_size = 0x2c
        s1_rel = hdr_size
        s2_rel = s1_rel + len(s1)
        sx_rel = s2_rel + len(s2)
        s3_rel = sx_rel + len(sxb)
        t1_rel = 0x14
        t1 = struct.pack("<II", 0, 0) + struct.pack("<II", 1, 0)
        t2_rel = t1_rel + len(t1)
        data_rel = s3_rel + t2_rel + 0x20
        data0_abs = base + data_rel
        data3_abs = data0_abs + len(sa)
        t2 = struct.pack("<IIII", 0, len(sa), len(sa), data0_abs) + struct.pack("<IIII", 3, len(sbd), len(sbd), data3_abs)
        s3 = struct.pack("<iIIII", -1, t1_rel, 2, t2_rel, 2) + t1 + t2
        h = struct.pack("<IIIIIIIIIII", 0, s1_rel, 1, s2_rel, len(ents), 0, 0, s3_rel, 1, sx_rel, len(sxb))
        blob = bytes(h) + bytes(s1) + s2 + sxb + s3
        blob = blob + sa + sbd
        submap_blobs.append((base, blob))
        me = 0x10 + m * 0x34
        put(out, me, "<IIII", 1 if m == 0 else 0, 0, base, len(blob))
        out[me + 0x10:me + 0x10 + 8] = (b"LEVEL%d" % m).ljust(8, b"\0")
        pos = base + len(blob)
        pos = (pos + 0xF) // 0x10 * 0x10
    full = bytearray(out)
    for base, blob in submap_blobs:
        if len(full) < base:
            full += bytes(base - len(full))
        full[base:base + len(blob)] = blob
    write(D, "sm/MAPS.SM1", bytes(full))
    write(D, "sm/STREAMS.SS1", bytes(ext2))

    # ---- old PS2 map, version 3 (Batman: Vengeance style): pitch rates, 0x800 interleave
    E1, E2 = 0x30, 0x3c
    st_l, st_r = psx_raw(c[:5000], loop=(20, 150)), psx_raw(b[:4000] + [0] * 1000)
    ilv = interleave([st_l, st_r], 0x800)
    per_ch = len(ilv) // 2
    ents = []
    pitch = int(22050 * 65536 / 48000)
    e = bytearray(E2); put(e, 0, "<II", 0x00010001, 1); put(e, 0x0c, "<I", per_ch); put(e, 0x14, "<I", 0)
    put(e, 0x1c, "<I", (1 << 7) | (1 << 5)); put(e, 0x20, "<II", pitch, 22050)
    ents.append(e)
    mono = psx_raw(a)
    e = bytearray(E2); put(e, 0, "<II", 0x00010002, 1); put(e, 0x0c, "<I", len(mono)); put(e, 0x14, "<I", 0)
    put(e, 0x1c, "<I", 0); put(e, 0x20, "<II", 0x4000, 12000)
    ents.append(e)
    s2 = b"".join(bytes(x) for x in ents)
    hdr_size = 0x24
    s1_rel, s2_rel = hdr_size, hdr_size + E1
    sx_rel = s2_rel + len(s2)
    s3_rel = sx_rel
    base = 0x40
    t1 = struct.pack("<II", 0, 0) + struct.pack("<II", 1, len(ilv))
    t2_rel = 0x14 + len(t1)
    data_abs = base + s3_rel + t2_rel + 0x10
    t2 = struct.pack("<IIII", 0, len(ilv) + len(mono), 0, data_abs)
    s3 = struct.pack("<iIIII", -1, 0x14, 2, t2_rel, 1) + t1 + t2
    blob = struct.pack("<IIIIIIIII", 0, s1_rel, 1, s2_rel, 2, s3_rel, 1, sx_rel, 0) + bytes(E1) + s2 + s3 + ilv + mono
    out = bytearray(0x40)
    put(out, 0, "<III", 3, 0x0c, 1)
    put(out, 0x0c, "<IIII", 1, 0, base, len(blob))
    out[0x1c:0x1c + 4] = b"BAT\0"
    write(D, "smold/BATMAN.SM1", bytes(out) + blob)

    # ---- Splinter Cell (PS2) map, version 7: internal PS-ADPCM, streamed PCM, v2 layered stream
    E1, E2 = 0x40, 0x70
    mono = psx_raw(b)
    ents = []
    e = bytearray(E2); put(e, 0, "<II", 0x00010001, 1); put(e, 0x08, "<I", 1); put(e, 0x0c, "<I", len(mono)); put(e, 0x14, "<I", 0)
    put(e, 0x1c, "<I", 0); put(e, 0x24, "<II", 1, 22050); put(e, 0x34, "<I", len(mono) // 16 * 28); put(e, 0x6c, "<I", 1)
    ents.append(e)
    pcm = b"".join(struct.pack("<hh", a[i], c[i]) for i in range(3000))
    ext3 = bytearray(0x80) + pcm
    e = bytearray(E2); put(e, 0, "<II", 0x00010002, 1); put(e, 0x0c, "<I", len(pcm)); put(e, 0x14, "<I", 0x80)
    put(e, 0x1c, "<I", 1 << 2); put(e, 0x24, "<II", 2, 22050); put(e, 0x34, "<I", 3000); put(e, 0x6c, "<I", 1)
    e[0x44:0x44 + 9] = b"SC_01.SS1"
    ents.append(e)
    # v2 layers: fixed blocks, no layer header data; 2 mono layers
    ln = 28 * 100
    # (v7 streamed "type 1" is PCM for vgmstream)
    la, lb = pcm16le(a[:ln] + [0] * max(0, ln - len(a))), pcm16le(c[:ln])
    blk = 0x400
    lay = bytearray(struct.pack("<IIIIII", 2, 2, 0, 0x10, blk, 0))
    q0 = q1 = 0
    bno = 1
    while q0 < len(la) or q1 < len(lb):
        room = blk - 0x10
        c0 = min(len(la) - q0, room // 2 // 16 * 16)
        c1 = min(len(lb) - q1, room - c0)
        lay += (struct.pack("<IIII", bno, len(lay), c0, c1) + la[q0:q0 + c0] + lb[q1:q1 + c1]).ljust(blk, b"\0")
        q0 += c0; q1 += c1; bno += 1
    lay_at = len(ext3)
    ext3 += lay
    sxv = struct.pack("<IHHII", 32000, 0, 1, 1, 0) + struct.pack("<II", 0, ln)
    sxv += struct.pack("<IHHII", 32000, 0, 1, 1, 0) + struct.pack("<II", 0, ln)
    e = bytearray(E2); put(e, 0, "<II", 0x00020001, 0x0d); put(e, 0x10, "<I", 0); put(e, 0x24, "<I", 2)
    put(e, 0x5c, "<I", lay_at); put(e, 0x64, "<I", len(lay)); e[0x34:0x34 + 9] = b"SC_01.SS1"
    ents.append(e)
    s2 = b"".join(bytes(x) for x in ents)
    s1_rel, s2_rel = 0x24, 0x24 + E1
    sx_rel = s2_rel + len(s2)
    s3_rel = sx_rel + len(sxv)
    base = 0x40
    t1 = struct.pack("<II", 0, 0)
    t2_rel = 0x14 + len(t1)
    data_abs = base + s3_rel + t2_rel + 0x10
    t2 = struct.pack("<IIII", 0, len(mono), len(mono), data_abs)
    s3 = struct.pack("<iIIII", -1, 0x14, 1, t2_rel, 1) + t1 + t2
    blob = struct.pack("<IIIIIIIII", 0, s1_rel, 1, s2_rel, len(ents), s3_rel, 1, sx_rel, len(sxv)) + bytes(E1) + s2 + sxv + s3 + mono
    out = bytearray(0x40)
    put(out, 0, "<III", 7, 0x0c, 1)
    put(out, 0x0c, "<IIII", 1, 0, base, len(blob))
    out[0x1c:0x1c + 4] = b"SC1\0"
    write(D, "sc/SC_MAP.SM1", bytes(out) + blob)
    write(D, "sc/SC_01.SS1", bytes(ext3))

    # ---- PS2 .BNM (Rayman 2: Revolution style) with external BNK_1.VB/VSB/VSC
    E1, E2 = 0x1c, 0x44
    vb = bytearray()
    va = psx_raw(a, loop=(10, 100))
    vb_off = len(vb); vb += va
    vsb_l, vsb_r = psx_raw(b), psx_raw(c[:len(b)])
    vsb = vsb_l + vsb_r
    vsc = interleave([psx_raw(c), psx_raw(a + [0] * (len(c) - len(a)))], 0x400)
    ents = []
    e = bytearray(E2); put(e, 0, "<II", 0x00000001, 1); put(e, 0x18, "<I", 1 << 7); put(e, 0x20, "<BxH", 1, 22050)
    put(e, 0x2c, "<II", len(va), vb_off); ents.append(e)
    e = bytearray(E2); put(e, 0, "<II", 0x00000002, 1); put(e, 0x18, "<I", 1 << 5); put(e, 0x20, "<BxH", 2, 16000)
    put(e, 0x2c, "<II", len(vsb_l), 0); ents.append(e)
    e = bytearray(E2); put(e, 0, "<II", 0x00000003, 1); put(e, 0x18, "<I", (1 << 5) | (1 << 6)); put(e, 0x20, "<BxH", 2, 24000)
    put(e, 0x2c, "<II", len(vsc), 1); ents.append(e)
    # sequence (type 0x0b): sound 1 then 1 again, bank 1
    sxs = struct.pack("<IIIII", (1 << 16) | 1, 0, 0, 0, 0) + struct.pack("<IIIII", (1 << 16) | 1, 0, 0, 0, 0)
    e = bytearray(E2); put(e, 0, "<II", 0x00000004, 0x0b); put(e, 0x10, "<I", 0); put(e, 0x18, "<iI", 0, 1); put(e, 0x24, "<I", 2)
    ents.append(e)
    s1_off = 0x24
    s2_off = s1_off + E1
    sx_off = s2_off + E2 * len(ents)
    hdr = struct.pack("<4sIIIIIIII", b"psx2", 1, s1_off, 1, s2_off, len(ents), sx_off, sx_off + len(sxs), 0)
    bnm = hdr + bytes(E1) + b"".join(bytes(x) for x in ents) + sxs
    write(D, "bnm/BNK_1.BNM", bnm)
    write(D, "bnm/BNK_1.VB", bytes(vb))
    write(D, "bnm/BNK_1.VSB", vsb)
    write(D, "bnm/BNK_1.VSC", bytes(0x800) + vsc)


if __name__ == "__main__":
    build_sb()
    build_ea()
    build_abk()
    build_hdr_dat()
    build_mpf()
    build_swvr()
    build_musx()
    build_hx()
    print("ok")
