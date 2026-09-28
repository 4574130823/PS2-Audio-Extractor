"""Test files for every VAG/ADS/HD-BD/ADX/RIFF variant, encrypted ADX, ACX, AIX, FSB,
Sony BNK, MTAF, TAC, Ogg Vorbis.

    python tools/fixtures/major.py

Files whose layout vgmstream detects from the file size or extension (so they can't be
found inside an archive) use these extensions, to pass to check.py's --no-embed-ext:
    swag,vig,vas,str,vag,800,hd,bd,acx,aac,laac
"""
import struct
import sys

from common import *  # noqa: F401,F403

D = out_dir("major")
R = random.Random(77)
ONLY = set(sys.argv[1:])  # optional: only build these groups


def want(group):
    # Ogg Vorbis files are opt-in (python major.py ogg): lewton decodes a few samples 1 off
    # from vgmstream's libvorbis, so they can't pass check.py's exact comparison.
    if group == "ogg":
        return group in ONLY
    return not ONLY or group in ONLY


def fake_psx(nframes, lead=True, end=True, loop=None, last_flag=0x01, seed=None):
    """Random but valid PS-ADPCM frames (quiet, no clipping): fast to make. `loop`: (start
    frame, end frame) flagged 0x06 / 0x03. `last_flag`: flag of the last data frame."""
    r = random.Random(seed if seed is not None else R.random())
    out = bytearray(16) if lead else bytearray()
    for i in range(nframes):
        flag = 0
        if loop and i == loop[0]:
            flag = 0x06
        elif loop and i == loop[1]:
            flag = 0x03
        elif i == nframes - 1 and not loop:
            flag = last_flag
        pred = r.randint(0, 4)
        shift = r.randint(9, 12)
        out += bytes([(pred << 4) | shift, flag]) + bytes(r.randint(0, 255) for _ in range(14))
    if end:
        out += bytes([0x00, 0x07]) + bytes(14)
    return bytes(out)


def vagp_hdr(version, size, rate, name=b"", reserved=0, extra=b"\0" * 12, le=False):
    f = "<" if le else ">"
    return b"VAGp" + struct.pack(">I", version) + struct.pack(f + "III", reserved, size, rate) + extra + name.ljust(16, b"\0")[:16]


# ---------------------------------------------------------------------------- VAG
if want("vag"):
    # VAG1 mono [Metal Gear Solid 3]
    d = fake_psx(300)
    write(D, "VAG/MGS3.MSV", b"VAG1" + struct.pack(">IIII", 0, 0, len(d), 32000) + bytes(12) + b"mgs3".ljust(16, b"\0") + bytes(16) + d)
    # VAG2 [Metal Gear Solid 3]
    l, r = fake_psx(250), fake_psx(250)
    d = interleave([l, r], 0x800)
    write(D, "VAG/MGS3B.MSV", b"VAG2" + struct.pack(">IIII", 0, 0, len(l), 48000) + bytes(12) + b"vag2".ljust(16, b"\0") + bytes(16) + d)
    # VAGi
    l, r = fake_psx(180), fake_psx(180)
    d = interleave([l, r], 0x400)
    hdr = (b"VAGi" + struct.pack("<II", 0, 0x400) + struct.pack(">II", len(l), 24000)).ljust(0x800, b"\0")
    write(D, "VAG/VAGI.XA2", hdr + d)

    # pGAV stereo [Jak II]: header repeated at the start of each 0x2000 block
    def paired(l, r, il, hdr):
        first = il - 0x30
        size = max(len(l), len(r))

        def blocks(x):
            x = x.ljust(size, b"\0")
            return [x[:first]] + [x[first:][i:i + il] for i in range(0, max(0, len(x) - first), il)]
        bl, br = blocks(l), blocks(r)
        buf = bytearray()
        for i in range(max(len(bl), len(br))):
            w = first if i == 0 else il
            for side in (bl, br):
                buf += (hdr if i == 0 else b"") + (side[i] if i < len(side) else b"").ljust(w, b"\0")
        return bytes(buf)
    l, r = fake_psx(700), fake_psx(700)
    h = b"pGAV" + struct.pack("<IIII", 0x20, 0, len(l) * 2, 44100) + bytes(12) + b"jak".ljust(16, b"\0")
    write(D, "VAG/JAK2.MSV", paired(l, r, 0x2000, h))
    # pGAV mono
    d = fake_psx(150)
    write(D, "VAG/JAKX.MSV", b"pGAV" + struct.pack("<IIII", 0x20, 0, len(d), 22050) + bytes(12) + b"jakmono".ljust(16, b"\0") + d)
    # pGAV Army Men RTS (mono: the stereo variant is newer than the r2117 test exe)
    d = fake_psx(200, lead=False)
    write(D, "VAG/ARMYMEN.MSV", b"pGAV" + struct.pack(">I", 0x20000000) + struct.pack("<III", 0, len(d), 22050) + bytes(12) + b"name".ljust(16, b"\0") + bytes(d))
    # VAGp paired 0x6000 [The Simpsons Wrestling]
    l, r = fake_psx(1900), fake_psx(1900)
    write(D, "VAG/SIMPWRES.SVG", paired(l, r, 0x6000, vagp_hdr(0x20, len(l), 22050, b"wrestling")))
    # VAGp paired 0x1000 with loop flags [Shikigami no Shiro]
    l, r = fake_psx(600, loop=(30, 500)), fake_psx(600, loop=(30, 500))
    write(D, "VAG/SHIKIGAMI.XA2", paired(l, r, 0x1000, vagp_hdr(0x20, len(l), 44100, b"shiki")))
    # VAGp paired 0x800, version 0x20 [ModernGroove]
    l, r = fake_psx(400), fake_psx(400)
    write(D, "VAG/GROOVE.SVG", paired(l, r, 0x800, vagp_hdr(0x20, len(l), 44100, b"groove")))
    # Edge of Reality: little endian, long name, loop flags
    d = fake_psx(300, loop=(10, 250))
    hdr = vagp_hdr(0x02000000, len(d), 32000, b"EOR_LONG_STREAM_", le=True) + b"NAME_CONTINUES_H"
    write(D, "VAG/EOR.SND", hdr + d)
    # Killzone
    d = fake_psx(260)
    write(D, "VAG/KILLZONE.SND", vagp_hdr(0x40000000, len(d), 48000, b"kz", le=True) + bytes(16) + d)
    # Kingdom Hearts II: loops in the header, channels at 0x1e
    l, r = fake_psx(400), fake_psx(400)
    d = interleave([l, r], 0x10)
    extra = struct.pack(">ii", 1000, 9000) + bytes(4)
    hdr = bytearray(vagp_hdr(4, len(d), 44100, b"kh2", extra=extra))
    hdr[0x1c] = 1
    hdr[0x1e] = 2
    hdr[0x1f] = 0x7f
    write(D, "VAG/KH2.VAS", bytes(hdr) + bytes(0x30) + d)
    # The Simpsons Skateboarding: "STEREOVAG2K"
    l, r = fake_psx(300), fake_psx(300)
    d = interleave([l, r], 0x800)
    hdr = (vagp_hdr(0x20, len(l), 32000, b"skate") + b"STEREOVAG2K\0").ljust(0x800, b"\0")
    write(D, "VAG/SKATE.SVG", hdr + d)
    # Need for Speed: Hot Pursuit 2 ("VAGx"): interleave from the end flags
    l, r = fake_psx(253, end=False), fake_psx(253, end=False)
    d = interleave([l, r], 0x800)
    hdr = bytearray(vagp_hdr(2, len(d), 22050, extra=b"\0" * 12))
    hdr[0x24:0x28] = b"VAGx"
    hdr[0x2c:0x30] = struct.pack(">I", 2)
    write(D, "VAG/NFSHP2.SND", bytes(hdr) + d)
    # Garfield: Saving Arlene (padding at the end is ignored)
    l, r = fake_psx(150, end=False) + bytes(0x40), fake_psx(150, end=False) + bytes(0x40)
    d = interleave([l, r], 0x400)
    write(D, "VAG/GARFIELD.STR", (vagp_hdr(0x20, len(d), 22050, b"garf", reserved=1)).ljust(0x800, b"\0") + d)
    # Eko Software: stereo, full loops, short last blocks
    l, r = fake_psx(330, seed=5), fake_psx(330, seed=6)
    last = len(l) % 0x400
    d = bytearray()
    for i in range(0, len(l) - last, 0x400):
        d += l[i:i + 0x400] + r[i:i + 0x400]
    d += l[len(l) - last:] + r[len(r) - last:]
    write(D, "VAG/WOODY.MSV", vagp_hdr(0x20, len(d), 22050, b"eko", reserved=0x01010101).ljust(0x800, b"\0") + bytes(d))
    # THQ Australia: size field = file size + 0x10
    d = fake_psx(200, loop=(5, 150))
    write(D, "VAG/SPONGE.STR", vagp_hdr(0x20, len(d) + 0x30 + 0x10, 22050, b"thq") + d)
    # NBA 06 .SKX stereo (KAudioDLL)
    l, r = fake_psx(100), fake_psx(100)
    h = vagp_hdr(0x20, len(l), 32000, b"KAudioDLL")
    write(D, "VAG/NBA06.STR", h + l + h + r)
    # standard with a long "full loop" (repeated frame header + end frame)
    d = bytearray(fake_psx(3000, end=False, last_flag=0x01))
    d += bytes([d[-16], 0x07]) + bytes(14)
    write(D, "VAG/FULLLOOP.SVG", vagp_hdr(0x20, len(d), 4000, b"full") + bytes(d))
    # MX vs. ATV Untamed (.vig)
    l, r = fake_psx(160), fake_psx(160)
    d = interleave([l, r], 0x10)
    write(D, "VAG/MX.VIG", vagp_hdr(0x20, len(l), 32000, b"mx").ljust(0x7e0, b"\0") + d)
    # Frantix (.swag): two halves, each with a header
    l, r = fake_psx(120, loop=(3, 100)), fake_psx(120, loop=(3, 100))
    h = b"VAGp" + struct.pack(">I", 0x20) + struct.pack("<III", 0, len(l) - 0x10, 44100) + bytes(12) + bytes(16)
    half = (h + bytes(16) + l).ljust(0x40 + len(l), b"\0")
    write(D, "VAG/FRANTIX.SWAG", half + (h + bytes(16) + r))
    # AAAp [The Red Star]
    l, r = fake_psx(140), fake_psx(140)
    d = interleave([l, r], 0x800)
    hdr = b"AAAp" + struct.pack("<HH", 0x800, 2) + vagp_hdr(0x20, len(l), 22050, b"L") + vagp_hdr(0x20, len(l), 22050, b"R")
    write(D, "VAG/REDSTAR.VAG", hdr + d)
    # footer [The Sims 2: Pets] (extensionless)
    d = fake_psx(170, loop=(20, 160), lead=False)
    body = d.ljust((len(d) + 0x3f) // 0x40 * 0x40, b"\0")
    ftr = b"VAGp" + struct.pack("<IIII", 2, 0, len(d), 24000) + bytes(12) + b"petsound".ljust(16, b"\0") + bytes([1]) + bytes(15)
    write(D, "VAG/SIMSPETS", body + ftr)
    # Evolution Games [Rocket Power: Beach Bandits]
    d = fake_psx(210)
    hdr = b"   \0" + struct.pack("<I", 0) + b"   \0" + struct.pack("<II", len(d), 22050) + b"    " * 2 + b"   \0" + b"Evolution Games\0"
    write(D, "VAG/EVO.VAG", (hdr + d).ljust((len(hdr) + len(d) + 0x7f) // 0x80 * 0x80, b"\0"))

# ---------------------------------------------------------------------------- ADS
if want("ads"):
    def ads(codec, rate, ch, il, body, loop=(-1, -1), hsize=0x18, pre=b"", pad_to=None, size_field=None):
        hdr = b"SShd" + struct.pack("<IIIIIii", hsize, codec, rate, ch, il, *loop) + b"SSbd" + u32le(size_field if size_field is not None else len(body))
        if pad_to:
            hdr = (hdr + pre).ljust(pad_to, b"\0")
        return hdr + body
    # Capcom codec 0x02 with loop start address * 0x10 + trailing silent frames
    l, r = fake_psx(300, end=False), fake_psx(300, end=False)
    sil = bytes([0x0c, 0x01]) + bytes(14)
    body = interleave([l + sil * 4, r + sil * 4], 0x400)
    write(D, "ADS/CAPCOM.ADS", ads(0x02, 48000, 2, 0x400, body, loop=(0x20, -1)))
    # cavia loop (sector-aligned offset) in a "cavia stream" container
    l, r = fake_psx(400, end=False), fake_psx(400, end=False)
    sil = bytes([0x0c, 0x02]) + bytes(14)
    body = interleave([l + sil * 2, r + sil * 2], 0x800)
    write(D, "ADS/CAVIA.ADS", b"cavia stream".ljust(0x7d8, b"\0") + ads(0x10, 44100, 2, 0x800, body, loop=(0x1000, -1)))
    # Katakamuna: loop start not sector aligned (address * 0x10), mono PCM-less
    d = fake_psx(250)
    write(D, "ADS/KATA.ADS", ads(0x10, 22050, 1, 0x10, d, loop=(0x13, -1)))
    # Super Galdelic Hour: "PAD!" + loop start in PCM bytes, padded to 0x800
    l = pcm16le(tone(22050, 0.3, [300]))
    write(D, "ADS/GALDELIC.ADS", ads(0x01, 22050, 1, 0x200, l, loop=(4000, -1), pre=b"PAD!", pad_to=0x800))
    # PCM loops * 0x200 [Gofun-go no Sekai]
    l, r = pcm16le(tone(24000, 0.4, [440])), pcm16le(tone(24000, 0.4, [660]))
    body = interleave([l, r], 0x200)
    write(D, "ADS/GOFUN.ADS", ads(0x01, 24000, 2, 0x200, body, loop=(2, len(body) // 0x200 - 1)))
    # PCM loops * 0x70 [Armored Core - Nexus]
    body = interleave([l, r], 0x100)
    write(D, "ADS/ACNEXUS.ADS", ads(0x01, 24000, 2, 0x100, body, loop=(3, len(body) // 0x70 - 2)))
    # PSX loops * 0x20 [A.C.E.]
    l, r = fake_psx(500, end=False), fake_psx(500, end=False)
    body = interleave([l, r], 0x800)
    write(D, "ADS/ACE.ADS", ads(0x10, 48000, 2, 0x800, body, loop=(10, len(body) // 0x20 - 3)))
    # PSX loops in samples [Eve of Extinction], 4 channels
    chans = [fake_psx(300, end=False) for _ in range(4)]
    body = interleave(chans, 0x400)
    write(D, "ADS/EVE.ADS", ads(0x10, 44100, 4, 0x400, body, loop=(1000, 7000)))
    # Evergrace II: body padded to a sector (from the file size)
    l, r = fake_psx(200), fake_psx(200)
    body = interleave([l, r], 0x800)
    write(D, "ADS/EVERGRACE.ADS", ads(0x10, 44100, 2, 0x800, body, pad_to=0x800))
    # True Fortune: header size 0x20, odd body size (x2 - 0x10) (file size dependent)
    d = fake_psx(200, end=False) + bytes(16)
    write(D, "ADS/TRUEFORT.800", ads(0x10, 22050, 1, 0x10, d, hsize=0x20, size_field=(len(d) + 0x28 - 0x18 + 0x10) // 2))
    # ADSC container
    l, r = fake_psx(260), fake_psx(260)
    body = interleave([l, r], 0x800)
    hdr = b"SShd" + struct.pack("<IIIIIii", 0x18, 0x10, 32000, 2, 0x800, -1, -1) + b"SSbd" + u32le(len(body))
    sub = (hdr + struct.pack("<II", 0x1000, 0)).ljust(0x1000 - 8, b"\0") + body
    write(D, "ADS/KENKA.ADS", b"ADSC" + u32le(1) + sub)
    # 0x07 padding frames at the end are trimmed
    l, r = fake_psx(280, end=False) + bytes([0, 7]) + bytes(14), fake_psx(280, end=False) + bytes([0, 7]) + bytes(14)
    body = interleave([l, r], 0x200)
    write(D, "ADS/PADDED.SS2", ads(0x10, 48000, 2, 0x200, body))

# ---------------------------------------------------------------------------- HD/BD
if want("hdbd"):
    def hd_bd(samples, dummy=False, extra=True, unknown=0xFF):
        bd = bytearray()
        offsets = []
        for n, loop in samples:
            offsets.append(len(bd))
            bd += fake_psx(n)
        entries = list(zip(samples, offsets))
        count = len(entries) + (1 if dummy else 0)
        table_len = (0x10 + 4 * (count + 1) + 15) // 16 * 16
        infos, rel = bytearray(), []
        for ((n, loop), o) in entries:
            rel.append(table_len + len(infos))
            infos += struct.pack("<IHBB", o, 22050, 1 if loop else 0, unknown)
        if dummy:  # PrincessSoft: last entry points at the end of the .bd
            rel.append(table_len + len(infos))
            infos += struct.pack("<IHBB", len(bd), 22050, 0, unknown)
        stored = count - 1 if extra else count
        vagi = bytearray(b"IECSigaV") + struct.pack("<II", table_len + len(infos), stored)
        vagi += b"".join(u32le(r) for r in rel) + u32le(0)
        vagi = vagi.ljust(table_len, bytes([0xff])) + infos
        head_size = 0x40
        vagi_off = 0x10 + head_size
        hd_size = vagi_off + len(vagi)
        head = (b"IECSdaeH" + struct.pack("<IIII", head_size, hd_size, len(bd), 0xFFFFFFFF)
                + struct.pack("<III", 0xFFFFFFFF, 0xFFFFFFFF, vagi_off) + u32le(0xFFFFFFFF)).ljust(head_size, bytes([0xff]))
        return b"IECSsreV" + struct.pack("<II", 0x10, 0x01010000) + head + bytes(vagi), bytes(bd)
    hd, bd = hd_bd([(40, False), (60, True), (35, False)], dummy=True, extra=False, unknown=0)
    write(D, "HDBD/PRINCESS.HBD", hd + bd)
    hd, bd = hd_bd([(50, True), (45, False)])
    write(D, "HDBD/BANK.HD", hd)
    write(D, "HDBD/BANK.BD", bd)

# ---------------------------------------------------------------------------- ADX
def adx_file(chans, rate, version=0x0400, loop=None, ainf=False, cinf=False, key=None, enc=3, hist=None, align=None):
    """An ADX file. `key`: (type, xor, mult, add) encrypts the frame scales like CRI does."""
    c1, c2 = adx_coefs(rate)
    enc_frames = [adx_encode_channel(c, c1, c2) for c in chans]
    n = len(chans[0])
    ch = len(chans)
    body = bytearray(struct.pack(">BBBBIIHH", enc, 0x12, 4, ch, rate, n, 500, version))
    if version == 0x0300:
        if loop:
            body += struct.pack(">HHIIII", 0, 1, 1, loop[0], 0, loop[1]) + bytes(4)
    elif version in (0x0400, 0x0408, 0x0409):
        body += bytes(4)
        hsize = max(8, 4 * ch)
        hb = bytearray(hsize)
        for i, hv in enumerate(hist or []):
            hb[i * 4:i * 4 + 4] = struct.pack(">hh", hv, hv)
        body += hb
        if loop:
            body += struct.pack(">HHIIIII", 0, 1, 1, loop[0], 0, loop[1], 0)
        if ainf:
            body += b"AINF" + struct.pack(">I", 0x20) + b"str_id".ljust(0x10, b"\0") + bytes(0x10)
        if cinf:
            body += b"CINF" + struct.pack(">I", 0x60) + b"ASO ".ljust(0x20, b"\0") + b"SND ".ljust(0x20, b"\0") + b"NAME.ADX".ljust(0x20, b"\0")
    hdr_len = 4 + len(body) + 6
    if align:
        hdr_len = (hdr_len + align - 1) // align * align
    cpo = hdr_len - 4
    header = (struct.pack(">HH", 0x8000, cpo) + bytes(body)).ljust(hdr_len - 6, b"\0") + b"(c)CRI"
    data = bytearray()
    xors = None
    if key:
        _, x, m, a = key
        xors = []
        for _ in range(len(enc_frames[0]) * ch):
            xors.append(x)
            x = (x * m + a) & 0x7fff
    k = 0
    for i in range(len(enc_frames[0])):
        for e in enc_frames:
            f = bytearray(e[i])
            if xors:
                sc = struct.unpack(">H", f[:2])[0]
                f[:2] = struct.pack(">H", ((sc & 0x1fff) ^ xors[k]) & 0x7fff)
            k += 1
            data += f
    return header + bytes(data)


def key8(s):
    primes = KEY8_PRIMES
    k1, k2, k3 = primes[0x100], primes[0x200], primes[0x300]
    for c in s:
        m = primes[(c if c < 0x80 else c - 0x100) + 0x80]
        k1, k2, k3 = primes[k1 * m % 0x400], primes[k2 * m % 0x400], primes[k3 * m % 0x400]
    return k1, k2, k3


def key9(code, subkey=0):
    if subkey:
        code = (code * ((subkey << 16) | ((~subkey & 0xFFFF) + 2))) & (2**64 - 1)
    code -= 1
    return (code >> 27) & 0x7fff, ((code >> 12) & 0x7ffc) | 1, ((code << 1) & 0x7fff) | 1


def load_primes():
    import os
    import re
    src = open(os.path.join(TOOLS, "..", "src", "formats", "adx_keys.rs")).read()
    body = src[src.index("KEY8_PRIMES"):]
    return [int(x, 16) for x in re.findall(r"0x[0-9A-Fa-f]{4}", body)]


if want("adx"):
    KEY8_PRIMES = load_primes()
    assert key8(b"karaage") == (0x49e1, 0x4a57, 0x553d)
    t = tone(32000, 0.3, [440, 880])
    write(D, "ADX/V3LOOP.ADX", adx_file([t], 32000, version=0x0300, loop=(1000, 8000)))
    s1, s2 = sweep(44100, 0.3, 300, 3000), sweep(44100, 0.3, 3000, 300)
    write(D, "ADX/AINF.ADX", adx_file([s1, s2], 44100, loop=(2000, 12000), ainf=True, hist=[100, -100], align=0x800))
    write(D, "ADX/CINF.ADX", adx_file([t], 32000, loop=(500, 9000), cinf=True))
    write(D, "ADX/V5.ADX", adx_file([t], 32000, version=0x0500))
    x, m, a = key8(b"GHMSC")
    write(D, "ADX/KEY8.ADX", adx_file([s1, s2], 44100, version=0x0408, loop=(3000, 10000), key=(8, x, m, a)))
    x, m, a = key9(683461999)  # Kisou Ryouhei Gunhound EX
    write(D, "ADX/KEY9.ADX", adx_file([t], 32000, version=0x0409, key=(9, x, m, a)))
    x, m, a = key8(b"\x83\x76\x83\x89\x83\x60\x83\x69Lovers_Day")  # Shift-JIS keystring
    write(D, "ADX/KEY8SJIS.ADX", adx_file([t], 32000, version=0x0408, key=(8, x, m, a)))

# ---------------------------------------------------------------------------- RIFF
def chunk(cid, body):
    return cid + u32le(len(body)) + body + (b"\0" if len(body) % 2 else b"")


def riff(chunks):
    body = b"WAVE" + b"".join(chunks)
    return b"RIFF" + u32le(len(body)) + body


def fmt_pcm(ch, rate, bits, ext=False):
    ba = ch * bits // 8
    if ext:
        guid = struct.pack("<IHH", 1, 0, 0x10) + bytes([0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71])
        return chunk(b"fmt ", struct.pack("<HHIIHHH", 0xFFFE, ch, rate, rate * ba, ba, bits, 22) + struct.pack("<HI", bits, 3) + guid)
    return chunk(b"fmt ", struct.pack("<HHIIHH", 1, ch, rate, rate * ba, ba, bits))


def smpl(loops):
    b = struct.pack("<9I", 0, 0, 22675, 60, 0, 0, 0, len(loops), 0)
    for s, e in loops:
        b += struct.pack("<6I", 0, 0, s, e, 0, 0)
    return chunk(b"smpl", b)


def cue(points):
    b = u32le(len(points))
    for cid, p in points:
        b += struct.pack("<II", cid, p) + b"data" + struct.pack("<III", 0, 0, p)
    return chunk(b"cue ", b)


def adtl(items):
    b = b"adtl"
    for cid, payload in items:
        b += chunk(cid, payload)
    return chunk(b"LIST", b)


if want("riff"):
    st = [pcm16le(tone(22050, 0.3, [440])), pcm16le(tone(22050, 0.3, [550]))]
    stereo = b"".join(st[0][i:i + 2] + st[1][i:i + 2] for i in range(0, len(st[0]), 2))
    n = len(st[0]) // 2
    write(D, "WAV/SMPLST.WAV", riff([fmt_pcm(2, 22050, 16), chunk(b"data", stereo), smpl([(1000, 5000)])]))
    # smpl loop end == last sample (+1 would pass the end: vgmstream takes it back)
    write(D, "WAV/SMPLEND.WAV", riff([fmt_pcm(2, 22050, 16), smpl([(100, n)]), chunk(b"data", stereo)]))
    # two smpl loops: ignored
    write(D, "WAV/SMPL2.WAV", riff([fmt_pcm(2, 22050, 16), chunk(b"data", stereo), smpl([(10, 200), (300, 400)])]))
    # WAVEFORMATEXTENSIBLE PCM + JUNK
    write(D, "WAV/EXT.WAV", riff([chunk(b"JUNK", bytes(28)), fmt_pcm(2, 22050, 16, ext=True), chunk(b"data", stereo)]))
    # PCM8 with one cue (loop start to the end) [Source engine]
    p8 = bytes((s >> 8) + 128 for s in tone(11025, 0.4, [300]))
    write(D, "WAV/CUE8.WAV", riff([fmt_pcm(1, 11025, 8), cue([(1, 1500)]), chunk(b"data", p8)]))
    # cue + adtl labels (cues win, end + 1) [Advanced Power Dolls 2]
    mono = pcm16le(sweep(16000, 0.5, 200, 2000))
    write(D, "WAV/CUELABL.WAV", riff([fmt_pcm(1, 16000, 16), chunk(b"data", mono), cue([(2, 7000), (1, 1234)]),
                                     adtl([(b"labl", u32le(1) + b"Marker 00:00:00.07\0"), (b"labl", u32le(2) + b"Marker 00:00:00.43\0")])]))
    # cue + ltxt region [Touhou Suimusou]
    write(D, "WAV/CUERGN.WAV", riff([fmt_pcm(1, 16000, 16), chunk(b"data", mono), cue([(1, 800)]),
                                    adtl([(b"ltxt", struct.pack("<II", 1, 5000) + b"rgn " + bytes(8))])]))
    # labels only (milliseconds)
    write(D, "WAV/LABL.WAV", riff([fmt_pcm(1, 16000, 16), chunk(b"data", mono),
                                  adtl([(b"labl", u32le(2) + b"Marker 00:00:00.40\0"), (b"labl", u32le(1) + b"Marker 00:00:00.05\0")])]))
    # wsmp (DLS) loop: start + length
    write(D, "WAV/WSMP.WAV", riff([fmt_pcm(1, 16000, 16), chunk(b"wsmp", struct.pack("<IHhiII", 0x14, 60, 0, 0, 0, 1) + struct.pack("<IIII", 0x10, 0, 600, 4000)),
                                  chunk(b"data", mono)]))

# ---------------------------------------------------------------------------- ACX / AIX
def aix(segments, chunk_size=0x7e0):
    """`segments`: list of (layer ADX files, samples, rate)."""
    rate = segments[0][2]
    layers = len(segments[0][0])
    data_offset = 0x800
    segs, seg_data = [], bytearray()
    for files, samples, srate in segments:
        buf = bytearray()
        pos = [0] * layers
        while any(pos[i] < len(files[i]) for i in range(layers)):
            for i, f in enumerate(files):
                if pos[i] >= len(f):
                    continue
                part = f[pos[i]:pos[i] + chunk_size]
                pos[i] += len(part)
                body = bytes([i, layers]) + struct.pack(">hH", len(part), 0xFFFF) + bytes(2) + part
                body = body.ljust((len(body) + 0xf) // 0x10 * 0x10, b"\0")
                buf += b"AIXP" + u32be(len(body)) + body
        buf += b"AIXE" + u32be(0x18) + bytes(0x18)
        segs.append((data_offset + len(seg_data), len(buf), samples, srate))
        seg_data += buf
    hdr = bytearray(b"AIXF" + u32be(data_offset - 8) + u32be(0x01000014) + u32be(0x800) + bytes(8) + u16be(len(segs)) + bytes(6))
    for s in segs:
        hdr += struct.pack(">IIIi", *s)
    hdr += bytes([1]) + bytes(15)
    hdr += bytes([layers]) + bytes(7)
    for f in segments[0][0]:
        hdr += struct.pack(">ii", rate, f[7])
    return bytes(hdr.ljust(data_offset, b"\0")) + bytes(seg_data)


if want("aix"):
    a = sweep(32000, 0.4, 200, 1500)
    b = tone(32000, 0.4, [500])
    n = len(a)
    one = adx_file([a, b], 32000, hist=[1, 2])
    write(D, "AIX/ONELAYER.AIX", aix([([one], n, 32000)]))
    l2 = adx_file([b, a], 32000)
    l3 = adx_file([tone(32000, 0.4, [300])], 32000)
    write(D, "AIX/LAYERS.AIX", aix([([one, l2, l3], n, 32000)], chunk_size=0x3f5))
    # several segments (intro + loop, and intro + loop + end), each its own ADX
    intro = adx_file([tone(32000, 0.3, [220]), tone(32000, 0.3, [330])], 32000)
    loop = adx_file([sweep(32000, 0.5, 300, 900), tone(32000, 0.5, [440])], 32000)
    write(D, "AIX/INTRO_LOOP.AIX", aix([([intro], int(32000 * 0.3), 32000), ([loop], int(32000 * 0.5), 32000)]))
    s1a, s1b = adx_file([tone(32000, 0.2, [600])], 32000), adx_file([tone(32000, 0.2, [700])], 32000)
    s2a, s2b = adx_file([sweep(32000, 0.4, 200, 600)], 32000), adx_file([sweep(32000, 0.4, 600, 200)], 32000)
    s3a, s3b = adx_file([tone(32000, 0.25, [250])], 32000), adx_file([tone(32000, 0.25, [350])], 32000)
    write(D, "AIX/THREE_SEG.AIX", aix([([s1a, s1b], int(32000 * 0.2), 32000), ([s2a, s2b], int(32000 * 0.4), 32000),
                                       ([s3a, s3b], int(32000 * 0.25), 32000)]))
    acx_files = [adx_file([a], 32000, version=0x0300, loop=(100, 9000)), adx_file([b, a], 32000)]
    table = b"".join(struct.pack(">II", 0x100 + sum(len(x) + 0x20 for x in acx_files[:i]), len(f)) for i, f in enumerate(acx_files))
    body = bytearray((u32be(0) + u32be(len(acx_files)) + table).ljust(0x100, b"\0"))
    for f in acx_files:
        body += f + bytes(0x20)
    write(D, "ACX/SE.ACX", bytes(body))

# ---------------------------------------------------------------------------- FSB
def fsb_sample_hdr(name, n, size, ls, le, mode, rate, ch, hsize=0x40):
    h = struct.pack("<H", hsize) + name.ljust(0x1e, b"\0")[:0x1e] + struct.pack("<IIiiIiHHHH", n, size, ls, le, mode, rate, 255, 128, 128, ch)
    return h.ljust(hsize, b"\0")


def fsb(ver, samples, flags=0, basic=False):
    """`samples`: (name, data, num_samples, loop_start, loop_end, mode, rate, channels)."""
    base, hmin, version = {2: (0x10, 0x40, None), 3: (0x18, 0x40, 0x00030000), 4: (0x30, 0x50, 0x00040000)}[ver]
    hdrs, data = bytearray(), bytearray()
    for i, (name, d, n, ls, le, mode, rate, ch) in enumerate(samples):
        if basic and i > 0:
            hdrs += struct.pack("<II", n, len(d))
        else:
            hdrs += fsb_sample_hdr(name, n, len(d), ls, le, mode, rate, ch, hmin)
        data += d
    head = b"FSB%d" % ver + struct.pack("<iII", len(samples), len(hdrs), len(data))
    if version is not None:
        head += struct.pack("<II", version, flags)
    head = head.ljust(base, b"\0")
    return head + bytes(hdrs) + bytes(data)


if want("fsb"):
    VAGM, STEREO, LOOPOFF, LOOPN, BITS16, BITS8, UNS = 0x800000, 0x40, 1, 2, 0x10, 8, 0x80
    l, r = fake_psx(150, lead=False), fake_psx(150, lead=False)
    st = interleave([l, r], 0x10)
    m1 = fake_psx(90, lead=False)
    n_st = len(l) // 16 * 28
    n_m1 = len(m1) // 16 * 28
    # FSB3 (PS2): stereo VAG with a loop, a mono one (full loop, small: disabled), a looped-off one
    write(D, "FSB/BANK3.FSB", fsb(3, [(b"music", st, n_st, 280, 3000, VAGM | STEREO | LOOPN, 32000, 2),
                                     (b"sfx", m1, n_m1, 0, n_m1 - 1, VAGM, 22050, 1),
                                     (b"voice", m1, n_m1, 0, 1000, VAGM | LOOPOFF, 22050, 1)]))
    # FSB3 basic headers: later samples reuse the first one's mode/rate/channels
    write(D, "FSB/BASIC3.FSB", fsb(3, [(b"a", m1, n_m1, 0, 0, VAGM | LOOPOFF, 22050, 1), (b"", m1[:0x400], 0x400 // 16 * 28, 0, 0, 0, 0, 0)], flags=2, basic=True))
    # FSB2 non-interleaved stereo VAG
    write(D, "FSB/NONIL2.FSB", fsb(2, [(b"ni", l + r, n_st, 0, 0, VAGM | STEREO | LOOPOFF, 24000, 2)]))
    write(D, "FSB/NONIL3.FSB", fsb(3, [(b"ni", l + r, n_st, 0, 0, VAGM | STEREO | LOOPOFF, 24000, 2)], flags=0x10))
    # FSB4 PCM16 and PCM8 unsigned
    p16 = pcm16le(tone(22050, 0.3, [440]))
    p8 = bytes((s >> 8) + 128 for s in tone(11025, 0.3, [300]))
    write(D, "FSB/PCM4.FSB", fsb(4, [(b"p16", p16, len(p16) // 2, 100, 5000, BITS16 | LOOPN, 22050, 1),
                                    (b"p8", p8, len(p8), 0, 0, BITS8 | UNS | LOOPOFF, 11025, 1)]))
    # FSB1
    d = fake_psx(120, lead=False)
    n1 = len(d) // 16 * 28
    h1 = b"FSB1" + struct.pack("<iI", 1, len(d)) + bytes(4) + b"fsb1sound".ljust(0x20, b"\0") + struct.pack("<IIi", n1, len(d), 22050) + bytes(8) + struct.pack("<Iii", VAGM | LOOPN, 500, 2000)
    write(D, "FSB/ONE.FSB", h1 + d)

# ---------------------------------------------------------------------------- Sony BNK
def bnk_sbv2(streams):
    """Bank v1 / SBv2 (Jak and Daxter): `streams`: (data, center_note, flags)."""
    sblk = 0x20
    t1, t2 = 0x40, 0x60
    t3 = t2 + 8 * len(streams)
    data = bytearray()
    tone_entries = bytearray()
    for d, center, flags in streams:
        tone_entries += struct.pack("<BBBBhBBBBHHHII", 0, 0x7f, center, 0, 0x40, 0, 0x7f, 0, 0, 0, 0, flags, len(data), 0)
        data += d
    sb = bytearray(b"SBv2" + u32le(2)).ljust(0x14, b"\0")
    sb += struct.pack("<HHHH", 1, len(streams), len(streams), 0) + struct.pack("<III", t1, t2, t3)
    sb = sb.ljust(t2, b"\0")
    for i in range(len(streams)):
        sb += struct.pack("<BBHI", 1, 0x7f, 0, t3 + i * 0x18)
    sb += tone_entries
    sb = sb.ljust((len(sb) + 0xf) // 0x10 * 0x10, b"\0")
    data_off = sblk + len(sb)
    hdr = struct.pack("<IIIIII", 1, 2, sblk, len(sb), data_off, len(data)).ljust(sblk, b"\0")
    return hdr + bytes(sb) + bytes(data)


def bnk_sblk3(streams, version=3, big=False):
    """Bank v3 / SBlk v3-5: `streams`: (data, center_note, flags, size_field)."""
    e = ">" if big else "<"
    sblk = 0x20
    t1, t2 = 0x40, 0x60
    t3 = t2 + 8 * len(streams)
    data = bytearray()
    tones = bytearray()
    for d, center, flags, size in streams:
        tones += struct.pack(e + "BBBBhBBBBHHHII", 0, 0x7f, center, 0, 0x40, 0, 0x7f, 0, 0, 0, 0, flags, len(data), size)
        data += d
    sb = bytearray((b"klBS" if big else b"SBlk") + struct.pack(e + "I", version)).ljust(0x16, b"\0")
    sb += struct.pack(e + "HHH", 1, len(streams), len(streams)) + struct.pack(e + "II", t1, t2)
    sb = sb.ljust(0x34, b"\0") + struct.pack(e + "II", t3, 0)
    sb = sb.ljust(t1, b"\0") + struct.pack(e + "BBBBII", 0, len(streams), 0, 0, 0, 0)
    sb = sb.ljust(t2, b"\0")
    for i in range(len(streams)):
        sb += struct.pack(e + "I", 0x01000000 | (i * 0x18)) + bytes(4)
    sb += tones
    sb = sb.ljust((len(sb) + 0xf) // 0x10 * 0x10, b"\0")
    data_off = sblk + len(sb)
    hdr = struct.pack(e + "IIIIII", 3, 2, sblk, len(sb), data_off, len(data)).ljust(sblk, b"\0")
    return hdr + bytes(sb) + bytes(data)


if want("bnk"):
    a = fake_psx(120, loop=(10, 100))
    b = fake_psx(90)
    write(D, "BNK/JAK.BNK", bnk_sbv2([(a, 0xc4, 0), (b, 0xb8, 0)]))
    p = pcm16le(tone(24000, 0.2, [440]))
    write(D, "BNK/YUGIOH.BNK", bnk_sblk3([(a, 0xc4, 0x40, len(a)), (b, 0xbd, 0, 0), (p, 0xb8, 0x80, len(p))]))
    # two subsongs of equal size = one stereo stream
    c, d = fake_psx(100), fake_psx(100)
    write(D, "BNK/ATV.BNK", bnk_sblk3([(c, 0xc4, 0, len(c)), (d, 0xc4, 0, len(d))]))
    write(D, "BNK/PS3.BNK", bnk_sblk3([(a, 0xc4, 0, len(a)), (b, 0xc0, 0, len(b))], version=4, big=True))

# ---------------------------------------------------------------------------- MTAF
def mtaf(tracks, frames, loop=None, name=b""):
    r = random.Random(len(name) + tracks * 7 + frames)
    samples = frames * 256 - 100
    h = bytearray(b"MTAF" + u32le(0x800 + frames * 0x110 * tracks) + bytes(0x18))
    h += bytes(b ^ 0xFF for b in name.ljust(0x20, b"\0")) if name else bytes(0x20)
    h += b"HEAD" + u32le(0xB0) + u32le(0) + u32le(2 * tracks) + u32le(127) + struct.pack("<HH", 64, 0)
    h += struct.pack("<ii", loop[0] if loop else 0, samples) + u32le(0x110 * tracks)
    h += struct.pack("<ii", (loop[0] if loop else 0) // 256, samples // 256) + u32le(0) + u32le(5 if loop else 4)
    h = h.ljust(0x7f8, b"\0") + b"DATA" + u32le(frames * 0x100 * tracks)
    data = bytearray()
    for f in range(frames):
        for t in range(tracks):
            data += struct.pack("<BBHhhhHhH", t, tracks, 0, r.randint(0, 20), r.randint(0, 20), r.randint(-3000, 3000), 0, r.randint(-3000, 3000), 0)
            data += bytes(r.choice([0x10, 0x81, 0x9a, 0x08, 0x77, 0xf0, 0x12, 0x34]) for _ in range(0x100))
    return bytes(h) + bytes(data)


if want("mtaf"):
    write(D, "MTAF/BGM.MTA", mtaf(1, 20, loop=(1000, 0), name=b"bgm_01.mta"))
    write(D, "MTAF/QUAD.MTA", mtaf(2, 12))

# ---------------------------------------------------------------------------- TAC
TAC_BLOCK = 0x4E000


def crc16_genibus(data):
    crc = 0xFFFF
    for b in data:
        crc ^= b << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) if crc & 0x8000 else (crc << 1)
            crc &= 0xFFFF
    return crc ^ 0xFFFF


class RangeEncoder:
    """Carry-less range coder mirroring tac_lib.c's read_codes (Subbotin style)."""
    M = 0xFFFFFFFF

    def __init__(self):
        self.low, self.range, self.out = 0, 0xFFFFFFFF, bytearray()

    def encode(self, cum, freq):
        self.range >>= 14
        self.low = (self.low + cum * self.range) & self.M
        self.range = (self.range * freq) & self.M
        while (self.low ^ ((self.low + self.range) & self.M)) <= 0xFFFFFF:
            self.out.append(self.low >> 24)
            self.low = (self.low << 8) & self.M
            self.range = (self.range << 8) & self.M
        while self.range <= 0xFFFF:
            self.out.append(self.low >> 24)
            self.range = (((~self.low) + 1) & 0xFFFF) << 8
            self.low = (self.low << 8) & self.M

    def finish(self):
        for _ in range(4):
            self.out.append(self.low >> 24)
            self.low = (self.low << 8) & self.M
        return bytes(self.out)


def tac_file(nframes, frame_last, joint=0, loop=None, seed=3, split_at=None):
    r = random.Random(seed)
    # frequency model: symbols 0..40 used, 41 is a never-used filler to reach 16384
    freqs = [0] * 256
    for s in range(41):
        freqs[s] = max(1, int(3000 / (1 + s)))
    freqs[41] = 16384 - sum(freqs)
    assert freqs[41] > 0
    cum = [0]
    for f in freqs:
        cum.append(cum[-1] + f)
    table = bytearray()
    for f in freqs:
        table += bytes([f]) if f < 0x80 else bytes([(f & 0x7F) | 0x80, f >> 7])

    def sym(v):
        return 2 * v if v >= 0 else -2 * v - 1

    frames = []
    for fi in range(nframes):
        enc = RangeEncoder()
        count = 0
        for ch in range(2):
            vals = [r.randint(14, 19)]  # base scale index
            bands = [r.choice([0, 0, r.randint(1, 18)]) if b < 20 else 0 for b in range(27)]
            if ch == 1 and joint:
                bands = [0] * 27
            vals += bands
            for b in bands:
                if b:
                    vals += [r.randint(-4, 4) for _ in range(32)]
            for v in vals:
                s = sym(v)
                enc.encode(cum[s], freqs[s])
            count += len(vals)
        stream = enc.finish()
        base, data = stream[:4], stream[4:]
        body = struct.pack("<HH", fi + 1, count) + base + data
        frame = struct.pack("<HH", crc16_genibus(body), len(body) - 4) + body
        frames.append(frame)
    blocks = [bytearray()]
    header_len = 0x20
    first = bytearray(header_len) + table
    blocks[0] += first
    for i, f in enumerate(frames):
        if split_at is not None and i == split_at:
            blocks[0] += bytes([0xFF] * 4)  # "next block" marker
            blocks.append(bytearray())
        blocks[-1] += f
    stream_size = len(blocks) * TAC_BLOCK
    loop_frame, loop_discard, loop_offset = loop if loop else (0, 0, stream_size)
    hdr = struct.pack("<IIHHHHIIII", header_len, 0x1234, loop_frame, loop_discard, nframes, frame_last, loop_offset, stream_size, joint, 0)
    blocks[0][:0x20] = hdr
    out = bytearray()
    for i, b in enumerate(blocks):
        out += b.ljust(TAC_BLOCK, b"\0") if i + 1 < len(blocks) else b
    if len(out) < TAC_BLOCK:
        out = out.ljust(TAC_BLOCK, b"\0")
    return bytes(out)


if want("tac"):
    write(D, "TAC/SO3.LAAC", tac_file(30, 500, loop=(1, 100, 0), split_at=18))
    write(D, "TAC/VP2.AAC", tac_file(12, 1023, joint=1, seed=9))

# ---------------------------------------------------------------------------- Ogg Vorbis
# Small Vorbis streams (libvorbis via libsndfile), base64 below (OGG_B64); their comment
# header is rewritten here to test vgmstream's loop tag conventions.
def ogg_crc(data):
    crc = 0
    for b in data:
        crc ^= b << 24
        for _ in range(8):
            crc = ((crc << 1) ^ 0x04C11DB7) if crc & 0x80000000 else (crc << 1)
            crc &= 0xFFFFFFFF
    return crc


def ogg_pages(b):
    pos, out = 0, []
    while pos < len(b):
        assert b[pos:pos + 4] == b"OggS"
        nseg = b[pos + 26]
        segs = list(b[pos + 27:pos + 27 + nseg])
        body = b[pos + 27 + nseg:pos + 27 + nseg + sum(segs)]
        out.append({"flags": b[pos + 5], "granule": b[pos + 6:pos + 14], "serial": b[pos + 14:pos + 18], "segs": segs, "body": body})
        pos += 27 + nseg + sum(segs)
    return out


def ogg_page(flags, granule, serial, seq, segs, body):
    h = bytearray(b"OggS" + bytes([0, flags]) + granule + serial + struct.pack("<I", seq) + bytes(4) + bytes([len(segs)]) + bytes(segs))
    page = h + body
    page[22:26] = struct.pack("<I", ogg_crc(page))
    return bytes(page)


def ogg_with_comments(raw, comments):
    pages = ogg_pages(raw)
    # the three header packets
    packets, cur, header_pages = [], b"", 0
    for p in pages:
        pos = 0
        for s in p["segs"]:
            cur += p["body"][pos:pos + s]
            pos += s
            if s < 255:
                packets.append(cur)
                cur = b""
        header_pages += 1
        if len(packets) >= 3:
            break
    ident, setup = packets[0], packets[2]
    vendor = b"fixtures"
    com = b"\x03vorbis" + struct.pack("<I", len(vendor)) + vendor + struct.pack("<I", len(comments))
    for c in comments:
        com += struct.pack("<I", len(c)) + c.encode()
    com += b"\x01"
    serial, zero = pages[0]["serial"], bytes(8)
    out = [ogg_page(0x02, zero, serial, 0, [len(ident)], ident)]
    # comment + setup, split in pages of up to 255 segments
    segs, body, cont = [], b"", 0
    seq = 1
    for pk in (com, setup):
        n = len(pk)
        lace = [255] * (n // 255) + [n % 255]
        pos = 0
        for s in lace:
            segs.append(s)
            body += pk[pos:pos + s]
            pos += s
            if len(segs) == 255:
                out.append(ogg_page(cont, zero, serial, seq, segs, body))
                seq += 1
                cont = 0x01 if s == 255 else 0
                segs, body = [], b""
    if segs:
        out.append(ogg_page(cont, zero, serial, seq, segs, body))
        seq += 1
    for p in pages[header_pages:]:
        out.append(ogg_page(p["flags"], p["granule"], serial, seq, p["segs"], p["body"]))
        seq += 1
    return b"".join(out)


OGG_B64 = {
    'mono': (
        'T2dnUwACAAAAAAAAAACEEewLAAAAACUmKjwBHgF2b3JiaXMAAAAAASJWAAAAAAAAN7AAAAAAAACpAU9nZ1MAAAAAAAAAAAAA'
        'hBHsCwEAAABIV9IkDlr////////////////FA3ZvcmJpczQAAABYaXBoLk9yZyBsaWJWb3JiaXMgSSAyMDIwMDcwNCAoUmVk'
        'dWNpbmcgRW52aXJvbm1lbnQpAQAAABIAAABFTkNPREVSPWxpYnNuZGZpbGUBBXZvcmJpcyJCQ1YBAEAAABhCECoFrWOOOsgV'
        'IYwZoqBCyinHHULQIaMkQ4g6xjXHGGNHuWSKQsmB0JBVAABAAACkHFdQckkt55xzoxhXzHHoIOecc+UgZ8xxCSXnnHOOOeeS'
        'co4x55xzoxhXDnIpLeecc4EUR4pxpxjnnHOkHEeKcagY55xzbTG3knLOOeecc+Ygh1JyrjXnnHOkGGcOcgsl55xzxiBnzHHr'
        'IOecc4w1t9RyzjnnnHPOOeecc84555xzjDHnnHPOOeecc24x5xZzrjnnnHPOOeccc84555xzIDRkFQCQAACgoSiK4igOEBqy'
        'CgDIAAAQQHEUR5EUS7Ecy9EkDQgNWQUAAAEACAAAoEiGpEiKpViOZmmeJnqiKJqiKquyacqyLMuy67ouEBqyCgBIAABQURTF'
        'cBQHCA1ZBQBkAAAIYCiKoziO5FiSpVmeB4SGrAIAgAAABAAAUAxHsRRN8STP8jzP8zzP8zzP8zzP8zzP8zzP8zwNCA1ZBQAg'
        'AAAAgihkGANCQ1YBAEAAAAghGhlDnVISXAoWQhwRQx1CzkOppYPgKYUlY9JTrEEIIXzvPffee++B0JBVAAAQAABhFDiIgcck'
        'CCGEYhQnRHGmIAghhOUkWMp56CQI3YMQQrice8u59957IDRkFQAACADAIIQQQgghhBBCCCmklFJIKaaYYoopxxxzzDHHIIMM'
        'Muigk046yaSSTjrKJKOOUmsptRRTTLHlFmOttdacc69BKWOMMcYYY4wxxhhjjDHGGCMIDVkFAIAAABAGGWSQQQghhBRSSCmm'
        'mHLMMcccA0JDVgEAgAAAAgAAABxFUiRHciRHkiTJkixJkzzLszzLszxN1ERNFVXVVW3X9m1f9m3f1WXf9mXb1WVdlmXdtW1d'
        '1l1d13Vd13Vd13Vd13Vd13Vd14HQkFUAgAQAgI7kOI7kOI7kSI6kSAoQGrIKAJABABAAgKM4iuNIjuRYjiVZkiZplmd5lqd5'
        'mqiJHhAasgoAAAQAEAAAAAAAgKIoiqM4jiRZlqZpnqd6oiiaqqqKpqmqqmqapmmapmmapmmapmmapmmapmmapmmapmmapmma'
        'pmmapmkCoSGrAAAJAAAdx3EcR3Ecx3EkR5IkIDRkFQAgAwAgAABDURxFcizHkjRLszzL00TP9FxRNnVTV20gNGQVAAAIACAA'
        'AAAAAADHczzHczzJkzzLczzHkzxJ0zRN0zRN0zRN0zRN0zRN0zRN0zRN0zRN0zRN0zRN0zRN0zRN0zRN0zRNA0JDVgIAZAAA'
        'EJOQSk6xV0YpxiS0XiqkFJPUe6iYYkw67alCBikHuYdKIaWg094ypZBSDHunmELIGOqhg5AxhbDX2nPPvfceCA1ZEQBEAQAA'
        'xiDGEGPIMSYlgxIxxyRkUiLnnJROSialpFZazKSEmEqLkXNOSiclk1JaC6llkkprJaYCAAACHAAAAiyEQkNWBABRAACIMUgp'
        'pBRSSjGnmENKKceUY0gp5ZxyTjnHmHQQKucYdA5KpJRyjjmnnHMSMgeVcw5CJp0AAIAABwCAAAuh0JAVAUCcAACAkHOKMQgR'
        'YxBCCSmFUFKqnJPSQUmpg5JSSanFklKMlXNSOgkpdRJSKinFWFKKLaRUY2kt19JSjS3GnFuMvYaUYi2p1Vpaq7nFWHOLNffI'
        'OUqdlNY6Ka2l1mpNrdXaSWktpNZiaS3G1mLNKcacMymthZZiK6nF2GLLNbWYc2kt1xRjzynGnmusucecgzCt1ZxayznFmHvM'
        'seeYcw+Sc5Q6Ka11UlpLrdWaWqs1k9Jaaa3GkFqLLcacW4sxZ1JaLKnFWFqKMcWYc4st19BarinGnFOLOcdag5Kx9l5aqznF'
        'mHuKreeYczA2x547SrmW1nourfVecy5C1tyLaC3n1GoPKsaec87B2NyDEK3lnGrsPcXYe+45GNtz8K3W4FvNRcicg9C5+KZ7'
        'MEbV2oPMtQiZcxA66CJ08Ml4lGoureVcWus91hp8zTkI0VruKcbeU4u9156bsL0HIVrLPcXYg4ox+JpzMDrnYlStwcecg5C1'
        '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'
        '2p5fT16inTZdj96JJzedjt6Tl2jVzTsd4QA='
    ),
    'stereo': (
        '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'
        'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAJE0DNM8DNM8DAAAA'
        'AAAAAM3zAE8UAU8UAQAAAAAAAMDyPMATPcATRQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
        'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHE0DNM8DNM8DAAAAAAAAAMvzAE8UAc8TAQAAAAAAAEDzPMATRcATRQAAAAAAAAAA'
        'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
        'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
        'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
        'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
        'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
        'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
        'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
        'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
        'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEAAAEOAAABFkKhISsCgDgBAIckQZIgSdA0gGRZ0DRoGkwTIFkWNA2a'
        'BtMEAAAAAAAAAAAAQPI0aBo0DaIIkDQPmgZNgygCAAAAAAAAAAAAIGkaNA2aBlEESJoGTYOmQRQBAAAAAAAAAAAA0EwToghR'
        'hGkCPNOEKEIUYZoAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgAAAgAEHAIAAE8pAoSErAoA4AQCHolgWAAA4kmNZAADgOJJlAQCA'
        'ZVmiCAAAlqWJIgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
        'AACAAACAAQcAgAATykChISsBgCgAAIeiWBZwHMsCjmNZQJIsC2BZAM0DaBpAFAGAAACAAgcAgAAbNCUWByg0ZCUAEAUA4FAU'
        'y9I0UeQ4lqVposiRLEvTRJFlaZrnmSY0zfNMEaLneaYJz/M804RpiqKqAlE0TQEAAAUOAAABNmhKLA5QaMhKACAkAMDhOJbl'
        'eaLoeaJomqrKcSzL80RRFE1TVVWV42iW54miKJqmqqoqy9I0zxNFUTRNVVVdaJrniaIomqaqui48z/NEURRNU1VdF57neaIo'
        'iqapqq4LURRF0zRNVVVV1wWiaJqmqaqq6rpAFEXTNFVVVV0XiKIomqaqqq7rAtM0TVVVVdeVXYBpqqqquq7rAlRVVV3XdWUZ'
        'oKqq6rquK8sA13Vd15VlWQbguq7ryrIsAADgwAEAIMAIOsmosggbTbjwABQasiIAiAIAAIxhSjGlDGMSQgqhYUxCSCFkUlIq'
        'KaUKQiollVJBSKWkUjJKLaWWUgUhlZJKqSCkUlIpBQCAHTgAgB1YCIWGrAQA8gAACGOUYowx5yRCSjHmnHMSIaUYc845qRRj'
        'zjnnnJSSMeecc05K6ZhzzjknpWTMOeeck1I655xzzkkppXTOOeeklFJC6Bx0UkopnXMOQgEAQAUOAAABNopsTjASVGjISgAg'
        'FQDA4DiWpWmeJ4qmaUmSpnme54mmqmqSpGmeJ4qmqao8z/NEURRNU1V5nueJoiiapqpyXVEURdM0TVUly6JoiqapqqoL0zRN'
        '01RV14VpmqZpqqrrwrZVVVVd13Vh26qqqq7rysB1Xdd1ZRnIruu6riwLAABPcAAAKrBhdYSTorHAQkNWAgAZAACEMQgphBBS'
        'yCCkEEJIKYWQAACAAQcAgAATykChISsBgFQAAIAQa6211lprDWPWWmuttdYS56y11lprrbXWWmuttdZaa6211lprrbXWWmut'
        'tdZaa6211lprrbXWWmuttdZaa6211lprrbXWWmuttdZaa6211lprrbXWWmuttdZaa6211lprrbXWWmuttVYAIHaFA8BOhA2r'
        'I5wUjQUWGrISAAgHAACMQYgx6CSUUkqFEGPQSUiltRgrhBiDUEpKrbWYPOcchFJaai3G5DnnIKTUWowxJtdCSCmllmKLsbgW'
        'QioptdZirMkYlVJqLbYYa+3FqJRKSzHGGGswxubUWowx1lqLMTq3EkuMMcZahBHGxRZjrLXXIowRssXSWq21BmOMsbm12GrN'
        'uRgjjK4ttVZrzQUAmDw4AEAl2DjDStJZ4WhwoSErAYDcAAACIaUYY8w555xzDkIIqVKMOecchBBCCKGUUlKlGHPOOQghhFBC'
        'KaWkjDHmHIQQQgillFJKaSllzDkIIYRQSimllNJS65xzEEIIpZRSSiklpdQ55yCEUEoppZRSSkothBBCKKGUUkoppZSUUkoh'
        'hFBKKaWUUkopqaWUQgillFJKKaWUUlJKKYUQQimllFJKKaWklForpZRSSimllFJKSS21lFIopZRSSimllJJaSimlUkoppZRS'
        'SiklpdRSSqWUUkoppZRSSkuppZRKKaWUUkoppZSUUkoppVRKKaWUUkopKaXUWkoppZRKKaWUUlprKaWWUiqllFJKKaW01Fpr'
        'LbWUSimllFJKaa21lFJKKZVSSimllFIAANCBAwBAgBGVFmKnGVcegSMKGSagQkNWAgBkAAAMo5RSSS1FgiKlGKSWQiUVc1BS'
        'iihzDlKsqULOIOYklYoxhJSDVDIHlVLMQQohZUwpBq2VGDrGmKOYaiqhYwwAAABBAACBkAkECqDAQAYAHCAkSAEAhQWGDhEi'
        'QIwCA+Pi0gYAIAiRGSIRsRgkJlQDRcV0ALC4wJAPABkaG2kXF9BlgAu6uOtACEEIQhCLAyggAQcn3PDEG55wgxN0ikodCAAA'
        'AACAAwA8AAAkG0BERDRzHB0eHyAhIiMkJSYnKAIAAAAA4AYAHwAASQoQERHNHEeHxwdIiMgISYnJCUoAACCAAAAAAAAIIAAB'
        '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'
        'YQ5z'
    ),
    'tri': (
        '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'
        'Ukgp5pxyDimlnGPOIaaUc8o55ZyD0kmpnHPSOSmRUso55ZxyzknpnFTOOSmdhAIAAAIcAAACLIRCQ1YEAHECAA7H8TxJ00RR'
        '0jRR9EzRdT3RdF1J00xTE0VV1URRVU1XtW3RVGVb0jTT1ERRVTVRVFVRNW3ZVFXb9kzTlk3X1W1RVXVbtm1heG3b9z3TtG1R'
        'VW3ddF1bd23Z92Vb141H00xTE0VX1URRdU1X1W1TdW1dE0XXFVVXlkXVlWVXlnVflWXd10TRdUXVlF1RdWVblV3fdmVZ903X'
        '9XVVloVflWXht3VdGG7fN55RVXVflV3fV2XZF27dNn7b94Vn0jTT1ETRVTXRVF3TVXXddF3b1kTRdUVXtWXRVF3ZlW3fV13Z'
        '9jVRdF3RVWVZdFVZVmXZ911Z9nVRVX1blWXfV13Z923fF4bZ1n3hdF1dV2XZF1ZZ9n3b15Xl1nXh+EzTtk3X1XXTdX3f9nVn'
        'mXVd+EXX9X1Vln1jtWVf+IXfqfvG8Yyqquuq7Qq/KsvCsAu789y+L5R12/ht3Wfcvo/x4/zGkWvbwjHrtnPcvq4sv/MzfmVY'
        'eqZp26br+rrpur4v67ox3L6vFFXV11VbNobVlYXjFn7j2H3hOEbX9X1Vln1jtWVh2H3feH5heJ7Xto3h9n3KbOtGH3yf8sy6'
        'je37xnL7Oud3js7wDAkAABhwAAAIMKEMFBqyIgCIEwBgEHIOMQUhUgxCCCGlDkJKEWMQMuekZMxJCaWkFkpJLWIMQuaYlMw5'
        'KaGUlkIpLYUSWgulxBZKaa21VmtqLdYQSmuhlBhDKS2m1mpMrdUaMQYhc05K5pyUUkproZTWMueodA5S6iCklFJqsaQUY+Wc'
        'lAw6Kh2ElEoqMZWUYgypxFZSirWkVGNrseUWY86hlBZLKrGVlGJtMeUYY8w5YgxC5pyUzDkpoZTWSkktVs5J6SCklDkoqaQU'
        'Yykpxcw5SR2ElDroKJWUYkwtxRZKia2kVGMpqcUWY84txVhDSS2WlGItKcXYYsy5xZZbB6G1kEqMoZQYW4w5t9ZqDaXEWFKK'
        'taRUY4y19hhjzqGUGEsqNZaUYm019tpirDm1lmtqseYWY8+15dZrzr2n1mpNseXaYsw95hhkzbkHD0JroZQWQykxttZqbTHm'
        'HEqJraRUYykp1hhjzi3W2kMpMZaUYi0p1RpjzDnW2GtqLdcWY8+pxZprzsHHmGNPLdYcY8w9xZZrzbn3mluQBQAADDgAAASY'
        'UAYKDVkJAEQBABCEKMUYhAYhxpyT0CDEmHNSKsacg5BKxZhzEErKnINQSkqZcxBKSSmUkkpKrYVSSkqptQIAAAocAAACbNCU'
        'WByg0JCVAEAqAIDBcSzL80RRNWXZsSTPE0XTVFXbdizL80TRNFXVti3PE0XTVFXX1XXL80TRVFXVdXXdE0XVVFXXlWXf90TR'
        'NFXVdWXZ903TdFXXlWXb9n3TNFXXdWVZtn1hdVXXlWXb1m1jWFXXdWXZtm1dOW7d1nXhF4ZhmNq67vu+LwzH8EwDAMATHACA'
        'CmxYHeGkaCyw0JCVAEAGAABhDEIGIYUMQkghhZRCSCklAABgwAEAIMCEMlBoyEoAIBUAACDEWmuttdZaYqm11lprrbWGSmut'
        'tdZaa6211lprrbXWWmuttdZaa6211lprrbXWWmuttZRSSimllFJKKaWUUkoppZRSSimllFJKKaWUUkoppZRSSimllFJKKaWU'
        'UkoppZRSSimllFJKKRUA6FfhAOD/YMPqCCdFY4GFhqwEAMIBAABjlGIMOukkpNQw5RiEUlJJpZVGMecglJJSSq1VzklIpaXW'
        'Wouxck5KSSm1FluMHYSUWmotxhhj7CCklFprMcYYYyilpRhjrDHWWkNJqbUYY4w111pSai3GWmutufeSUosxxlxr7rmX1mKs'
        'teacc849tRZjrTXn3HPwqbUYY8619957UK3FWGuuOQfhewEA3A0OABAJNs6wknRWOBpcaMhKACAkAIBAiDHGnHMOQgghREox'
        '5pxzEEIIIYRIKcaccw5CCCGEkDHmnHMQQgihlFIyxpxzDkIIJZRQSuaccxBCCKGUUkrJnHMOQgghlFJKKR10EEIIoZRSSiml'
        'cw5CCKGUUkoppYQQQiillFJKKaWUEEIIpZRSSimllBJCCKWUUkoppZRSQgihlFJKKaWkUkoIoZRSSimllFJKCSGUUkoppZRS'
        'SimhhFJKKaWUUkopJZRQSimllFJKKqUUAABw4AAAEGAEnWRUWYSNJlx4AAoNWQkAAAEAIM5abClGRjHnIIbIIMQghgopxZy1'
        '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'
    ),
}


if want("ogg"):
    import base64
    raw = {k: base64.b64decode("".join(v)) for k, v in OGG_B64.items()}
    write(D, "OGG/PLAIN.OGG", raw["mono"])
    write(D, "OGG/LOOPLEN.OGG", ogg_with_comments(raw["stereo"], ["LOOPSTART=2000", "LOOPLENGTH=9000", "TITLE=Stage 1"]))
    write(D, "OGG/LOOPEND.OGG", ogg_with_comments(raw["mono"], ["LOOP_START=500", "LOOP_END=7000"]))
    write(D, "OGG/LPTAG.OGG", ogg_with_comments(raw["mono"], ["lp=100,9000"]))
    write(D, "OGG/KAMAI.OGG", ogg_with_comments(raw["stereo"], ["L=4410"]))
    write(D, "OGG/TRI.OGG", ogg_with_comments(raw["tri"], ["COMMENT=loop(10,8000)"]))
    # inside an .acx [12Riven]
    o = ogg_with_comments(raw["mono"], ["LOOPSTART=300"])
    a = adx_file([tone(32000, 0.2, [700])], 32000)
    table = struct.pack(">II", 0x40, len(o)) + struct.pack(">II", 0x40 + len(o) + 0x10, len(a))
    body = bytearray((u32be(0) + u32be(2) + table).ljust(0x40, b"\0")) + o + bytes(0x10) + a
    write(D, "ACX/RIVEN.ACX", bytes(body))

print("major fixtures in", D)
