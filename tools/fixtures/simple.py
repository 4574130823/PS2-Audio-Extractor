"""Test files for the small PS-ADPCM header formats (vgmstream meta/<name>.c)."""
import struct
import sys

from common import *  # noqa: F401,F403

D = out_dir("simple")
ONLY = set(sys.argv[1:])  # build only these formats (all when empty)

_cache = {}


def enc(samples, **kw):
    """psx_encode, memoized (the encoder is slow)."""
    key = (tuple(samples), tuple(sorted(kw.items())))
    if key not in _cache:
        _cache[key] = psx_encode(samples, **kw)
    return _cache[key]


def want(name):
    return not ONLY or name in ONLY


def L(rate, secs=0.3):
    return tone(rate, secs, [440, 660])


def R(rate, secs=0.3):
    return sweep(rate, secs, 300, 2000)


def stereo(rate, il, secs=0.3, fill=b"\0"):
    """Interleaved stereo PS-ADPCM padded to whole rows, and the per-channel size before padding."""
    l, r = enc(L(rate, secs)), enc(R(rate, secs))
    size = max(len(l), len(r))
    return interleave([l, r], il, fill), size


# ---------------------------------------------------------------- A2M
if want("a2m"):
    data, _ = stereo(22050, 0x6000)
    hdr = (b"A2M\0PS2\0" + bytes(8) + u32be(22050)).ljust(0x30, b"\0")
    write(D, "a2m/SCOOBY.INT", hdr + data)

# ---------------------------------------------------------------- VGV (by extension only)
if want("vgv"):
    d = enc(L(22050, 0.4))
    hdr = u32le(22050) + struct.pack("<f", len(d) / 16 * 28 / 22050) + bytes(8)
    write(D, "vgv/RUNE.VGV", hdr + d)

# ---------------------------------------------------------------- IIVB
if want("iivb"):
    l, r = enc(L(24000)), enc(R(24000))
    n = max(len(l), len(r))
    write(D, "iivb/BGM01.IVB", b"BVII" + u32le(n) + u32be(24000) + bytes(4) + l.ljust(n, b"\0") + r.ljust(n, b"\0"))

# ---------------------------------------------------------------- VMS
if want("vms"):
    def vms(chans, rate, il):
        data = interleave([enc(c) for c in chans], il) if len(chans) > 1 else enc(chans[0])
        vag = b"VAGp" + struct.pack(">IIII", 0x20, 0, len(data), rate) + bytes(12) + b"VMS".ljust(16, b"\0")
        hdr = b"VMS " + u32le(1) + u32le(len(chans)) + u32le(len(data) // 16) + u32le(il) + u32le(rate) + u32le(0x20) + u32le(0x50)
        return hdr + vag + data
    write(D, "vms/MONO.VMS", vms([L(22050)], 22050, 0))
    write(D, "vms/STEREO.VMS", vms([L(32000), R(32000)], 32000, 0x400))

# ---------------------------------------------------------------- hgC1
if want("hgc1"):
    l, r = enc(L(32000)), enc(R(32000))
    n = max(len(l), len(r))
    data = interleave([l.ljust(n, b"\0"), r.ljust(n, b"\0")], 0x10)
    write(D, "hgc1/KOTT2.STR", b"hgC1strm" + u32le(2) + u32le(n // 16) + u32le(32000) + bytes(12) + data)

# ---------------------------------------------------------------- SL3
if want("sl3"):
    data, _ = stereo(44100, 0x800)
    hdr = (b"SL3\0" + bytes(0x10) + u32le(2) + u32le(44100) + u32le(0) + u32le(0x800)).ljust(0x8000, b"\0")
    write(D, "sl3/TDU.MS", hdr + data)

# ---------------------------------------------------------------- SMSS
if want("smss"):
    l, r = enc(L(22050), loop=(4, 200)), enc(R(22050), loop=(4, 200))
    n = max(len(l), len(r))
    data = interleave([l, r], 0x800)
    hdr = (b"SMSS" + u32le(0x800) + u32le(0x800) + u32le(2) + u32le(22050) + u32le(0) + u32le(0x50) + u32le(n)).ljust(0x800, b"\0")
    write(D, "smss/TTA.VSF", hdr + data)

# ---------------------------------------------------------------- SVS
if want("svs"):
    for name, pitch in (("MUSIC001.BGM", 3763), ("AMB.SVS", 0x1000)):
        data, n = stereo(44100, 0x10)
        write(D, "svs/" + name, b"SVS\0" + u32le(3) + u32le(1) + u32le(n // 16) + u32le(pitch) + u32le(0x3fff) + bytes(8) + data)

# ---------------------------------------------------------------- BG00
if want("bg00"):
    data, n = stereo(44100, 0x800)
    vag = lambda: b"VAGp" + struct.pack(">IIII", 0x20, 0, n, 44100) + bytes(12) + b"BG".ljust(16, b"\0")
    hdr = (b"BG00" + bytes(12) + u32le(0x800)).ljust(0x40, b"\0") + vag() + vag()
    write(D, "bg00/IBARA.BG00", hdr.ljust(0x800, b"\0") + data)

# ---------------------------------------------------------------- HSF
if want("hsf"):
    data, _ = stereo(44100, 0x100)
    write(D, "hsf/v1/BGM.HSF", b"HSF\0" + u32le(3) + u32le(3763) + u32le(0x100) + data)
    data, _ = stereo(32000, 0x200)
    write(D, "hsf/v3/BGM.HSF", b"HSF " + u32le(3) + u32le(32000) + u32le(0x200) + data)

# ---------------------------------------------------------------- SVAG (SNK)
if want("svag_snk"):
    l, r = enc(L(22050), loop=(3, 150)), enc(R(22050), loop=(3, 150))
    n = max(len(l), len(r))
    data = interleave([l.ljust(n, b"\0"), r.ljust(n, b"\0")], 0x10)
    write(D, "svag_snk/WH.SVAG", b"VAGm" + u32le(0) + u32le(22050) + u32le(2) + u32le(n // 16) + u32le(0) + u32le(4) + u32le(n // 16 - 2) + data)
    m = enc(L(11025))
    write(D, "svag_snk/MONO.SVAG", b"VAGm" + u32le(0) + u32le(11025) + u32le(1) + u32le(len(m) // 16) + u32le(0) + u32le(0) + u32le(0) + m)

# ---------------------------------------------------------------- MSV
if want("msv"):
    d = enc(L(24000), end=False)
    write(D, "msv/FIGHT.MSV", b"MSVp" + u32be(0x20) + bytes(4) + u32be(len(d)) + u32be(24000) + bytes(12) + b"FIGHTCLUB".ljust(16, b"\0") + d)

# ---------------------------------------------------------------- SVGp (loop from frame flags)
if want("svgp"):
    l, r = enc(L(22050), loop=(10, 180)), enc(R(22050), loop=(10, 180))
    data = interleave([l, r], 0x400)
    hdr = (b"SVGp" + b"HUNTER_BGM".ljust(16, b"\0") + u32le(0x400) + u32le(len(data))).ljust(0x2c, b"\0") + u32be(22050)
    write(D, "svgp/HUNTER.SVG", hdr.ljust(0x800, b"\0") + data)

# ---------------------------------------------------------------- SMPL (.v0 alone: no dual stereo)
if want("smpl"):
    d = enc(L(22050))
    hdr = b"SMPL" + u32be(3) + bytes(4) + u32be(len(d) + 0x10) + u32be(22050) + bytes(12) + b"HOMURA".ljust(16, b"\0") + u32le(560) + bytes(12)
    write(D, "smpl/BGM00.V0", hdr + d)

# ---------------------------------------------------------------- VDS/VDM
if want("vds_vdm"):
    # VDS: stereo, looping (size field short: the file's size counts)
    data, n = stereo(32000, 0x800)
    hdr = (b"VDS " + u32le(len(data) - 0x800) + u32le(0x10) + u32le(32000) + u32le(2) + u32le(0x800)
           + u32le(0x800 + 0x1000) + u32le(0x800 + len(data)) + b"\x01\x7f\x40\x02").ljust(0x800, b"\0")
    write(D, "vds_vdm/GK.VDS", hdr + data)
    # VDM: mono, not looping (size field counts)
    d = enc(L(22050))
    hdr = (b"VDM " + u32le(len(d) - 0x20) + u32le(0x10) + u32le(22050) + u32le(1) + u32le(0)
           + u32le(0) + u32le(0) + b"\x00\x7f\x40\x04").ljust(0x800, b"\0")
    write(D, "vds_vdm/VOICE.VDM", hdr + d)

# ---------------------------------------------------------------- AHV
if want("ahv"):
    l, r = enc(L(22050)), enc(R(22050))
    n = max(len(l), len(r))
    l, r = l.ljust(n, b"\0"), r.ljust(n, b"\0")
    il = 0x400
    full = n // il * il
    data = interleave([l[:full], r[:full]], il) + l[full:] + r[full:]  # short last block
    vag = b"VAGp" + struct.pack(">IIII", 0x20, 0, n, 22050) + bytes(12) + b"AHV".ljust(16, b"\0")
    write(D, "ahv/HEADHUNT.AHV", (b"AHV\0" + u32le(0) + u32le(n) + u32le(22050) + u32le(il) + vag).ljust(0x800, b"\0") + data)
    m = enc(L(11025))
    write(D, "ahv/MONO.AHV", (b"AHV\0" + u32le(0) + u32le(len(m)) + u32le(11025) + u32le(0)).ljust(0x800, b"\0") + m)

# ---------------------------------------------------------------- MUSC
if want("musc"):
    data, _ = stereo(32000, 0x400)
    data = data + bytes(0x800 - len(data) % 0x800 if len(data) % 0x800 else 0)
    hdr = b"MUSC" + u16le(0) + u16le(32000) + bytes(8) + u32le(0x800) + u32le(len(data)) + u32le(0x800)
    write(D, "musc/TY.MUS", hdr.ljust(0x800, b"\0") + data)  # ends in 0-frames: loops
    data2, _ = stereo(22050, 0x400)
    data2 = data2[:-0x10] + b"\x0c\x00\x00\x00" + bytes(12)  # ends in silence: no loop
    hdr = b"MUSC" + u16le(0) + u16le(22050) + bytes(8) + u32le(0x800) + u32le(len(data2)) + u32le(0x800)
    write(D, "musc/SPYRO.MUSC", hdr.ljust(0x800, b"\0") + data2)

# ---------------------------------------------------------------- VPK
if want("vpk"):
    data, n = stereo(48000, 0x800)
    hdr = b" KPV" + u32le(n) + u32le(0x800) + u32le(0x1000) + u32le(48000) + u32le(2)
    write(D, "vpk/GOW.VPK", hdr.ljust(0x7fc, b"\0") + u32le(0) + data)
    data, n = stereo(44100, 0x200)
    hdr = b" KPV" + u32le(n) + u32le(0x800) + u32le(0x400) + u32le(44100) + u32le(2)
    write(D, "vpk/SLY.VPK", hdr.ljust(0x7fc, b"\0") + u32le(0x400) + data)

# ---------------------------------------------------------------- AST (MicroVision)
if want("ast_mv"):
    data, _ = stereo(32000, 0x4000)
    write(D, "ast_mv/PTO4.AST", (b"AST\0" + u32le(32000) + u32le(0x4000) + u32le(0x800 + len(data)) + u32be(0)).ljust(0x800, b"\0") + data)
    data, _ = stereo(44100, 0x800)
    write(D, "ast_mv/NOWG.AST", (b"AST\0" + u32le(44100) + u32le(0x800) + u32le(0x800 + len(data)) + u32be(0x20002000)).ljust(0x800, b"\xaa") + data)

# ---------------------------------------------------------------- AST (Marvelous)
if want("ast_mmv"):
    data, _ = stereo(24000, 0x800)
    hdr = b"AST\0" + u32le(0x100 + len(data)) + u32le(24000) + u32le(2) + u32le(0x800) + u32le(len(data) // 0x1000) + u32le(0) + struct.pack("<f", 0.3)
    write(D, "ast_mmv/REBORN.AST", (hdr + b"bgm_reborn_01").ljust(0x100, b"\0") + data)

# ---------------------------------------------------------------- SEB (by extension only)
if want("seb"):
    data, n = stereo(32000, 0x800)
    ns = n // 16 * 28
    hdr = u32le(2) + u32le(32000) + u32le(0) + u32le(0) + u32le(0x800 + 0x100) + u32le(280) + u32le(0x800 + len(data)) + u32le(ns) + u32le(0)
    write(D, "seb/BGM.SEB", hdr.ljust(0x800, b"\0") + data)
    d = enc(L(22050))
    ns = len(d) // 16 * 28 - 10
    hdr = u32le(1) + u32le(22050) + u32le(0) + u32le(0) + u32le(0) + u32le(0) + u32le(0x800 + len(d)) + u32le(ns) + u32le(1)
    write(D, "seb/0012.GMS", hdr.ljust(0x800, b"\0") + d)

# ---------------------------------------------------------------- MIC (by extension only)
if want("mic_koei"):
    data, n = stereo(44100, 0x10)
    hdr = u32le(0x800) + u32le(44100) + u32le(2) + u32le(0x10) + u32le(len(data) // 0x20) + u32le(20) + bytes(8)
    write(D, "mic_koei/CS2.MIC", hdr.ljust(0x800, b"\0") + data)
    l, r = enc(L(22050)), enc(R(22050))
    data = interleave([l, r, l, r], 0x20)
    hdr = u32le(0x800) + u32le(22050) + u32le(4) + u32le(0x20) + u32le(len(data) // 0x80) + u32le(1) + bytes(8)
    write(D, "mic_koei/DT2.MIC", hdr.ljust(0x800, b"\0") + data)

# ---------------------------------------------------------------- RSTM (Rockstar)
if want("rstm_rockstar"):
    data, n = stereo(32000, 0x10)
    hdr = b"RSTM" + u32le(0) + u32le(32000) + u32le(2) + bytes(8) + u32le(len(data)) + u32le(0x200) + u32le(len(data) - 0x40)
    write(D, "rstm_rockstar/MC3.RSM", hdr.ljust(0x800, b"\0") + data)
    d = enc(L(22050))
    hdr = b"RSTM" + u32le(0) + u32le(22050) + u32le(1) + bytes(8) + u32le(len(d)) + u32le(0) + u32le(len(d))
    write(D, "rstm_rockstar/BULLY.RSTM", hdr.ljust(0x800, b"\0") + d)

# ---------------------------------------------------------------- STER
if want("ster"):
    data, n = stereo(44100, 0x10)
    write(D, "ster/BAROQUE.STER", b"STER" + u32le(n) + u32le(0x50) + u32be(len(data)) + u32be(44100) + bytes(12) + b"baroque".ljust(16, b"\0") + data)
    data, n = stereo(22050, 0x10)
    write(D, "ster/SS.SFS", b"STER" + u32le(n) + u32le(0xFFFFFFFF) + u32be(len(data)) + u32be(22050) + bytes(12) + b"star".ljust(16, b"\0") + data)

# ---------------------------------------------------------------- VS/STR (The Bouncer, blocked)
if want("vs_str"):
    def vs_str(chans, ids):
        chunks = [[c[i:i + 0x7e0] for i in range(0, len(c), 0x7e0)] for c in chans]
        nb = max(len(c) for c in chunks)
        out = bytearray()
        for b in range(nb):
            size = len(chunks[0][b]) if b < len(chunks[0]) else 0
            if b == nb - 1:
                size += 8  # not a whole frame: vgmstream decodes whole frames only
            for c, cid in zip(chunks, ids):
                piece = c[b] if b < len(c) else b""
                out += (cid + u32le(size) + u32le(nb - 1 - b) + u32le(0) + u32le(0x1234)).ljust(0x20, b"\0") + piece.ljust(0x7e0, b"\0")
        return bytes(out)
    l, r = enc(L(44100)), enc(R(44100))
    n = max(len(l), len(r))
    write(D, "vs_str/BOUNCER.VS", vs_str([l.ljust(n, b"\0"), r.ljust(n, b"\0")], [b"STRL", b"STRR"]))
    write(D, "vs_str/VOICE.STR", vs_str([enc(L(44100, 0.2))], [b"STRM"]))

# ---------------------------------------------------------------- IAB (blocked)
if want("iab"):
    l, r = enc(L(32000)), enc(R(32000))
    n = max(len(l), len(r))
    l, r = l.ljust(n, b"\0"), r.ljust(n, b"\0")
    body, pos, sizes = bytearray(), 0, [0x400, 0x600, 0x400]
    i = 0
    while pos < n:
        cs = min(sizes[i % len(sizes)], n - pos)
        blk = u32le(0x48124812) + u32le(0) + u32le(cs * 2) + u32le(0x10 + cs * 2 + 0x20) + l[pos:pos + cs] + r[pos:pos + cs] + bytes(0x20)
        body += blk
        pos += cs
        i += 1
    body += u32le(0x48124812) + bytes(12)  # last block: empty, size 0
    total = 0x40 + len(body)
    hdr = u32be(0x10000000) + u32le(32000) + u32le(0) + u32le(0x400) + bytes(12) + u32le(total)
    write(D, "iab/UEKI.IAB", hdr.ljust(0x40, b"\0") + body)

# ---------------------------------------------------------------- FILp (blocked)
if want("filp"):
    def filp(l, r, rate, loop, sizes):
        n = max(len(l), len(r))
        l, r = l.ljust(n, b"\0"), r.ljust(n, b"\0")
        pieces, pos, i = [], 0, 0
        while pos < n:
            cs = min(sizes[i % len(sizes)], n - pos)
            pieces.append((l[pos:pos + cs], r[pos:pos + cs]))
            pos += cs
            i += 1
        total = sum(0x800 + 2 * len(a) for a, _ in pieces)
        out = bytearray()
        for a, b in pieces:
            vag = lambda: b"VAGp" + u32be(0x20) + bytes(4) + u32le(n) + u32le(rate) + bytes(12) + b"FIL".ljust(16, b"\0")
            hdr = (b"FILp" + u32le(2) + u32le(0) + u32le(total) + bytes(8) + u32le(0x800 + 2 * len(a))).ljust(0x34, b"\0") + u32le(0 if loop else 1)
            hdr = hdr.ljust(0x100, b"\0") + vag() + vag()
            out += hdr.ljust(0x800, b"\0") + a + b
        return bytes(out)
    write(D, "filp/DEADAIM.FIL", filp(enc(L(24000)), enc(R(24000)), 24000, True, [0x1000, 0x800]))
    write(D, "filp/NOLOOP.FIL", filp(enc(L(22050)), enc(R(22050)), 22050, False, [0x1800]))
