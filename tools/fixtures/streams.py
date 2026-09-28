"""Test files for streamed and banked formats (vgmstream meta parsers in src/formats/).

    python tools/fixtures/streams.py [name ...]   (no names: build everything)
"""
import struct
import sys

from common import *  # noqa: F401,F403

D = out_dir("streams")
BUILDERS = {}


def builder(fn):
    BUILDERS[fn.__name__] = fn
    return fn


def enc(n, rate=22050, f=(440,), loop=None, lead=True, end=True):
    """PS-ADPCM of a short tone of `n` samples."""
    return psx_encode(tone(rate, n / rate, list(f)), lead=lead, end=end, loop=loop)


def u32s(*v):
    return b"".join(u32le(x) for x in v)


# ---------------------------------------------------------------- PS2P
def vag_thq(data, rate, name):
    size = 0x30 + len(data)
    return b"VAGp" + struct.pack(">IIII", 0x20, 0, size + 0x10, rate) + bytes(12) + name.encode().ljust(16, b"\0") + data


@builder
def ps2p():
    files = [vag_thq(enc(3000, f=(300,)), 22050, "MONO"),
             vag_thq(enc(2800, f=(500,)), 22050, "LEFT"),
             vag_thq(enc(2800, f=(700,)), 22050, "RIGHT"),
             vag_thq(enc(4200, f=(900,), loop=(10, 120)), 32000, "LOOP")]
    aux_ids = [0, 1, 3]
    names = [b"mono\0", b"stereo\0", b"looped\0"]
    align = 0x800
    fc, ac = len(files), len(aux_ids)
    offsets, pos = [], align
    for f in files:
        offsets.append(pos)
        pos = (pos + len(f) + align - 1) // align * align
    table = bytearray()
    for i, f in enumerate(files):
        nxt = offsets[i + 1] if i + 1 < fc else 0
        table += u32s(len(f), 0, nxt)
    aux = bytearray()
    strings = bytearray(b"\0\0\0\0")  # overlaps the last entry's end pointer
    ends = []
    for n in names:
        strings += n
        ends.append(len(strings))
    for i, a in enumerate(aux_ids):
        aux += u32le(a) + bytes(0x14) + u32le(ends[i])
    hdr = b"ps2p" + u32s(1, 0, align, 0, fc, ac, 0) + table + aux[:-4] + strings
    out = bytearray(hdr.ljust(align, b"\0"))
    for o, f in zip(offsets, files):
        out = out.ljust(o, b"\0") + f
    write(D, "PS2P/BANK.SOUNDS", bytes(out))


# ---------------------------------------------------------------- VSV
def vsv_file(chans, rate, blocks, word0, adjust, loop_word):
    il = 0x800
    streams = [enc(blocks * 128 * 28 - 56, rate, f=(300 + 200 * i,)) for i in range(len(chans))]
    body = bytearray(interleave(streams, il))
    body = body[:blocks * len(chans) * il].ljust(blocks * len(chans) * il, bytes(1))
    hdr = struct.pack("<HBBHHHBBHH", word0, 0, 0x50, adjust, loop_word, rate, 0, 1 if len(chans) == 2 else 0x10, blocks * len(chans), 3)
    body[0:16] = hdr
    return bytes(body)


@builder
def vsv():
    write(D, "VSV/STEREO.VSV", vsv_file([0, 1], 32000, 3, 0x0101, 0x0400, 0x8001))
    write(D, "VSV/SAGA.VSV", vsv_file([0], 22050, 4, 0x0000, 0x0000, 0x8003))


# ---------------------------------------------------------------- MultiStream (MJH/MSH/MIH/MIC)
def ms_stream(chans, rate, frame_size, frames, freqs=(440,)):
    """Interleaved PS-ADPCM filling `frames` blocks of `frame_size` per channel."""
    per = frame_size * frames
    streams = [enc(per // 16 * 28 - 56, rate, f=(freqs[0] + 150 * c,)) for c in range(chans)]
    return interleave([s[:per].ljust(per, bytes(1)) for s in streams], frame_size)


@builder
def mjh():
    subs = [(2, 22050, 0x400, 3), (1, 16000, 0x800, 2), (2, 32000, 0x100, 9)]
    hdr = u32le(len(subs)).ljust(0x40, bytes(1))
    body = bytearray()
    for ch, rate, fs, fc in subs:
        hdr += (u32s(0x40, 0, ch, rate, fs, fc)).ljust(0x40, bytes(1))
        body += ms_stream(ch, rate, fs, fc)
    write(D, "MJH/BANK.MJH", hdr)
    write(D, "MJH/BANK.MJB", bytes(body))


@builder
def msh():
    sounds = [(enc(2000, 22050, f=(400,)), 0x64, 22050), (b"", 0, 0), (enc(3000, 11025, f=(250,), loop=(5, 80)), 0x64 | (1 << 24) | 1, 11025),
              (enc(1500, 44100, f=(1200,)), 0x50, 44100)]
    body, entries = bytearray(), bytearray()
    for data, cfg, rate in sounds:
        if data:
            entries += u32s(len(data), cfg, len(body), rate)
            body += data
            body = bytearray(pad(bytes(body), 0x800))
        else:
            entries += u32s(0, 0, 0, 0)
    hdr = u32s(0, 0x34, len(sounds)) + entries
    hdr = u32le(len(hdr)) + hdr[4:]
    write(D, "MSH/SFX.MSH", hdr)
    write(D, "MSH/SFX.MSB", bytes(body))


def mih_header(ch, rate, fs, fc, last=0):
    return u32s(0x40, (last << 8) | 0x20, ch, rate, fs, fc).ljust(0x40, bytes(1))


@builder
def mib():
    write(D, "MIB/MUSIC.MIH", mih_header(2, 44100, 0x800, 3, last=0x230))
    write(D, "MIB/MUSIC.MIB", ms_stream(2, 44100, 0x800, 3))
    name = b"gettingup_track\0"
    write(D, "MIB/ECKO.MIH", u32le(len(name)) + name + u32le(0x40) + mih_header(1, 22050, 0x400, 5))
    write(D, "MIB/ECKO.MIB", ms_stream(1, 22050, 0x400, 5, (330,)))
    write(D, "MIC/ROGUE.MIC", mih_header(2, 32000, 0x200, 6) + ms_stream(2, 32000, 0x200, 6, (500,)))
    write(D, "MIC/GLADIUS.MIC", mih_header(1, 24000, 0x100, 0) + ms_stream(1, 24000, 0x100, 11, (700,)))


# ---------------------------------------------------------------- XAVS
def chunk(cid, data):
    return u32le(cid | (len(data) << 8)) + data


def xavs_file(tracks, rate, il, video=True):
    """`tracks`: {chunk id: [left, right] samples}; audio is chunked in random sizes."""
    import random as rnd
    r = rnd.Random(7)
    streams = {}
    for cid, (left, right) in tracks.items():
        n = len(left) // (il // 2) * (il // 2)
        streams[cid] = interleave([pcm16le(left[:n]), pcm16le(right[:n])], il)
    out = bytearray(b"XAVS" + u32s(0x01000080 if video else 0, 0, len(tracks) | (0x50 << 16), 0x1000, 0x800))
    if video:
        out += chunk(0x56, bytes(r.randrange(256) for _ in range(0x90)))
        out += u32le(0x21)
    pos = {cid: 0 for cid in streams}
    while any(pos[c] < len(streams[c]) for c in streams):
        for cid, data in streams.items():
            if pos[cid] < len(data):
                n = r.choice([0x180, 0x400, 0x260, 0x800])
                out += chunk(cid, data[pos[cid]:pos[cid] + n])
                pos[cid] += n
        if video and r.random() < 0.5:
            out += chunk(0x56, bytes(r.randrange(256) for _ in range(r.randrange(0x10, 0x200))))
            out += u32le(0x21)
    out += u32le(0x5F)
    return bytes(out)


@builder
def xavs():
    write(D, "XAVS/MOVIE.XAV", xavs_file({0x41: (tone(48000, 0.3, [440]), tone(48000, 0.3, [660]))}, 48000, 0x200))
    write(D, "XAVS/TRACKS.XAV", xavs_file({0x61: (sweep(24000, 0.4), tone(24000, 0.4, [300])),
                                           0x62: (tone(24000, 0.3, [880]), sweep(24000, 0.3, 3000, 100))}, 24000, 0x100, video=False))


# ---------------------------------------------------------------- VGS
def frames_of(data, flag):
    fr = [bytearray(data[i:i + 16]) for i in range(0, len(data), 16)]
    for f in fr:
        f[1] = flag
    return fr


@builder
def vgs():
    a = frames_of(enc(6000, 44100, f=(440,), lead=False, end=False), 0)
    b = frames_of(enc(6000, 44100, f=(550,), lead=False, end=False), 1)[:-1]  # one frame short
    c = frames_of(enc(3000, 22050, f=(220,), lead=False, end=False), 2)
    hdr = b"VgS!" + u32le(2) + u32s(44100, len(a), 44100, len(b), 22050, len(c))
    hdr = hdr.ljust(0x80, bytes(1))
    body = bytearray()
    for i in range(len(a)):
        body += a[i] + (b[i] if i < len(b) else bytes(16))
        if i % 2 == 0 and i // 2 < len(c):
            body += c[i // 2]
    write(D, "VGS/SONG.VGS", hdr + bytes(body))
    # old: channels, rate, frames; interleave 0x2000 with a short last row
    l, r = enc(5000, 32000, f=(300,)), enc(5000, 32000, f=(450,))
    data = interleave([l, r], 0x2000)
    data = data[:0x4000 + 0x2000 + 0x800]
    write(D, "VGS_OLD/KARAOKE.VGS", u32s(2, 32000, len(data) // 32, 0) + data)


# ---------------------------------------------------------------- EXST
def exst_header(ch, rate, loop, ls, le, size=0x78):
    return (b"EXST" + struct.pack("<HHIIII", 0, ch, rate, loop, ls, le)).ljust(size, bytes(1))


@builder
def exst():
    il = 0x400
    data = interleave([enc(9000, 32000, f=(300,)), enc(9000, 32000, f=(400,))], il)[:0x2000 + 0x400 + 0x1a0]
    write(D, "EXST/SEP.STS", exst_header(2, 32000, 1, 1, 3))
    write(D, "EXST/SEP.INT", data)
    mono = enc(5000, 22050, f=(600,))
    write(D, "EXST/JOIN.X", exst_header(1, 22050, 0, 0, 0) + mono)
    data2 = interleave([enc(4000, 24000, f=(700,)), enc(4000, 24000, f=(800,))], il)[:0x1800]
    write(D, "EXST/GACHA.STS", exst_header(2, 24000, 1, 0, 3, size=0x80) + data2)
    d3 = interleave([enc(3000, 48000, f=(500,)), enc(3000, 48000, f=(900,))], 0x10)
    write(D, "EXST/COLOSSUS.STS_CP3", exst_header(2, 48000, 1, 0x200, len(d3) - 0x100))
    write(D, "EXST/COLOSSUS.INT_CP3", d3)


# ---------------------------------------------------------------- IMC
def imc_stream(chans, rate, frames_per_block, blocks):
    il = frames_per_block * 16
    per = il * blocks // len(chans)
    streams = [enc(int(per / 16 * 28 * (0.8 - 0.1 * c)), rate, f=(350 + 250 * c,)) for c in range(len(chans))]
    data = interleave([s[:per].ljust(per, bytes(1)) for s in streams], il)
    return u32s(len(chans), rate, frames_per_block, blocks) + data


@builder
def imc():
    write(D, "IMC/SINGLE.IMC", imc_stream([0, 1], 22050, 0x40, 6))
    subs = [(b"ST10_A", imc_stream([0, 1], 32000, 0x80, 10)), (b"ST10_B", imc_stream([0], 16000, 0x20, 7)),
            (b"ST10_C", imc_stream([0, 1], 44100, 0x100, 12))]
    table = bytearray(u32le(len(subs)))
    pos = 4 + 0x20 * len(subs)
    body = bytearray()
    for name, d in subs:
        table += name.ljust(8, bytes(1)) + u32s(0x002ADE77, 1) + u32le(pos + len(body)) + u32s(0xF0950000, 2, 0)
        body += d
    write(D, "IMC_CONT/STAGE.IMC", bytes(table) + bytes(body))


# ---------------------------------------------------------------- LP/AP/LEP
@builder
def lp_ap_lep():
    il = 0x800
    data = interleave([enc(7000, 44100, f=(300,)), enc(7000, 44100, f=(500,))], il)
    start = 0x800
    end = start + len(data)
    hdr = b"AP  " + u32s(end - 0x20, 44100, il, 0x7F7F, start + il * 2, end - 0x20, start - 0x20)
    write(D, "LP/BGM00001.AP", hdr.ljust(start, bytes(1)) + data + bytes([255]) * 0x100)
    data2 = interleave([enc(3000, 22050, f=(700,)), enc(3000, 22050, f=(900,))], 0x10)
    hdr2 = b"LEP " + u32s(1, len(data2), 0) + struct.pack("<HH", 0x3FFF, 22050)
    hdr2 = hdr2.ljust(0x58, bytes(1)) + u32le(0x420)
    write(D, "LP/VOICE.LEP", hdr2.ljust(0x800, bytes(1)) + data2)


# ---------------------------------------------------------------- IKM
def ikm_file(chans, rate, loop=None):
    data = interleave([enc(4000, rate, f=(400 + 150 * c,)) for c in range(chans)], 0x10)
    ls, le = loop or (0, 0)
    hdr = b"IKM\0" + u32s(0, 0, 0, 0, ls, le, 0, 3).ljust(0x3c, bytes(1))
    hdr += b"AST\0" + u32s(rate, 0, len(data), chans)
    return hdr.ljust(0x800, bytes(1)) + data


@builder
def ikm():
    write(D, "IKM/BGM01.IKM", ikm_file(2, 44100, loop=(1000, 3900)))
    write(D, "IKM/SE01.IKM", ikm_file(1, 22050))


# ---------------------------------------------------------------- JOE
@builder
def joe():
    cd = bytes([0xCD]) * 0x60
    # Counter Terrorism and later: 0x4020 header, interleave 0x10, 0xCD padding
    data = interleave([enc(3000, 32000, f=(300,)) + cd, enc(3000, 32000, f=(450,)) + cd], 0x10)
    write(D, "JOE/CTSF.JOE", u32s(32000, len(data), 8, 0).ljust(0x4020, bytes([0xCC])) + data)
    # NYR: 0x800 header, interleave 0x2000
    data = interleave([enc(5000, 44100, f=(500,)), enc(5000, 44100, f=(650,))], 0x2000)[:0x4000 + 0x1a00]
    write(D, "JOE/NYR.JOE", u32s(44100, len(data), 0x2000, 0xFFFFFFFF).ljust(0x800, bytes(1)) + data)
    # Sitting Ducks: size doubled, interleave 0x8000
    data = interleave([enc(2500, 22050, f=(800,)), enc(2500, 22050, f=(1000,))], 0x8000)
    write(D, "JOE/DUCKS.JOE", u32s(22050, len(data) * 2, 0xCCCCCCCC, 0xCCCCCCCC) + data)


# ---------------------------------------------------------------- ADM
def adm_file(blocks, freqs):
    """Blocks of 0x800 (0x400 per channel); every other block ends 3 lines early."""
    frames = [frames_of(enc(blocks * 64 * 28, 44100, f=(fq,), lead=False, end=False), 0) for fq in freqs]
    pos = 0
    out = bytearray()
    for k in range(blocks):
        full = k % 2 == 0
        n = 64 if full else 61
        for c in range(2):
            part = bytearray()
            for i in range(n):
                f = bytearray(frames[c][pos + i])
                if i == 0:
                    f[1] = 0x06 if full else 0x02
                if f[:4] == bytes(4):
                    f[2] = 0x11  # keep used lines distinguishable from unused (zero) ones
                part += f
            fill = bytes(16) if c == 1 else bytes([0x5A]) * 16
            out += part + fill * ((0x400 - len(part)) // 16)
        pos += n
    return bytes(out)


@builder
def adm():
    write(D, "ADM/MS_SAINT.ADM", adm_file(22, (440, 330)))
    write(D, "ADM/MS_ITEM.ADM", adm_file(21, (550, 660)))
    exe = bytearray(0x23BAF0 + 0x1C * 51)
    for i, name in enumerate([b"MS_TITLE.ADM", b"MS_SAINT.ADM", b"MS_ITEM.ADM"]):
        exe[0x23B3C0 + 0x20 * i:0x23B3C0 + 0x20 * i + len(name)] = name
    e = 0x23BAF0 + 0x1C
    exe[e:e + 0x10] = u32s(0x3000, 0x5800, 44100, 0)  # MS_SAINT loops from 0x3000
    e = 0x23BAF0 + 0x1C * 2
    exe[e:e + 0x10] = u32s(0, 0x5000, 44100, 1)  # MS_ITEM doesn't loop
    write(D, "ADM/SLPM_655.55", bytes(exe))


# ---------------------------------------------------------------- HXD
def hxd_header(entries, bank, interleave):
    n = len(entries)
    body = b"".join(struct.pack("<iIHHIHHII", rate, off, 0x0EB3, 0x64, 0, flags, 0, ls, le) for rate, off, flags, ls, le in entries)
    size = 0x20 + len(body)
    return b"\0DXH" + u32s(0x1000, n, 1 if bank else 0, size, interleave, 0, 0) + body


@builder
def hxd():
    sounds = [enc(1500, 22050, f=(500,)), enc(2500, 32000, f=(300,), loop=(4, 60)), enc(1200, 44100, f=(900,))]
    bd, entries = bytearray(), []
    for i, (s, rate) in enumerate(zip(sounds, [22050, 32000, 44100])):
        entries.append((rate, len(bd), 0x30 if i == 1 else 0x10, 5 * 16 // 0x20 if i == 1 else 0, (61 * 16) // 0x20 if i == 1 else 0))
        bd += s
        bd = bytearray(pad(bytes(bd), 0x40))
    entries.insert(2, (32000, entries[1][1], 0x10, 0, 0))  # repeated offset
    write(D, "HXD/SE_BANK.HXD", hxd_header(entries, True, 0))
    write(D, "HXD/SE_BANK.BD", bytes(bd))
    data = interleave([enc(6000, 44100, f=(330,)), enc(6000, 44100, f=(440,))], 0x800)[:0x2000 + 0x500]
    write(D, "HXD/BGM01.HXD", hxd_header([(44100, 0, 0x22, 0x40, 0), (44100, 0, 0x22, 0x40, 0)], False, 0x800))
    write(D, "HXD/BGM01.STR", data)


# ---------------------------------------------------------------- 2PFS
@builder
def pfs2():
    il = 0x1000
    data = interleave([enc(8000, 44100, f=(260,)), enc(8000, 44100, f=(390,))], il)
    # v1 music
    hdr = b"2PFS" + struct.pack("<HHII", 1, 4, 7, 0x50 + len(data)) + bytes([1]) + bytes(0x1F)
    hdr += u32s(0xDEADBEEF, len(data), 0x800 - 0x40, 1)
    hdr += bytes([2, 1]) + struct.pack("<HIII", 0x40, 44100, 0, 0)
    write(D, "2PFS/BGM_V1.SAP", hdr.ljust(0x800, bytes(1)) + data)
    # v2 music
    hdr = b"2PFS" + struct.pack("<HHII", 2, 1, 8, 0x60 + len(data)) + bytes([1]) + bytes(0x1F)
    hdr += u32s(0xDEADBEEF, len(data), 0x800 - 0x40, 1)
    hdr += bytes([2, 1, 0, 0]) + struct.pack("<IIIIII", 0x20, 32000, 0, 0, 1, 0)
    write(D, "2PFS/BGM_V2.SAP", hdr.ljust(0x800, bytes(1)) + data)
    # v2 bank of voices
    sounds = [(enc(1500, 24000, f=(600,)), 2048), (enc(2200, 12000, f=(300,)), 1024), (enc(1800, 48000, f=(1500,)), 4096)]
    body, entries = bytearray(), bytearray()
    for s, pitch in sounds:
        entries += bytes([1, 0, 0, 0]) + u32s(0x1000, pitch, len(body), len(s), 0, 0, 0)
        body += s
    hdr = b"2PFS" + struct.pack("<HHII", 2, 1, 9, 0) + bytes([3]) + bytes(0x1F)
    hdr += u32s(0xDEADBEEF, 0x20 * len(sounds), 0x10, 0, 0, len(body), 0x800 - 0x50, 0, 9, len(sounds), 127, 0)
    write(D, "2PFS/VOICE.IAP", (hdr + entries).ljust(0x800, bytes(1)) + body)


# ---------------------------------------------------------------- RXWS
def rxws_file(streams, names, with_body):
    """`streams`: (type, channels, rate, data, loop_start_bytes or -1)."""
    body = bytearray()
    entries = bytearray(u32le(len(streams)))
    for kind, ch, rate, data, ls in streams:
        entries += struct.pack("<BBHIBBHIIIi", kind, 0x1C, 0x8002 | (ls >= 0), 0x7F7F, 0, ch, rate, 0, len(body), len(data), ls)
        body += data
        body = bytearray(pad(bytes(body), 0x800))
    form = b"FORM" + u32s(len(entries), 0x100, 0) + entries
    offs, strs = [], bytearray()
    for n in names:
        offs.append(4 + 4 * len(names) + len(strs))
        strs += n + bytes(1)
    ftxt_body = u32le(len(names)) + b"".join(u32le(o) for o in offs) + strs
    ftxt = b"FTXT" + u32s(len(ftxt_body), 0x100, 0) + ftxt_body
    chunks = form + ftxt
    if with_body:
        chunks += b"BODY" + u32s(len(body), 0x100, 0) + body
    return b"RXWS" + u32s(len(chunks), 0x200, 0) + chunks, bytes(body)


@builder
def rxws():
    st = interleave([enc(3000, 44100, f=(400,)), enc(3000, 44100, f=(600,))], 0x10)
    pcm = pcm16le(tone(22050, 0.1, [700]))
    xws, _ = rxws_file([(0, 2, 44100, st, 0x400), (1, 1, 22050, pcm, -1), (0, 1, 32000, enc(2000, 32000, f=(250,)), -1)],
                       [b"bgm_title", b"se_click", b"se_door"], True)
    write(D, "RXWS/BANK.XWS", xws)
    xwh, xwb = rxws_file([(0, 1, 24000, enc(2500, 24000, f=(350,), loop=(3, 70)), 0x30), (0, 2, 48000, st, -1)], [b"a", b"b"], False)
    write(D, "RXWS/VOICE.XWH", xwh)
    write(D, "RXWS/VOICE.XWB", xwb)


# ---------------------------------------------------------------- RKV
@builder
def rkv():
    mono = enc(4000, 22050, f=(500,))
    write(D, "RKV/AMB01.RKV", u32s(22050, 500, 3900, 0).ljust(0x800, bytes(1)) + mono)
    st = interleave([enc(5000, 32000, f=(300,)), enc(5000, 32000, f=(420,))], 0x400)[:0x1000 + 0x260]
    write(D, "RKV/MUS01.RKV", u32s(0, 32000, 0xFFFFFFFF, 0, 1).ljust(0x800, bytes(1)) + st)


# ---------------------------------------------------------------- SDF
@builder
def sdf():
    rate = 8000
    long_l = psx_encode(tone(rate, 0.3, [300]) + [0] * (rate * 10))
    long_r = psx_encode(tone(rate, 0.3, [450]) + [0] * (rate * 10))
    data = interleave([long_l, long_r], 0x400)[:len(long_l) * 2 - 0x2a0]
    write(D, "SDF/HUGO_BGM.SDF", b"SDF\0" + u32s(3, len(data), rate, 2, 0x400) + data)
    data = enc(3000, 22050, f=(600,))
    write(D, "SDF/HUGO_SE.SDF", b"SDF\0" + u32s(3, len(data), 22050, 1, 0) + data)
    # NDS PCM16 (vgmstream r2117 only knows mono PCM16 here; newer sources add PCM8 and channels)
    pcm = pcm16le(tone(16000, 0.15, [500]))
    write(D, "SDF/NDS16.SDF", b"SDF" + bytes(1) + u32s(3, len(pcm), 0) + u32le(16000) + bytes([1, 1]) + struct.pack("<H", 0) + pcm)


# ---------------------------------------------------------------- SKEX
def vag_std(data, rate, name, version=0x20):
    return b"VAGp" + struct.pack(">IIII", version, 0, len(data), rate) + bytes(12) + name.ljust(16, bytes(1)) + data


def vag_kaudio_stereo(left, right, rate):
    cs = max(len(left), len(right))
    half = lambda d: (b"VAGp" + struct.pack(">IIII", 0x20, 0, cs, rate) + bytes(12) + b"KAudioDLL".ljust(16, bytes(1)) + d.ljust(cs, bytes(1)))
    return half(left) + half(right)


def skex_pack(files, version, table_inside):
    """`files`: (type, data). Returns (skx, tbl or None)."""
    body = bytearray()
    offs = []
    for kind, d in files:
        offs.append(0x800 + len(body))
        body += pad(d, 0x800)
    total = 0x800 + len(body)
    if version == 0x1070:
        ents = b"".join(u32s(o, k, 0) for o, (k, _) in zip(offs, files)) + u32s(total, 0, 0)
        table = ents
    else:
        ents = bytearray()
        for o, (k, _) in zip(offs, files):
            ents += u32le(o) + bytes([0, 0, 0, k])
        ents += u32le(total) + bytes(4)
        table = (b"STBL" + struct.pack("<HHHH", version, 3, len(files), 0)).ljust(0x50, bytes(1)) + ents
    if table_inside:
        hdr = b"SKEX" + struct.pack("<HHII", version, 3, 0, 0) + u32s(0x100, len(table)) + struct.pack("<H", len(files))
        skx = (hdr.ljust(0x100, bytes(1)) + table).ljust(0x800, bytes(1)) + body
        return skx, None
    hdr = b"SKEX" + struct.pack("<HHII", version, 3, 0, 0) + u32s(0, 0) + struct.pack("<H", len(files))
    return hdr.ljust(0x800, bytes(1)) + body, table


@builder
def skex():
    mono = vag_std(enc(2000, 22050, f=(500,)), 22050, b"sfx_hit")
    st = vag_kaudio_stereo(enc(3000, 32000, f=(300,)), enc(3000, 32000, f=(450,)), 32000)
    mono2 = vag_std(enc(1500, 44100, f=(900,), loop=(4, 40)), 44100, b"sfx_loop")
    skx, _ = skex_pack([(5, mono), (0, b"cfg" * 5), (12, st), (5, mono2)], 0x2070, True)
    write(D, "SKEX/NBA06.SKX", skx)
    skx, tbl = skex_pack([(5, mono2), (12, st)], 0x1070, False)
    write(D, "SKEX_TBL/MLB04.SKX", skx)
    write(D, "SKEX_TBL/MLB04.TBL", tbl)


# ---------------------------------------------------------------- VAS (KCEO)
def vas_ps2(n, rate, freqs, loop=None):
    data = interleave([enc(n, rate, f=(fq,)) for fq in freqs], 0x200)
    lf, ls = (1, loop) if loop is not None else (0, 0)
    return u32s(len(data), rate, 0x96, 0, lf, ls).ljust(0x800, bytes(1)) + data


@builder
def vas_kceo():
    write(D, "VAS/BGM.VAS", vas_ps2(6000, 44100, (300, 400), loop=0x400))
    subs = [vas_ps2(2000, 22050, (500, 600)), vas_ps2(2600, 32000, (700, 800), loop=0x200)]
    # PS2 container with offset table
    body, table = bytearray(), bytearray()
    for s in subs:
        table += u32s((0x1000 + len(body)) // 0x800, 0, len(s) - 0x800, 0)
        body += pad(s, 0x800)
    hdr = (bytes([0xAB, 0x8A, 0x5A, 0x00]) + u32s((0x800 + len(body)) // 0x800, len(subs), 0, len(subs))).ljust(0x94, bytes(1)) + u32s(1, len(table))
    write(D, "VAS/CONT_TBL.VAS", hdr.ljust(0x800, bytes(1)) + bytes(table).ljust(0x800, bytes(1)) + bytes(body))
    # PS2 container, files one after another
    body = b"".join(subs)
    hdr = (bytes([0xAB, 0x8A, 0x5A, 0x00]) + u32s(len(body) // 0x800, len(subs), 0, len(subs))).ljust(0x800, bytes(1))
    write(D, "VAS/CONT_SEQ.VAS", hdr + body)
    # PC: PCM16
    pcm = pcm16le([x for p in zip(tone(22050, 0.1, [440]), tone(22050, 0.1, [550])) for x in p])
    write(D, "VAS/PC.VAS", u32s(1, 2, 4, 22050, 0x100, len(pcm), 0x96, 0, 0, len(pcm), 0, 0, 0).ljust(0x800, bytes(1)) + pcm)


# ---------------------------------------------------------------- PSF / SCH
def psf_single(chans, rate_value, n, freqs, flags=None):
    frames = [enc(n, 22050, f=(fq,)) for fq in freqs[:chans]]
    blocks = max(len(f) for f in frames) // 16
    data = interleave([f.ljust(blocks * 16, bytes(1)) for f in frames], 0x10)
    if flags is None:
        flags = 0xC0 if chans == 2 else 0x21
    return b"PSF" + bytes([flags]) + u32le((rate_value << 20) | blocks) + data


@builder
def psf():
    write(D, "PSF/GE_MUS.PSF", psf_single(2, 3763, 4000, (300, 450)))
    write(D, "PSF/GE_SFX.PSF", psf_single(1, 1882, 2500, (700,)))
    # segmented: 7 segments (0 holds the tracks' entry points), 4 tracks
    points = [[1, 3, 4, 6], [2, 3, 4, 6], [1, 3, 4, 6], [1, 3, 4, 6], [1, 3, 5, 6], [1, 3, 4, 6], [1, 3, 4, 6]]
    segs = [psf_single(2, 3763, 600 + 150 * i, (200 + 60 * i, 260 + 60 * i)) for i in range(len(points))]
    table_len = 8 + 0x0C * len(points)
    body, table = bytearray(), bytearray()
    for p, s in zip(points, segs):
        table += u32le(table_len + len(body)) + struct.pack("<4H", *p)
        body += pad(s, 0x10)
    write(D, "PSF/CDS_THEME.PSF", b"PSF\x60" + u32le(len(points)) + bytes(table) + bytes(body))
    # SCH with internal PS2 sounds (PFSM), a BANK chunk, and the "HDRSND" prefix
    def pfsm(lang, res, pitch, n, fq):
        data = enc(n, 22050, f=(fq,))
        body = struct.pack("<iIIHBB", lang, res, len(data), pitch, 0xFF, 0xCC) + data
        return b"PFSM" + u32le(len(body)) + body
    chunks = pfsm(-1, 1234, 1882, 1500, 500) + b"BANK" + u32le(8) + bytes(8) + pfsm(2, 99, 1365, 2000, 800)
    write(D, "SCH/LEVEL1.SCH", b"SCH\0" + u32le(len(chunks)) + chunks)
    chunks = pfsm(0, 7, 4096, 1200, 1000)
    write(D, "SCH/LEVEL2.SCH", b"HDRSND" + bytes(8) + b"SCH\0" + u32le(len(chunks)) + chunks)



# ---------------------------------------------------------------- LP (rotated PCM)
@builder
def lp():
    """Enthusia "LP": PCM16 stored rotated right by one bit, AP-style header."""
    rotr = lambda v: ((v & 0xFFFF) >> 1 | (v & 1) << 15)
    il, start = 0x800, 0x800
    l, r = tone(44100, 0.3, [330]), tone(44100, 0.3, [440])
    enc_ch = lambda x: b"".join(struct.pack("<H", rotr(v)) for v in x)
    data = interleave([enc_ch(l), enc_ch(r)], il)
    end = start + len(data)
    hdr = b"LP  " + u32s(end - 0x20, 44100, il, 0x7F7F, start + il * 2, end - 0x20, start - 0x20)
    write(D, "LP/BGM00002.LP", hdr.ljust(start, bytes(1)) + data + bytes([255]) * 0x100)


# ---------------------------------------------------------------- PSF segments that need a decoder reset
@builder
def psf_reset():
    """Segmented PSF whose segments start without a silent lead frame, so each segment's
    first frame uses the predictor: decoding must restart at each segment like vgmstream."""
    def seg(n, freqs):
        frames = [psx_encode(tone(22050, n / 22050, [fq]), lead=False) for fq in freqs]
        blocks = max(len(f) for f in frames) // 16
        data = interleave([f.ljust(blocks * 16, bytes(1)) for f in frames], 0x10)
        return b"PSF" + bytes([0xC0]) + u32le((3763 << 20) | blocks) + data
    # same segment table as psf() above
    points = [[1, 3, 4, 6], [2, 3, 4, 6], [1, 3, 4, 6], [1, 3, 4, 6], [1, 3, 5, 6], [1, 3, 4, 6], [1, 3, 4, 6]]
    segs = [seg(600 + 150 * i, (200 + 60 * i, 260 + 60 * i)) for i in range(len(points))]
    table_len = 8 + 0x0C * len(points)
    body, table = bytearray(), bytearray()
    for pt, sg in zip(points, segs):
        table += u32le(table_len + len(body)) + struct.pack("<4H", *pt)
        body += pad(sg, 0x10)
    write(D, "PSF/RESET_THEME.PSF", b"PSF" + bytes([0x60]) + u32le(len(points)) + bytes(table) + bytes(body))


if __name__ == "__main__":
    want = sys.argv[1:] or list(BUILDERS)
    for w in want:
        BUILDERS[w]()
        print("built", w)
