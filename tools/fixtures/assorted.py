"""Test files for OMU, INT/WP2, PCM (KCEJE), LPCM, SPM, VOI, JSTM, GbTs, ILD, IVB,
SVAG (KCET), VS (MH), VSF, NPSF, XA2, MSA, VGS (PS), VS (Square), P2BT/MOVE/VISA, XABp (HD2+BD),
VBK, VIG (KCES), PCM (Success), SRE+PCM, MCG, PWB.

    python tools/fixtures/assorted.py [--blocked]

--blocked also writes files for variants that can't be decoded yet (XOR-encrypted JSTM,
encrypted VIG), which check.py reports as missed.
"""
import random
import struct
import sys

from common import *  # noqa: F401,F403

D = out_dir("assorted")
BLOCKED = "--blocked" in sys.argv


# ---------------------------------------------------------------- helpers
def fake_psx(frames, seed, lead=True, end=True, loop=None):
    """Valid PS-ADPCM noise (fast, for long data): `frames` data frames with small
    predictors/shifts. `loop` = (start_frame, end_frame) get flags 0x06 / 0x03."""
    rnd = random.Random(seed)
    out = bytearray(16) if lead else bytearray()
    for i in range(frames):
        flag = 0
        if loop and i == loop[0]:
            flag = 6
        elif loop and i == loop[1]:
            flag = 3
        elif end and not loop and i == frames - 1:
            flag = 1
        out += bytes([(rnd.randint(0, 1) << 4) | rnd.randint(9, 12), flag]) + bytes(rnd.getrandbits(8) for _ in range(14))
    if end:
        out += bytes([0, 7]) + bytes(14)
    return bytes(out)


def padto(b, n, fill=b"\0"):
    assert len(b) <= n, (len(b), n)
    return b + fill * (n - len(b))


def pad16(b):
    return pad(b, 16)


def interleave_short_last(chans, il):
    """Interleave with a shorter last block: full `il` blocks, then the rest of each channel
    (all channels the same length)."""
    n = len(chans[0])
    assert all(len(c) == n for c in chans)
    out = bytearray()
    full = n // il * il
    for i in range(0, full, il):
        for c in chans:
            out += c[i:i + il]
    for c in chans:
        out += c[full:]
    return bytes(out)


def pcm_interleave(chans, il):
    """16-bit LE PCM channels interleaved in `il` byte blocks (padded to whole blocks)."""
    return interleave([pcm16le(c) for c in chans], il)


def st(rate, secs, f):
    return tone(rate, secs, [f]), tone(rate, secs, [f * 1.5])


# ---------------------------------------------------------------- OMU
def omu(chans, rate):
    data = pcm_interleave(chans, 0x200)
    h = b"OMU " + u32le(0) + b"FRMT" + u32le(0x20) + u32le(rate) + bytes([len(chans), 16, 0, 0])
    h = h.ljust(0x3c, b"\0") + u32le(len(data))
    return h + data


write(D, "OMU/STEREO.OMU", omu(list(st(22050, 0.3, 440)), 22050))
write(D, "OMU/MONO.OMU", omu([tone(16000, 0.25, [300])], 16000))

# ---------------------------------------------------------------- raw INT / WP2
write(D, "INT/MUSIC.INT", pcm_interleave(list(st(48000, 0.2, 500)), 0x200))
write(D, "INT/QUAD.WP2", pcm_interleave([tone(48000, 0.15, [200 * (i + 1)]) for i in range(4)], 0x200))


# ---------------------------------------------------------------- PCM (KCE Japan East)
def pcm_kceje(l, r, loop=None):
    data = b"".join(struct.pack("<hh", a, b) for a, b in zip(l, r))
    ls, le = loop or (0, 0)
    h = struct.pack("<IIII", len(data), len(data) // 4, ls, le).ljust(0x800, b"\0")
    return h + data + bytes(0x100)


write(D, "KCEJE/LOOP.PCM", pcm_kceje(*st(24000, 0.3, 350), loop=(1000, 6000)))
write(D, "KCEJE/ONCE.PCM", pcm_kceje(*st(24000, 0.2, 600)))


# ---------------------------------------------------------------- LPCM (Shade)
def lpcm(l, r, loop=None):
    data = b"".join(struct.pack("<hh", a, b) for a, b in zip(l, r))
    ls, le = loop or (0, 0)
    return (b"LPCM" + struct.pack("<iiiI", len(l), ls, le, 0)).ljust(0x800, b"\0") + data


write(D, "LPCM/SONG.W", lpcm(*st(48000, 0.3, 440), loop=(2000, 12000)))
write(D, "LPCM/SE.LPCM", lpcm(*st(48000, 0.2, 700)))


# ---------------------------------------------------------------- SPM
def spm(l, r, loop):
    data = b"".join(struct.pack("<hh", a, b) for a, b in zip(l, r))
    return b"SPM\0" + struct.pack("<Iiii", 0x20 + len(data), loop[0], loop[1], 0x7f) + bytes(12) + data


write(D, "SPM/BGM.SPM", spm(*st(48000, 0.3, 330), (500, 14000)))


# ---------------------------------------------------------------- VOI
def voi(chans, mode):
    il = 0x200 if mode == 0 else 0x100
    data = pcm_interleave(chans, il) if len(chans) > 1 else pcm16le(chans[0])
    return struct.pack("<III", len(chans), len(data) // 2, mode).ljust(0x800, b"\0") + data


write(D, "VOI/V0001.VOI", voi(list(st(48000, 0.2, 400)), 0))
write(D, "VOI/V0002.VOI", voi([tone(24000, 0.3, [250])], 1))


# ---------------------------------------------------------------- JSTM (XOR, not decodable yet)
if BLOCKED:
    l, r = st(22050, 0.2, 440)
    data = bytes(b ^ 0x5A for b in b"".join(struct.pack("<hh", a, b) for a, b in zip(l, r)))
    write(D, "JSTM/KIND.STM", b"JSTM" + struct.pack("<HHIIIi", 2, 2, 22050, len(data), 0, -1) + bytes(8) + data)


# ---------------------------------------------------------------- GbTs
def gbts(chans, rate, loop=None):
    enc = [pad16(c) for c in chans]
    n = max(len(c) for c in enc)
    enc = [c.ljust(n, b"\0") for c in enc]
    data = interleave(enc, 0x10)
    ch = len(chans)
    ls, ll = (loop[0] * 16 * ch, (loop[1] - loop[0]) * 16 * ch) if loop else (0x20, 0)
    h = b"GbTs" + struct.pack("<IIIIIii", 0x24, 0x800, len(data), ls, ll, rate, ch) + u32le(1) + u32le(0x10 * ch)
    return h.ljust(0x800, b"\0") + pad(data, 0x800)


gl, gr = st(22050, 0.4, 523)
write(D, "GBTS/POP9.GBTS", gbts([psx_encode(gl, loop=(10, 250)), psx_encode(gr, loop=(10, 250))], 22050, loop=(11, 251)))
write(D, "GBTS/MONO.GBTS", gbts([psx_encode(tone(32000, 0.2, [660]))], 32000))


# ---------------------------------------------------------------- ILD
def ild(chans, rate, il, loop=None):
    n = max(len(c) for c in chans)
    chans = [c.ljust(n, b"\0") for c in chans]
    data = interleave(chans, il)
    ch = len(chans)
    ls, le = (loop[0] * 16, loop[1] * 16) if loop else (0, 0)
    h = b"ILD\0" + struct.pack("<IIII", ch, 0x800, n * ch, 0x10 * ch)
    for c in range(ch):
        h += struct.pack("<IIIIIIII", 0, 0x20, n, il, 1, rate, ls, le)
    return h.ljust(0x800, b"\0") + data


il_ = [fake_psx(300, 11), fake_psx(300, 12)]
write(D, "ILD/BATTLE.ILD", ild(il_, 44100, 0x400, loop=(40, 280)))
write(D, "ILD/QUAD.ILD", ild([fake_psx(150, 20 + i) for i in range(4)], 32000, 0x800))


# ---------------------------------------------------------------- IVB
def ivb(tracks, il):
    """tracks: list of (left, right) PS-ADPCM, all padded to the same number of blocks."""
    total = len(tracks)
    info = []
    blocks = 0
    for l, r in tracks:
        n = max(len(l), len(r))
        nb = (n + il - 1) // il
        last = n - (nb - 1) * il
        info.append((nb * il, nb, last))
        blocks = max(blocks, nb)
    h = b"IVB\0" + struct.pack("<iiI", total, il, 0)
    for size, nb, last in info:
        h += struct.pack("<IIII", size, nb, last, 0)
    h = h.ljust(0x800, b"\0")
    chunks = [interleave([l.ljust(blocks * il, b"\0"), r.ljust(blocks * il, b"\0")], il) for l, r in tracks]
    data = bytearray()
    for k in range(blocks):
        for c in chunks:
            data += c[k * 2 * il:(k + 1) * 2 * il]
    return h + bytes(data)


write(D, "IVB/BGM01.IVB", ivb([(fake_psx(170, 30), fake_psx(170, 31)), (fake_psx(120, 32), fake_psx(120, 33))], 0x400))


# ---------------------------------------------------------------- SVAG (KCET)
def svag(chans, rate, il, loop_start=None):
    n = max(len(c) for c in chans)
    chans = [c.ljust(n, b"\0") for c in chans]
    data = interleave_short_last(chans, il) if len(chans) > 1 else chans[0]
    h = b"Svag" + struct.pack("<IIHHIII", len(data), rate, len(chans), 0, il, 1 if loop_start is not None else 0, (loop_start or 0) * 16)
    h = h.ljust(0x400, b"\0")
    h = (h + (h[:0x20] if len(chans) > 1 else b"")).ljust(0x800, b"\0")
    return h + data


write(D, "SVAG/SH2.SVAG", svag([fake_psx(333, 40), fake_psx(333, 41)], 44100, 0x800, loop_start=100))
write(D, "SVAG/MONO.SVAG", svag([psx_encode(tone(22050, 0.2, [500]))], 22050, 0))


# ---------------------------------------------------------------- VS (Melbourne House)
def vs_mh(l, r, rate):
    n = max(len(l), len(r))
    l, r = l.ljust(n, b"\0"), r.ljust(n, b"\0")
    out = bytearray(b"\xC8\0\0\0" + u32le(rate))
    for i in range(0, n, 0x1000):
        a, b = l[i:i + 0x1000], r[i:i + 0x1000]
        out += u32le(len(a)) + a + u32le(len(b)) + b
    return bytes(out)


write(D, "VSMH/MIB.VS", vs_mh(fake_psx(700, 50), fake_psx(700, 51), 48000))


# ---------------------------------------------------------------- VSF
def vsf(chans, pitch, loop_start=None, short_header=False, extra_flags=0x10):
    n = max(len(c) for c in chans)
    n = (n + 0x3ff) // 0x400 * 0x400 if len(chans) > 1 else pad16(chans[0]).__len__()
    chans = [c.ljust(n, b"\0") for c in chans]
    data = interleave(chans, 0x400) if len(chans) > 1 else chans[0]
    flags = (1 if len(chans) > 1 else 0) | (2 if loop_start is not None else 0) | extra_flags | (0x100 if short_header else 0)
    h = b"VSF\0" + struct.pack("<IIIIIIIiII", len(data), 7, 0x10000, n // 16, 0x10, loop_start or 0, flags, pitch, 0x7f, 0)
    start = 0x80 if short_header else 0x800
    return h.ljust(start, b"\xff") + data


write(D, "VSF/MUSASHI.VSF", vsf([fake_psx(200, 60), fake_psx(200, 61)], 0x1000, loop_start=30))
write(D, "VSF/VOICE.VSF", vsf([psx_encode(tone(24000, 0.3, [300]))], 0x0800, short_header=True))


# ---------------------------------------------------------------- NPSF
def npsf(chans, rate, name, loop_start=-1):
    n = max(len(c) for c in chans)
    chans = [c.ljust(n, b"\0") for c in chans]
    data = interleave(chans, 0x800) if len(chans) > 1 else chans[0]
    h = b"NPSF" + struct.pack("<IiiiiiIIIIII", 0x1000, n, len(chans), 0x800, loop_start, rate, 0x3e8, 0, 0, 0, 0, 0x40)
    h += name.encode().ljust(0x20, b"\0")
    return h.ljust(0x800, b"\xff") + data


write(D, "NPSF/TEKKEN.NPS", npsf([fake_psx(250, 70), fake_psx(250, 71)], 48000, "bgm_stage01", loop_start=2800))
write(D, "NPSF/VOICE.NPSF", npsf([psx_encode(tone(22050, 0.25, [440]))], 22050, "vo_win"))


# ---------------------------------------------------------------- XA2 (Acclaim)
def xa2(chans, il, rcrp=False):
    n = max(len(c) for c in chans)
    chans = [c.ljust(n, b"\0") for c in chans]
    data = interleave(chans, il)
    h = u32le(len(chans)) + (b"" if rcrp else u32le(il)) + b"".join(u32be(len(data) // len(chans)) for _ in chans)
    return h.ljust(0x800, b"\0") + data


write(D, "XA2/RACE.XA2", xa2([fake_psx(200, 80), fake_psx(200, 81)], 0x800))
write(D, "XA2/RCRP.XA2", xa2([fake_psx(300, 82), fake_psx(300, 83)], 0x1000, rcrp=True))


# ---------------------------------------------------------------- MSA
def msa(chans, rate, data_size, konohana=False):
    il = 0x6000 if konohana else 0x4000
    n = max(len(c) for c in chans)
    chans = [c.ljust(n, b"\0") for c in chans]
    data = interleave(chans, il)
    return struct.pack("<IIIII", 0, data_size, 0, 0x1234 if konohana else 0, rate) + data


m = [fake_psx(1100, 90), fake_psx(1100, 91)]
write(D, "MSA/STAGE1.MSA", msa(m, 44100, 0x9000))  # size inside the file
write(D, "MSA/AME.MSA", msa(m, 0, 0x7fffffff))  # size past the file: whole blocks only
write(D, "MSA/KONO.MSA", msa([fake_psx(700, 92), fake_psx(700, 93)], 22050, 0x8000, konohana=True))


# ---------------------------------------------------------------- VGS (Princess Soft)
def vgs_ps(l, r, rate, il, name):
    n = max(len(l), len(r))
    l, r = l.ljust(n, b"\0"), r.ljust(n, b"\0")
    data = interleave_short_last([l, r], il)
    h = b"VGS\0" + struct.pack(">IIII", 4, 0, n, rate) + bytes(12) + name.encode().ljust(16, b"\0")
    return h + data


write(D, "VGS/GIN.VGS", vgs_ps(fake_psx(2300, 100), fake_psx(2300, 101), 44100, 0x8000, "GIN_BGM"))
write(D, "VGS/METAL.VGS", vgs_ps(fake_psx(8300, 102), fake_psx(8300, 103), 48000, 0x20000, "METAL"))


# ---------------------------------------------------------------- VS (Square)
def vs_square(chans, pitch, flags=None):
    ch = len(chans)
    per = 0x800 - 0x20
    n = max(len(c) for c in chans)
    blocks = (n + per - 1) // per
    chans = [c.ljust(blocks * per, b"\0") for c in chans]
    flags = (1 if ch > 1 else 0) if flags is None else flags
    out = bytearray()
    for k in range(blocks):
        for c in chans:
            out += b"VS\0\0" + struct.pack("<IIIIIII", flags, k, blocks - 1 - k, pitch, 0x64, 0, 0) + c[k * per:(k + 1) * per]
    return bytes(out)


write(D, "VSSQ/FFX_V01.VS", vs_square([psx_encode(tone(24000, 0.25, [320]))], 0x800))
write(D, "VSSQ/PROWRES.VS", vs_square([fake_psx(300, 110), fake_psx(300, 111)], 0x1000))


# ---------------------------------------------------------------- P2BT / MOVE / VISA
def p2bt(magic, chans, rate, il, name, loop_start=0):
    n = max(len(c) for c in chans)
    chans = [c.ljust(n, b"\0") for c in chans]
    data = interleave_short_last(chans, il) if len(chans) > 1 else chans[0]
    h = magic + struct.pack("<IiiIIIIiI", 0x7fc, rate, loop_start * 16 * len(chans), len(data), il, 1, 0x10, len(chans), 1) + name.encode().ljust(16, b"\0")
    return h.ljust(0x800, b"\0") + data


write(D, "P2BT/AFDS.VIS", p2bt(b"VISA", [fake_psx(150, 120), fake_psx(150, 121)], 44100, 0x400, "AFDS.VIS", loop_start=20))
write(D, "P2BT/POPN.P2BT", p2bt(b"P2BT", [fake_psx(90, 122), fake_psx(90, 123)], 44100, 0x10, "POPN.P2BT"))
write(D, "P2BT/MENU.MOVE", p2bt(b"MOVE", [psx_encode(tone(22050, 0.2, [700]))], 22050, 0x10, "MENU.MOVE"))


# ---------------------------------------------------------------- XABp (.HD2 + .BD)
def xabp(sounds, entries):
    """sounds: PS-ADPCM blobs stored in the .BD; entries: (sound index, pitch)."""
    bd = bytearray()
    offs = []
    for s in sounds:
        offs.append(len(bd))
        bd += s
    bd += bytes(0x40)
    hd = b"pBAX" + struct.pack("<IIhH", len(bd), 0, len(entries), 0x10)
    for i, (si, pitch) in enumerate(entries):
        # Rate as an SPU2 pitch at 0x0e (current vgmstream) and as Hz at 0x16 (vgmstream r2117,
        # which check.py runs, as a signed 16-bit value): both read the same rate.
        hd += struct.pack("<IIIHH", i, 0x7f, 0x40, 0, pitch) + struct.pack("<IHHII", 0, 0, 48000 * pitch // 4096, offs[si], 0)
    return hd, bytes(bd)


hd2, bd = xabp([psx_encode(tone(24000, 0.2, [440])), psx_encode(sweep(48000, 0.3), loop=(5, 400)), fake_psx(60, 130)],
               [(0, 0x800), (1, 0xa00), (2, 0x555), (0, 0x800)])
write(D, "XABP/SE.HD2", hd2)
write(D, "XABP/SE.BD", bd)


# ---------------------------------------------------------------- VBK
def vbk(streams):
    """streams: (channel blobs, rate, interleave)."""
    table = bytearray()
    body = bytearray()
    for i, (chans, rate, il) in enumerate(streams):
        n = max(len(c) for c in chans)
        chans = [c.ljust(n, b"\0") for c in chans]
        data = interleave(chans, il) if len(chans) > 1 else chans[0]
        table += struct.pack("<IIIiiI", n * len(chans), 100 + i, len(body), rate, il, len(chans) - 1)
        body += pad(data, 0x800)
    start = (0x14 + len(table) + 0x7ff) // 0x800 * 0x800
    h = (b".VBK" + struct.pack("<IiII", 2, len(streams), start, start + len(body)) + table).ljust(start, b"\0")
    return h + bytes(body)


write(D, "VBK/STITCH.VBK", vbk([
    ([fake_psx(1500, 140, loop=(100, 1450)), fake_psx(1500, 141, loop=(100, 1450))], 4000, 0x400),  # long: loops
    ([psx_encode(tone(22050, 0.2, [900]))], 22050, 0),
    ([fake_psx(200, 142, loop=(10, 150)), fake_psx(200, 143, loop=(10, 150))], 22050, 0x800),  # short: no loop
]))


# ---------------------------------------------------------------- VIG (KCES)
def vig(chans, rate, il, loop=None, encrypt=False):
    n = max(len(c) for c in chans)
    ch = len(chans)
    if ch > 1:
        n = (n + il - 1) // il * il
    chans = [c.ljust(n, b"\0") for c in chans]
    data = interleave(chans, il) if ch > 1 else chans[0]
    ls, ll = (loop[0] * 16 * ch, (loop[1] - loop[0]) * 16 * ch) if loop else (0, 0)
    if encrypt:
        d = bytearray(data)
        for i in range(0, len(d), 16):
            d[i] ^= 0xFF
            d[i + 2] = (d[i + 2] - 2) & 0xFF
        data = bytes(d)
    h = b"\x01\x00\x64\x08" + struct.pack("<IIIIIiiII", 0, 0x800, len(data), ls, ll, rate, ch, 1 if encrypt else 0, il if ch > 1 else 0)
    return h.ljust(0x800, b"\0") + data


write(D, "VIG/POPN11.VIG", vig([fake_psx(260, 150, loop=(20, 250)), fake_psx(260, 151, loop=(20, 250))], 44100, 0x800, loop=(21, 251)))
write(D, "VIG/SE.VIG", vig([psx_encode(tone(32000, 0.2, [330]))], 32000, 0))
if BLOCKED:
    write(D, "VIG/IIDX.VIG", vig([fake_psx(100, 152), fake_psx(100, 153)], 44100, 0x800, encrypt=True))


# ---------------------------------------------------------------- PCM (Success)
def pcm_success(chans, rate, loop=None):
    il = 0x800
    ch = len(chans)
    n = max(len(c) for c in chans)
    blocks = (n + il - 1) // il
    chans = [c.ljust(blocks * il, b"\0") for c in chans]
    data = interleave(chans, il)
    # loop: (start block, start adjust, end block)
    lb, la, le = loop or (0, 0, 0)
    h = b"PCM " + struct.pack("<IIiiiiiiiiI", 0x10000, len(data), rate, ch, 1 if loop else 0, blocks, la, lb, 0x800 if loop else 0, le, 1)
    return h.ljust(0x800, b"\0") + data


write(D, "SUCCESS/METAL.PCM", pcm_success([fake_psx(400, 160), fake_psx(400, 161)], 44100, loop=(1, 0x100, 2)))
write(D, "SUCCESS/MONO.PCM", pcm_success([psx_encode(tone(22050, 0.3, [260]))], 22050))


# ---------------------------------------------------------------- SRE + PCM (Capcom)
def sre_pcm(streams):
    """streams: (channel blobs, rate, loop (start, end) in per-channel bytes or None)."""
    pcm = bytearray()
    t2 = bytearray()
    for chans, rate, loop in streams:
        n = max(len(c) for c in chans)
        ch = len(chans)
        if ch > 1:
            n = (n + 0xfff) // 0x1000 * 0x1000
        chans = [c.ljust(n, b"\0") for c in chans]
        data = interleave(chans, 0x1000) if ch > 1 else chans[0]
        ls, le = loop or (0, 0)
        t2 += struct.pack("<iHHIIIIiI", ch, rate, 0, len(pcm), len(data), ls, le, 1 if loop else 0, 0)
        pcm += pad(data, 0x800)
    t1 = bytes(0x60)
    sre = struct.pack("<iIiI", 1, 0x10, len(streams), 0x10 + len(t1)) + t1 + bytes(t2)
    return sre, bytes(pcm)


sre, spcm = sre_pcm([([psx_encode(tone(22050, 0.2, [500]))], 22050, None),
                     ([fake_psx(300, 170, loop=(10, 290)), fake_psx(300, 171, loop=(10, 290))], 32000, (11 * 16, 291 * 16))])
write(D, "SRE/VJ.SRE", sre)
write(D, "SRE/VJ.PCM", spcm)


# ---------------------------------------------------------------- MCG
def mcg(chans, rate, il, name):
    ch = len(chans)
    n = max(len(c) for c in chans)
    chans = [c.ljust(n, b"\0") for c in chans]
    data = interleave(chans, il)
    track_size = len(data) * 2 // ch

    def vagp(suffix):
        return b"VAGp" + struct.pack(">IIII", 4, 0, n, rate) + bytes(12) + (name + suffix).encode().ljust(16, b"\0")
    h = b"MCG\0" + struct.pack("<IIIIiII", 0x20, 0x50, 0x80, track_size, il, 0, 0) + vagp("L") + vagp("R")
    return h + data


write(D, "MCG/TC.GCM", mcg([psx_encode(sweep(44100, 0.3, 300, 3000)), psx_encode(sweep(44100, 0.3, 3000, 300))], 44100, 0x800, "TC_BGM"))
write(D, "MCG/TCM.GCM", mcg([fake_psx(200, 180 + i) for i in range(6)], 48000, 0x800, "TC_MULTI"))


# ---------------------------------------------------------------- PWB
def pwb(sounds):
    """sounds: (PS-ADPCM blob, loop (start, end) in bytes or None)."""
    entries = bytearray()
    body = bytearray()
    for i, (s, loop) in enumerate(sounds):
        ls, le = loop or (0, 0)
        entries += struct.pack("<IIIIII", i, 0x000AC449, len(body), len(s), ls, le - ls)
        body += pad(s, 0x40)
    eoff = 0x40
    doff = pad(bytes(eoff + len(entries)), 0x800).__len__()
    h = b"WB\x02\x00" + struct.pack("<IIIIIIIIIII", 0, 0x20, 0x20, eoff, len(entries), doff, len(body), 1, len(sounds), 0x18, doff)
    return (h.ljust(eoff, b"\0") + bytes(entries)).ljust(doff, b"\0") + bytes(body)


write(D, "PWB/PSYCHO.PWB", pwb([(psx_encode(tone(24000, 0.2, [440])), None),
                                (psx_encode(tone(24000, 0.3, [220, 330]), loop=(3, 200)), (4 * 16, 201 * 16)),
                                (fake_psx(80, 190), None)]))

print("assorted fixtures written to", D)
