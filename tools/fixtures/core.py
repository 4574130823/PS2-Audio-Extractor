"""Test files for the core formats: VAG, ADS, HD/BD, ADX, WAV (with and without loops)."""
import struct

from common import *  # noqa: F401,F403

D = out_dir("core")


def vagp(samples, rate, name, loop=None):
    data = psx_encode(samples, loop=loop)
    return b"VAGp" + struct.pack(">IIII", 0x20, 0, len(data), rate) + bytes(12) + name.encode().ljust(16, b"\0")[:16] + data


def vag_stereo_paired(left, right, rate, il=0x800):
    l, r = psx_encode(left), psx_encode(right)
    size = max(len(l), len(r))
    first = il - 0x30
    hdr = b"VAGp" + struct.pack(">IIII", 0x20, 0, size, rate) + bytes(12) + b"STEREO".ljust(16, b"\0")

    def blocks(d):
        d = d.ljust(size, b"\0")
        return [d[:first]] + [d[first:][i:i + il] for i in range(0, len(d) - first, il)]
    bl, br = blocks(l), blocks(r)
    buf = bytearray()
    for i in range(max(len(bl), len(br))):
        w = first if i == 0 else il
        for side in (bl, br):
            buf += (hdr if i == 0 else b"") + (side[i] if i < len(side) else b"").ljust(w, b"\0")
    return bytes(buf)


def ads(chans, rate, psx=True, il=0x800):
    if psx:
        data, codec = interleave([psx_encode(c) for c in chans], il), 0x10
    else:
        il = 0x400
        data, codec = interleave([pcm16le(c) for c in chans], il), 0x01
    return b"SShd" + struct.pack("<IIIII", 0x18, codec, rate, len(chans), il) + struct.pack("<ii", -1, -1) + b"SSbd" + u32le(len(data)) + data


def hd_bd(samples):
    bd = bytearray()
    offsets = []
    for s, _, loop in samples:
        offsets.append(len(bd))
        bd += psx_encode(s)
    n = len(samples)
    table_len = (0x10 + 4 * (n + 1) + 15) // 16 * 16
    infos, rel = bytearray(), []
    for (s, rate, loop), off in zip(samples, offsets):
        rel.append(table_len + len(infos))
        infos += struct.pack("<IHBB", off, rate, 1 if loop else 0, 0xFF)
    vagi = bytearray(b"IECSigaV") + struct.pack("<II", table_len + len(infos), n - 1)
    vagi += b"".join(u32le(r) for r in rel) + u32le(0)
    vagi = vagi.ljust(table_len, b"\xff") + infos
    head_size = 0x40
    vagi_off = 0x10 + head_size
    hd_size = vagi_off + len(vagi)
    head = (b"IECSdaeH" + struct.pack("<IIII", head_size, hd_size, len(bd), 0xFFFFFFFF)
            + struct.pack("<III", 0xFFFFFFFF, 0xFFFFFFFF, vagi_off) + u32le(0xFFFFFFFF)).ljust(head_size, b"\xff")
    return b"IECSsreV" + struct.pack("<II", 0x10, 0x01010000) + head + bytes(vagi), bytes(bd)


def wav_loop(samples, rate, start, end):
    data = pcm16le(samples)
    smpl = b"smpl" + u32le(60) + struct.pack("<9I", 0, 0, 1000000000 // rate, 60, 0, 0, 0, 1, 0) + struct.pack("<6I", 0, 0, start, end - 1, 0, 0)
    body = b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 1, 1, rate, rate * 2, 2, 16) + smpl + b"data" + u32le(len(data)) + data
    return b"RIFF" + u32le(len(body)) + body


write(D, "BGM/TITLE.VAG", vagp(tone(22050, 2.5, [440, 660]), 22050, "TITLE"))
write(D, "BGM/LOOP.VAG", vagp(sweep(22050, 2.0, 300, 900), 22050, "LOOP", loop=(20, 1500)))
write(D, "BGM/STEREO.VAG", vag_stereo_paired(tone(32000, 1.5, [330]), tone(32000, 1.5, [495]), 32000))
write(D, "STREAM/MUSIC.ADS", ads([sweep(48000, 3.0), sweep(48000, 3.0, 4000, 200)], 48000))
write(D, "STREAM/VOICE.ADS", ads([tone(24000, 1.0, [300, 900])], 24000, psx=False))
hd, bd = hd_bd([(tone(22050, 0.4, [880]), 22050, False), (tone(11025, 0.6, [220, 330]), 11025, True),
                (sweep(44100, 0.5, 100, 8000), 44100, False)])
write(D, "SOUND/SE.HD", hd)
write(D, "SOUND/SE.BD", bd)
write(D, "BGM/JINGLE.ADX", adx([tone(44100, 2.0, [523, 659, 784]), tone(44100, 2.0, [392, 494])], 44100))
write(D, "BGM/LOOPED.ADX", adx([sweep(32000, 2.0, 200, 1200)], 32000, loop=(8000, 60000)))
write(D, "SE/CLIP.WAV", wav(tone(22050, 0.4, [350]), 22050))
write(D, "SE/LOOPCLIP.WAV", wav_loop(tone(22050, 0.8, [500]), 22050, 2000, 15000))
print("core fixtures in", D)
