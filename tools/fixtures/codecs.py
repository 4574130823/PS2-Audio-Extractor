"""Test files for SSND, AUS, MCSS, XAU, STMA, SFX0, WMW, MPC3, ADP (Ongakukan), RSD,
STR+WAV, MUL, SMP, WD, RWS, AUDIOPKG; and the IMA/AICA/MPC3/Ongakukan codecs they use.

The encoders here are greedy (each code picked by simulating the decoder): the output only
has to be valid data vgmstream decodes the same way we do.
"""
import struct

from common import *  # noqa: F401,F403

D = out_dir("codecs")

# ---------------------------------------------------------------- IMA family
IMA_STEPS = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143,
    157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411,
    1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767]
IMA_INDEX = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8]


def clamp16(v):
    return max(-32768, min(32767, v))


def ima_expand(code, hist, idx):
    step = IMA_STEPS[idx]
    d = step >> 3
    if code & 1: d += step >> 2
    if code & 2: d += step >> 1
    if code & 4: d += step
    if code & 8: d = -d
    return clamp16(hist + d), max(0, min(88, idx + IMA_INDEX[code]))


def ima_greedy(target, hist, idx, expand=ima_expand):
    best = None
    for code in range(16):
        h, i = expand(code, hist, idx)
        e = abs(h - target)
        if best is None or e < best[0]:
            best = (e, code, h, i)
    return best[1], best[2], best[3]


def ima_codes(samples, expand=ima_expand):
    """Headerless IMA codes for one channel."""
    hist, idx, codes = 0, 0, []
    for x in samples:
        c, hist, idx = ima_greedy(x, hist, idx, expand)
        codes.append(c)
    return codes


def pack_nibbles(codes, high_first):
    if len(codes) % 2:
        codes = codes + [0]
    out = bytearray()
    for a, b in zip(codes[0::2], codes[1::2]):
        out.append((a << 4 | b) if high_first else (b << 4 | a))
    return bytes(out)


def ima_mono(samples, high_first):
    return pack_nibbles(ima_codes(samples), high_first)


def xbox_frames(samples):
    """Xbox IMA: list of (header 4 bytes, 32 data bytes) frames of 64 samples."""
    frames, idx = [], 0
    for k in range(0, len(samples), 64):
        fr = samples[k:k + 64]
        fr = fr + [0] * (64 - len(fr))
        hist = fr[0]
        hdr = struct.pack("<hBB", hist, idx, 0)
        codes = []
        for x in fr[1:]:
            c, hist, idx = ima_greedy(x, hist, idx)
            codes.append(c)
        codes.append(0)
        frames.append((hdr, pack_nibbles(codes, False)))
    return frames


def xbox_mono(samples):
    return b"".join(h + d for h, d in xbox_frames(samples))


def xbox_stereo(left, right):
    out = bytearray()
    for (hl, dl), (hr, dr) in zip(xbox_frames(left), xbox_frames(right)):
        out += hl + hr
        for k in range(0, 32, 4):
            out += dl[k:k + 4] + dr[k:k + 4]
    return bytes(out)


def interleave_last(chans, il):
    """Blocks of `il` per channel; the last row splits what's left evenly (vgmstream's
    interleave_last)."""
    size = max(len(c) for c in chans)
    chans = [c.ljust(size, b"\0") for c in chans]
    if il == 0:
        return b"".join(chans)
    out = bytearray()
    for i in range(0, size, il):
        for c in chans:
            out += c[i:i + il]
    return bytes(out)


# ---------------------------------------------------------------- SSND
def ssnd(chans, rate, codec, il):
    n = len(chans[0])
    if codec == 1:
        data = interleave_last([ima_mono(c, True) for c in chans], il)
    else:
        data = interleave_last([pcm16le(c) for c in chans], il)
    hdr = b"SSND" + u32le(0x18) + struct.pack("<HHHIII", codec, len(chans), 16, rate, il, n)
    return hdr.ljust(0x20, b"\0") + data


write(D, "SSND/MUSIC.SND", ssnd([sweep(22050, 0.5), tone(22050, 0.5, [300])], 22050, 1, 0x400))
write(D, "SSND/VOICE.SND", ssnd([tone(16000, 0.4, [500, 700])], 16000, 1, 0))
write(D, "SSND/PCM.SND", ssnd([tone(11025, 0.3, [200]), tone(11025, 0.3, [250])], 11025, 0, 0x200))


# ---------------------------------------------------------------- AUS
def aus(chans, rate, xbox=False, loop=None):
    if xbox:
        data = xbox_stereo(*chans) if len(chans) == 2 else xbox_mono(chans[0])
        n = len(data) // (0x24 * len(chans)) * 64
    else:
        enc = [psx_encode(c) for c in chans]
        n = len(enc[0]) // 16 * 28
        data = interleave(enc, 0x800) if len(chans) > 1 else enc[0]
    ls, le = loop or (0, 0)
    hdr = b"AUS " + struct.pack("<HHIHHIiiI", 0, 2 if xbox else 0, n, len(chans), 0, rate, ls, le, 1 if loop else 0)
    return hdr.ljust(0x800, b"\0") + data


write(D, "AUS/STAGE.AUS", aus([sweep(22050, 0.4), sweep(22050, 0.4, 3000, 300)], 22050, loop=(1000, 8000)))
write(D, "AUS/JINGLE.AUS", aus([tone(24000, 0.3, [600])], 24000))
write(D, "AUS/XBOX.AUS", aus([tone(22050, 0.3, [440]), tone(22050, 0.3, [660])], 22050, xbox=True))


# ---------------------------------------------------------------- MCSS
def mcss(chans, rate, xbox=False):
    if xbox:
        pairs = [xbox_stereo(chans[i], chans[i + 1]) for i in range(0, len(chans), 2)]
        data = interleave(pairs, 0x4800)
        il, chan_size = 0x4800, 0
    else:
        enc = [psx_encode(c) for c in chans]
        il, chan_size = 0x800, len(enc[0])
        data = interleave(enc, il) if len(chans) > 1 else enc[0]
    hdr = b"MCSS" + struct.pack("<IIIIBBHII", 0x100, 0x40, len(data), rate, len(chans) // 2, 0, len(chans), il, chan_size)
    hdr += b"Guerrilla MSS"
    return hdr.ljust(0x40, b"\0") + data


write(D, "MCSS/KZ_MUSIC.MSS", mcss([sweep(32000, 0.4, 100, 2000), tone(32000, 0.4, [400])], 32000))
write(D, "MCSS/KZ_VOICE.MSS", mcss([tone(22050, 0.3, [350, 900])], 22050))
write(D, "MCSS/XB_AMB.MSS", mcss([tone(22050, 0.3, [f]) for f in (200, 300, 400, 500)], 22050, xbox=True))


# ---------------------------------------------------------------- XAU
def xau(chans, rate, xbox=False, loop=(0, 0)):
    ch = len(chans)
    head = b"XAU\0" + struct.pack("<II", 0x100, 0x40) + (b"XB\0\0" if xbox else b"PS2\0") + struct.pack("<ii", *loop)
    head = (head + bytes([ch])).ljust(0x40, b"\0")
    if xbox:
        data = xbox_stereo(*chans) if ch == 2 else xbox_mono(chans[0])
        fmt = b"fmt " + struct.pack("<IHHIIHH", 0x14, 0x69, ch, rate, rate * ch, 0x24 * ch, 4) + struct.pack("<HH", 2, 64)
        smpl = b"smpl" + u32le(0x10) + bytes(0x10)
        body = b"WAVE" + fmt + smpl + b"data" + u32le(len(data)) + data
        return head + b"RIFF" + u32le(len(body) + 0x100) + body  # sometimes wrong RIFF size
    enc = [psx_encode(c) for c in chans]
    per_ch = len(enc[0])
    vag = b"VAGp" + struct.pack(">IIII", 0x20, 0, per_ch, rate)
    data = interleave(enc, 0x8000) if ch > 1 else enc[0]
    return (head + vag).ljust(0x800, b"\0") + data


write(D, "XAU/BGM01.XAU", xau([sweep(44100, 0.5), sweep(44100, 0.5, 5000, 100)], 44100, loop=(2000, 20000)))
write(D, "XAU/SE01.XAU", xau([tone(22050, 0.2, [1000])], 22050))
write(D, "XAU/XB01.XAU", xau([tone(22050, 0.3, [330]), tone(22050, 0.3, [440])], 22050, xbox=True, loop=(100, 5000)))


# ---------------------------------------------------------------- STMA
def stma(chans, rate, bps=4, il_field=0x8000, loop=None, big=False):
    ch = len(chans)
    if bps == 4:
        il = 0x80 if il_field == 0xc000 else 0x40
        enc = [ima_mono(c, True) for c in chans]
        size = max(len(e) for e in enc)
        size = (size + il - 1) // il * il
        data = interleave([e.ljust(size, b"\0") for e in enc], il) if ch > 1 else enc[0]
    else:
        data = b"".join((pcm16be if big else pcm16le)([c[i] for c in chans]) for i in range(len(chans[0])))
    p = ">" if big else "<"
    loop_end = 0x800 + (loop[1] if loop else len(data))
    hdr = (b"AMTS" if big else b"STMA") + struct.pack(p + "IIIIIII", 0x696F, il_field, rate, bps, ch, len(data), loop_end)
    if loop:
        hdr += struct.pack(p + "II", 1, loop[0])
    else:
        hdr += b"\xcc" * 8
    return hdr.ljust(0x800, b"\0") + data


write(D, "STMA/RDR_MUS.STM", stma([sweep(32000, 0.4), tone(32000, 0.4, [440])], 32000, loop=(1000, 0x1800)))
write(D, "STMA/RDR_VOX.STM", stma([tone(22050, 0.3, [300])], 22050, il_field=0xc000))
write(D, "STMA/RDR_WIDE.STM", stma([tone(22050, 0.3, [300]), tone(22050, 0.3, [310])], 22050, il_field=0xc000))
write(D, "STMA/SH2_PCM.STM", stma([tone(22050, 0.2, [500]), tone(22050, 0.2, [700])], 22050, bps=16))
write(D, "STMA/GC_PCM.STM", stma([tone(22050, 0.2, [500])], 22050, bps=16, big=True))


# ---------------------------------------------------------------- SFX0 (Monster Games)
def sfx0(samples, rate, codec, loop=False):
    if codec == 0xCFFF:
        data, c1, c2 = psx_encode(samples), 0x00040002, 0
    elif codec == 0x69:
        data, c1, c2 = xbox_mono(samples), 0x00040024, 0x00400002
    elif codec == 1:
        data, c1, c2 = pcm16le(samples), 0x00100002, 0
    else:  # .sf0: PCM big endian with a short header
        data = pcm16be(samples)
        return u32le(len(data)) + u32le(0x20) + struct.pack("<BBHHHiIII", 0, 0, 0, 0, 1, rate, rate * 2, 0x00100000, 0) + data
    head = u32le(len(data)) + u32le(0x30) + struct.pack("<BBHHHiIII", 1 if loop else 0, 0, 0, codec, 1, rate, rate * 2, c1, c2)
    return head.ljust(0x30, b"\0") + data


def sfx0_early(samples, rate, loop):
    data = psx_encode(samples, lead=False, loop=(0, len(samples) // 28 - 1) if loop else None)
    return u32le(len(data)) + struct.pack("<HHiIIH", 0xCFFF, 1, rate, rate, 0x00040002, 0x6164) + data


write(D, "SFX0/ENGINE.SFX", sfx0(tone(22050, 0.3, [120, 240]), 22050, 0xCFFF, loop=True))
write(D, "SFX0/CRASH.SFX", sfx0(sweep(22050, 0.3, 2000, 100), 22050, 0xCFFF))
write(D, "SFX0/XB_HORN.SFX", sfx0(tone(22050, 0.2, [400]), 22050, 0x69, loop=True))
write(D, "SFX0/GC_TIRE.SFX", sfx0(tone(32000, 0.2, [800]), 32000, 1))
write(D, "SFX0/MINI.SF0", sfx0(tone(32000, 0.1, [900]), 32000, 2))
write(D, "SFX0/OLD_IDLE.SFX", sfx0_early(tone(22050, 0.3, [100]), 22050, True))
write(D, "SFX0/OLD_HIT.SFX", sfx0_early(tone(22050, 0.2, [700]), 22050, False))


# ---------------------------------------------------------------- AICA / WMW
AICA_SCALE = [230, 230, 230, 230, 307, 409, 512, 614, 230, 230, 230, 230, 307, 409, 512, 614]


def cdiv(a, b):
    """C integer division (truncates toward zero)."""
    q = abs(a) // abs(b)
    return q if (a >= 0) == (b >= 0) else -q


def aica_expand(code, hist, step):
    hist = cdiv(hist * 254, 256)
    d = (((code & 7) * 2 + 1) * step) >> 3
    d = min(d, 32767)
    if code & 8:
        d = -d
    s = clamp16(hist + d)
    step = max(0x7f, min(0x6000, (step * AICA_SCALE[code]) >> 8))
    return s, step


def aica_codes(samples):
    hist, step, codes = 0, 0x7f, []
    for x in samples:
        best = min(range(16), key=lambda c: abs(aica_expand(c, hist, step)[0] - x))
        hist, step = aica_expand(best, hist, step)
        codes.append(best)
    return codes


def wmw(chans, rate, loop=None):
    codes = [aica_codes(c) for c in chans]
    if len(chans) == 2:
        data = bytes((a << 4) | b for a, b in zip(*codes))
    else:
        data = pack_nibbles(codes[0], True)
    ls, le = loop or (0, 0)
    hdr = b"WMW " + bytes([2, 1 if loop else 0, 4, len(chans)]) + struct.pack("<iIIII", rate, 0x40, len(data), ls, le)
    hdr = hdr.ljust(0x28, b"\0") + struct.pack("<II", 0x7f, 0x7f)
    return hdr.ljust(0x40, b"\0") + data


write(D, "WMW/GV_BGM.WMW", wmw([sweep(32000, 0.4), tone(32000, 0.4, [523])], 32000, loop=(0x100, 0x2000)))
write(D, "WMW/GV_SE.WMW", wmw([tone(22050, 0.3, [300, 1200])], 22050))


# ---------------------------------------------------------------- MPC3
MPC3_STEPS = [[[2, 2, 3, 7, 15, 27, 45, 70, 104, 148, 202, 268, 347, 441, 551, 677, 821, 984, 1168, 1374, 1602, 1854, 2131, 2435, 2767, 3127, 3517, 3938, 4392, 4880, 5402, 5960, 6555, 7189, 7862, 8577, 9333, 10132, 10976, 11865, 12802, 13786, 14819, 15903, 17038, 18226, 19469, 20766, 22120, 23531, 25001, 26531, 28123, 29776, 31494, 33276, 35124, 37039, 39023, 41076, 43201, 45397, 47666, 50010], [1, 1, 2, 5, 10, 18, 31, 48, 72, 101, 139, 184, 239, 303, 378, 465, 564, 677, 803, 944, 1101, 1274, 1465, 1674, 1902, 2150, 2418, 2707, 3019, 3355, 3714, 4097, 4507, 4942, 5405, 5896, 6416, 6966, 7546, 8157, 8801, 9477, 10188, 10933, 11714, 12530, 13384, 14276, 15207, 16177, 17188, 18240, 19334, 20471, 21652, 22877, 24148, 25464, 26828, 28240, 29700, 31210, 32770, 34382], [0, 0, 1, 2, 4, 8, 14, 22, 32, 46, 63, 83, 108, 138, 172, 211, 256, 307, 365, 429, 500, 579, 666, 761, 864, 977, 1099, 1230, 1372, 1525, 1688, 1862, 2048, 2246, 2457, 2680, 2916, 3166, 3430, 3708, 4000, 4308, 4631, 4969, 5324, 5695, 6084, 6489, 6912, 7353, 7813, 8291, 8788, 9305, 9841, 10398, 10976, 11574, 12194, 12836, 13500, 14186, 14895, 15628], [0, 0, 0, 0, 1, 3, 5, 8, 13, 18, 25, 33, 43, 55, 68, 84, 102, 123, 146, 171, 200, 231, 266, 304, 345, 390, 439, 492, 549, 610, 675, 745, 819, 898, 982, 1072, 1166, 1266, 1372, 1483, 1600, 1723, 1852, 1987, 2129, 2278, 2433, 2595, 2765, 2941, 3125, 3316, 3515, 3722, 3936, 4159, 4390, 4629, 4877, 5134, 5400, 5674, 5958, 6251]], [[1, 1, 2, 4, 9, 17, 28, 44, 65, 92, 126, 167, 217, 276, 344, 423, 513, 615, 730, 858, 1001, 1159, 1332, 1522, 1729, 1954, 2198, 2461, 2745, 3050, 3376, 3725, 4097, 4493, 4914, 5360, 5833, 6332, 6860, 7416, 8001, 8616, 9262, 9939, 10649, 11391, 12168, 12978, 13825, 14707, 15626, 16582, 17576, 18610, 19683, 20797, 21952, 23149, 24389, 25673, 27000, 28373, 29791, 31256], [0, 0, 1, 2, 5, 10, 17, 26, 39, 55, 75, 100, 130, 165, 206, 254, 308, 369, 438, 515, 600, 695, 799, 913, 1037, 1172, 1319, 1477, 1647, 1830, 2025, 2235, 2458, 2696, 2948, 3216, 3499, 3799, 4116, 4449, 4800, 5169, 5557, 5963, 6389, 6835, 7300, 7787, 8295, 8824, 9375, 9949, 10546, 11166, 11810, 12478, 13171, 13889, 14633, 15403, 16200, 17023, 17874, 18753], [0, 0, 0, 1, 3, 6, 11, 17, 26, 37, 50, 67, 86, 110, 137, 169, 205, 246, 292, 343, 400, 463, 532, 608, 691, 781, 879, 984, 1098, 1220, 1350, 1490, 1638, 1797, 1965, 2144, 2333, 2533, 2744, 2966, 3200, 3446, 3704, 3975, 4259, 4556, 4867, 5191, 5530, 5882, 6250, 6632, 7030, 7444, 7873, 8319, 8781, 9259, 9755, 10269, 10800, 11349, 11916, 12502], [0, 0, 0, 0, 0, 1, 2, 4, 6, 9, 12, 16, 21, 27, 34, 42, 51, 61, 73, 85, 100, 115, 133, 152, 172, 195, 219, 246, 274, 305, 337, 372, 409, 449, 491, 536, 583, 633, 686, 741, 800, 861, 926, 993, 1064, 1139, 1216, 1297, 1382, 1470, 1562, 1658, 1757, 1861, 1968, 2079, 2195, 2314, 2438, 2567, 2700, 2837, 2979, 3125]], [[1, 1, 2, 5, 10, 18, 31, 48, 72, 101, 139, 184, 239, 303, 378, 465, 564, 677, 803, 944, 1101, 1274, 1465, 1674, 1902, 2150, 2418, 2707, 3019, 3355, 3714, 4097, 4507, 4942, 5405, 5896, 6416, 6966, 7546, 8157, 8801, 9477, 10188, 10933, 11714, 12530, 13384, 14276, 15207, 16177, 17188, 18240, 19334, 20471, 21652, 22877, 24148, 25464, 26828, 28240, 29700, 31210, 32770, 34382], [1, 1, 1, 3, 7, 13, 22, 35, 52, 74, 101, 134, 173, 220, 275, 338, 410, 492, 584, 687, 801, 927, 1065, 1217, 1383, 1563, 1758, 1969, 2196, 2440, 2701, 2980, 3277, 3594, 3931, 4288, 4666, 5066, 5488, 5932, 6401, 6893, 7409, 7951, 8519, 9113, 9734, 10383, 11060, 11765, 12500, 13265, 14061, 14888, 15747, 16638, 17562, 18519, 19511, 20538, 21600, 22698, 23833, 25005], [0, 0, 1, 2, 4, 8, 14, 22, 32, 46, 63, 83, 108, 138, 172, 211, 256, 307, 365, 429, 500, 579, 666, 761, 864, 977, 1099, 1230, 1372, 1525, 1688, 1862, 2048, 2246, 2457, 2680, 2916, 3166, 3430, 3708, 4000, 4308, 4631, 4969, 5324, 5695, 6084, 6489, 6912, 7353, 7813, 8291, 8788, 9305, 9841, 10398, 10976, 11574, 12194, 12836, 13500, 14186, 14895, 15628], [0, 0, 0, 1, 2, 5, 8, 13, 19, 27, 37, 50, 65, 82, 103, 127, 154, 184, 219, 257, 300, 347, 399, 456, 518, 586, 659, 738, 823, 915, 1012, 1117, 1229, 1348, 1474, 1608, 1749, 1899, 2058, 2224, 2400, 2584, 2778, 2981, 3194, 3417, 3650, 3893, 4147, 4412, 4687, 4974, 5273, 5583, 5905, 6239, 6585, 6944, 7316, 7701, 8100, 8511, 8937, 9376]], [[1, 1, 2, 5, 11, 20, 34, 53, 78, 111, 151, 201, 260, 331, 413, 508, 616, 738, 876, 1030, 1201, 1390, 1598, 1826, 2075, 2345, 2638, 2954, 3294, 3660, 4051, 4470, 4916, 5392, 5897, 6432, 6999, 7599, 8232, 8899, 9601, 10339, 11114, 11927, 12779, 13670, 14601, 15574, 16590, 17648, 18751, 19898, 21092, 22332, 23620, 24957, 26343, 27779, 29267, 30807, 32400, 34047, 35749, 37507], [1, 1, 1, 3, 6, 11, 19, 31, 45, 64, 88, 117, 152, 193, 241, 296, 359, 430, 511, 601, 701, 811, 932, 1065, 1210, 1368, 1538, 1723, 1921, 2135, 2363, 2607, 2868, 3145, 3440, 3752, 4083, 4433, 4802, 5191, 5600, 6031, 6483, 6957, 7454, 7974, 8517, 9085, 9677, 10295, 10938, 11607, 12303, 13027, 13778, 14558, 15366, 16204, 17072, 17971, 18900, 19861, 20854, 21879], [0, 0, 0, 1, 2, 5, 8, 13, 19, 27, 37, 50, 65, 82, 103, 127, 154, 184, 219, 257, 300, 347, 399, 456, 518, 586, 659, 738, 823, 915, 1012, 1117, 1229, 1348, 1474, 1608, 1749, 1899, 2058, 2224, 2400, 2584, 2778, 2981, 3194, 3417, 3650, 3893, 4147, 4412, 4687, 4974, 5273, 5583, 5905, 6239, 6585, 6944, 7316, 7701, 8100, 8511, 8937, 9376], [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]]]
MPC3_INDEX = [5, 1, -1, -3]


def mpc3_step(code, mode, hist, idx):
    index, sign = code & 3, code & 4
    diff = MPC3_STEPS[mode][index][idx]
    hist = hist + diff if sign else hist - 1 - diff
    return hist, max(0, min(63, idx + MPC3_INDEX[index]))


def mpc3_subblock(target, hist, idx):
    best = None
    for mode in range(4):
        h, i, err, codes = hist, idx, 0, []
        for x in target:
            c = min(range(8), key=lambda c: abs(mpc3_step(c, mode, h, i)[0] - x))
            h, i = mpc3_step(c, mode, h, i)
            err += (h - x) ** 2
            codes.append(c)
        if best is None or err < best[0]:
            best = (err, mode, codes, h, i)
    _, mode, codes, h, i = best
    word = mode << 30
    for k, c in enumerate(codes):
        word |= c << (3 * k)
    return word, h, i


def mpc3(chans, rate, subblocks):
    """`subblocks`: 10-sample sub-blocks per channel per block."""
    ch = len(chans)
    n = len(chans[0]) // 10
    blocks = bytearray()
    state = [(0, 0)] * ch
    per_block = subblocks * 10
    for b0 in range(0, n * 10, per_block):
        header = 0
        words = [[] for _ in range(ch)]
        for c in range(ch):
            h, i = state[c]
            h = max(-32768, min(32767, h)) & ~0x3f
            header |= ((h & 0xffc0) | i) << (16 * c)
            for s in range(b0, b0 + per_block, 10):
                tgt = (chans[c][s:s + 10] + [0] * 10)[:10]
                w, h, i = mpc3_subblock(tgt, h, i)
                words[c].append(w)
            state[c] = (h, i)
        blocks += u32le(header)
        for k in range(subblocks):
            for c in range(ch):
                blocks += u32le(words[c][k])
    hdr = b"MPC3" + u32be(0x00011400) + struct.pack("<IiiII", ch, rate, n, subblocks, len(blocks))
    return hdr + bytes(blocks)


write(D, "MPC3/SPY_THEME.MC3", mpc3([tone(22050, 0.3, [440])[:6600], tone(22050, 0.3, [660])[:6600]], 22050, 330))
write(D, "MPC3/T3_VOICE.MC3", mpc3([sweep(16000, 0.3, 200, 3000)[:4800]], 16000, 480))


# ---------------------------------------------------------------- Ongakukan ADP
ONGAKUKAN_FILTER = [233, 549, 453, 375, 310, 233, 233, 233, 233, 233, 233, 233, 310, 375, 453, 549]


def wrap16(v):
    return (v + 0x8000) % 0x10000 - 0x8000


def ongakukan_byte(hist, scale, hi, lo):
    s0 = wrap16(hist + (hi - 8) * scale)
    scale = (scale * ONGAKUKAN_FILTER[hi]) >> 8
    s1 = wrap16(s0 + (lo - 8) * scale)
    scale = (scale * ONGAKUKAN_FILTER[lo]) >> 8
    return s0, s1, scale


def ongakukan_encode(samples):
    hist, scale, out = 0, 0x10, bytearray()
    for k in range(0, len(samples) - 1, 2):
        a, b = samples[k], samples[k + 1]
        best = None
        for byte in range(256):
            s0, s1, sc = ongakukan_byte(hist, scale, byte >> 4, byte & 15)
            if sc < 4 or sc > 20000:
                continue
            e = (s0 - a) ** 2 + (s1 - b) ** 2
            if best is None or e < best[0]:
                best = (e, byte, s1, sc)
        _, byte, hist, scale = best
        out.append(byte)
    return bytes(out)


def adp_ongakukan(samples, rate, fmt_size=0x10, diff=2):
    data = ongakukan_encode(samples)
    pcm = len(data) * 4
    fmt = struct.pack("<HHIIHH", 1, 1, rate, rate * 2, 2, 16) + bytes(fmt_size - 0x10)
    head = b"RIFF" + u32le(0x24 + pcm + diff) + b"WAVE" + b"fmt " + u32le(fmt_size) + fmt
    if fmt_size == 0x10:
        head += b"data" + u32le(pcm)
    else:
        head += b"fact" + u32le(4) + u32le(len(samples))
    return head[:0x2c].ljust(0x2c, b"\0") + data


write(D, "ADP/MIDO_ANN.ADP", adp_ongakukan(tone(22050, 0.3, [300, 450]), 22050))
write(D, "ADP/MIDO_BELL.ADP", adp_ongakukan(sweep(24000, 0.2, 500, 3000), 24000, fmt_size=0x12, diff=0))


# ---------------------------------------------------------------- RSD (Radical)
def rad_frames(chans):
    """Radical IMA: 0x14*ch frames (per-channel step+hist header, then nibbles by channel)."""
    ch = len(chans)
    state = [(0, 0)] * ch
    out = bytearray()
    for k in range(0, len(chans[0]), 32):
        heads, codes = bytearray(), []
        for c in range(ch):
            hist, idx = state[c]
            heads += struct.pack("<hh", idx, hist)
            cc = []
            for x in (chans[c][k:k + 32] + [0] * 32)[:32]:
                code, hist, idx = ima_greedy(x, hist, idx)
                cc.append(code)
            codes.append(cc)
            state[c] = (hist, idx)
        body = bytearray()
        for i in range(0, 32, 2):
            for c in range(ch):
                body.append(codes[c][i] | codes[c][i + 1] << 4)
        out += heads + body
    return bytes(out)


def rsd(version, codec, chans, rate, il=0, name=None):
    ch = len(chans)
    if codec == b"VAG ":
        enc = [psx_encode(c) for c in chans]
        data = interleave(enc, il or 0x10) if ch > 1 else enc[0]
    elif codec == b"XADP":
        data = xbox_stereo(*chans) if ch == 2 else xbox_mono(chans[0])
    elif codec == b"RADP":
        data = rad_frames(chans)
    elif codec == b"PCM ":
        data = b"".join(pcm16le([c[i] for c in chans]) for i in range(len(chans[0])))
    else:
        data = b"".join(pcm16be([c[i] for c in chans]) for i in range(len(chans[0])))
    hdr = b"RSD" + version + codec + struct.pack("<iii", ch, 16, rate)
    if version in b"23":
        start = 0x80
        hdr += struct.pack("<II", il if codec == b"VAG " else 4, start)
    else:
        start = 0x800
        if version == b"4" and codec in (b"PCM ", b"PCMB"):
            start = 0x80
        hdr += bytes(8)
        if name:
            hdr = hdr[:0x18] + name.encode() + b"\0"
    return hdr.ljust(start, b"\0") + data


write(D, "RSD/HR_MUSIC.RSD", rsd(b"4", b"VAG ", [sweep(24000, 0.4), tone(24000, 0.4, [480])], 24000))
write(D, "RSD/RR_DIALOG.RSD", rsd(b"2", b"VAG ", [tone(22050, 0.3, [320, 640]), tone(22050, 0.3, [330])], 22050, il=0x2000))
write(D, "RSD/CTTR_SFX.RSD", rsd(b"6", b"VAG ", [tone(32000, 0.2, [900])], 32000, name="d:/crash/sound/sfx_boom.wav"))
write(D, "RSD/XB_MUSIC.RSD", rsd(b"6", b"XADP", [tone(22050, 0.3, [220]), tone(22050, 0.3, [330])], 22050, name="music.wav"))
write(D, "RSD/GC_RADP.RSD", rsd(b"4", b"RADP", [tone(22050, 0.3, [250]), tone(22050, 0.3, [375])], 22050))
write(D, "RSD/DS_PCM.RSP", rsd(b"3", b"PCM ", [tone(22050, 0.2, [440])], 22050))
write(D, "RSD/HR_PCMB.RSP", rsd(b"4", b"PCMB", [tone(22050, 0.2, [440]), tone(22050, 0.2, [550])], 22050))


# ---------------------------------------------------------------- STR+WAV (Blitz Games)
def blitz_expand(code, hist, idx):
    step = IMA_STEPS[idx]
    if step == 22385:
        step = 22358
    elif step == 24623:
        step = 24633
    d = code & 7
    if code & 8:
        d = -d
    return hist + (step >> 1) + d * step, max(0, min(88, idx + IMA_INDEX[code]))


def put(buf, at, fmt, *v):
    struct.pack_into(fmt, buf, at, *v)


def strwav_ps2(chans, rate, loop=None):
    """Zapper/FOP/Bad Boys II (PS2) header: body interleave 0x8000 (1 track) or 0x4000."""
    ch = len(chans)
    tracks = 1 if ch <= 2 else ch // 2
    il = 0x8000 if tracks == 1 else 0x4000
    enc = [psx_encode(c) for c in chans]
    body = interleave(enc, il) if ch > 1 else enc[0]
    n = len(enc[0]) // 16 * 28
    hs = 0xe0 + 4 * 4
    h = bytearray(hs)
    put(h, 0x04, ">I", 0x800)
    put(h, 0x0c, "<I", 0x12345678)
    flags = (1 if loop else 0) | (2 if ch >= 2 else 0) | 4
    put(h, 0x20, "<iiII", n, rate, 16, flags)
    ls, le = loop or (0, n)
    put(h, 0x38, "<i", ls)
    put(h, 0x40, "<i", tracks)
    put(h, 0x54, "<i", le)
    put(h, 0x70, "<i", rate)
    put(h, 0x78, "<II", 4, 0xe0)
    return bytes(h), body


def strwav_zapper_beta(chans, rate):
    """Zapper Beta (PS2): stereo tracks in 0x20000 chunks."""
    tracks = len(chans) // 2
    enc = [psx_encode(c).ljust(0x10000, b"\0") for c in chans]
    body = bytearray()
    for t in range(tracks):
        body += interleave([enc[2 * t], enc[2 * t + 1]], 0x8000)
    n = min(len(c) for c in chans) // 28 * 28
    h = bytearray(0x78)
    put(h, 0x04, ">I", 0x900)
    put(h, 0x0c, "<I", 0x1234)
    put(h, 0x2c, "<iII", 44100, 1, 2 | 4)
    put(h, 0x5c, "<ii", n, tracks)
    return bytes(h), bytes(body)


def strwav_xbox_pw3(chans, rate):
    """Pac-Man World 3 (Xbox): Xbox IMA, 0xD800 interleave per stereo pair half."""
    data = xbox_stereo(*chans).ljust(0xD800 * 2, b"\0")
    n = len(chans[0]) // 64 * 64
    hs = 0x100 + 2 * 0x40
    h = bytearray(hs)
    put(h, 0x04, ">I", 0x800)
    put(h, 0x0c, "<I", 0x55)
    put(h, 0x20, "<iiII", n, rate, 0x10, 2 | 4)
    put(h, 0x70, "<i", 1)
    put(h, 0xb0, "<i", rate)
    put(h, 0xe0, "<II", 0x100, 2)
    return bytes(h), data


def strwav_pc_bb2(chans, rate, loop=None):
    """Bad Boys II (PC): Blitz IMA."""
    ch = len(chans)
    enc = [pack_nibbles(ima_codes(c, blitz_expand), False) for c in chans]
    body = interleave(enc, 0x10000) if ch > 1 else enc[0]  # one track: 0x10000 interleave
    n = len(chans[0])
    hs = 0x140
    h = bytearray(hs)
    put(h, 0x04, ">I", 0x800)
    put(h, 0x0c, "<I", 0x77)
    put(h, 0x20, "<iiII", n, rate, 16, (1 if loop else 0) | (2 if ch == 2 else 0))
    ls, le = loop or (0, 0)
    put(h, 0x30, "<i", le)
    put(h, 0x38, "<i", ls)
    put(h, 0xf8, "<i", 1)
    put(h, 0x114, "<i", rate)
    put(h, 0x128, "<II", 4, 0x130)
    return bytes(h), body


hd, body = strwav_ps2([sweep(22050, 0.4), sweep(22050, 0.4, 2500, 150)], 22050, loop=(500, 9000))
write(D, "STRWAV/MU_TITLE.WAV", hd)
write(D, "STRWAV/MU_TITLE.WAV.STR", body)
hd, body = strwav_ps2([tone(22050, 0.3, [300])], 22050)
write(D, "STRWAV/VO_LINE.WAV", hd)
write(D, "STRWAV/VO_LINE.STR", body)
hd, body = strwav_ps2([tone(22050, 0.2, [f]) for f in (200, 300, 400, 500)], 22050)
write(D, "STRWAV/MU_4CH.WAV", hd)
write(D, "STRWAV/MU_4CH.WAV.STR", body)
hd, body = strwav_zapper_beta([tone(22050, 0.2, [f]) for f in (250, 350, 450, 550)], 44100)
write(D, "STRWAV/ZB_MUSIC.WAV", hd)
write(D, "STRWAV/ZB_MUSIC.WAV.STR", body)
hd, body = strwav_xbox_pw3([tone(22050, 0.3, [330]), tone(22050, 0.3, [440])], 22050)
write(D, "STRWAV/XB_PW3.WAV", hd)
write(D, "STRWAV/XB_PW3.WAV.STR", body)
hd, body = strwav_pc_bb2([tone(22050, 0.3, [330]), tone(22050, 0.3, [220])], 22050, loop=(100, 6000))
write(D, "STRWAV/PC_BB2.WAV", hd)
write(D, "STRWAV/PC_BB2.WAV.STR", body)


# ---------------------------------------------------------------- MUL (Crystal Dynamics)
CD_STEPS = [min(s, 0x1fff) * 4 for s in IMA_STEPS]
CD_DELTAS = [0x0800, 0x1800, 0x2800, 0x3800, 0x4800, 0x5800, 0x6800, 0x7800,
             -0x0800, -0x1800, -0x2800, -0x3800, -0x4800, -0x5800, -0x6800, -0x7800]


def s16(v):
    return (v + 0x8000) % 0x10000 - 0x8000


def cd_expand(code, hist, idx):
    d = s16((CD_STEPS[idx] * CD_DELTAS[code]) >> 16)
    return clamp16(hist + d), max(0, min(88, idx + IMA_INDEX[code]))


def cd_encode(samples):
    """Crystal Dynamics IMA frames (0x24: hist, step, 0, then 63 nibbles after a skipped one)."""
    out, idx = bytearray(), 0
    for k in range(0, len(samples), 64):
        fr = (samples[k:k + 64] + [0] * 64)[:64]
        hist, start_idx = fr[0], idx
        codes = [0]
        for x in fr[1:]:
            c, hist, idx = ima_greedy(x, hist, idx, cd_expand)
            codes.append(c)
        out += struct.pack("<hBB", fr[0], start_idx, 0) + pack_nibbles(codes, False)
    return bytes(out)


def mul(chans, rate, codec="psx", loop=-1, big=False, block_frames=40):
    ch = len(chans)
    if codec == "psx":
        enc = []
        for c in chans:
            e = bytearray(psx_encode(c, lead=False, end=False))
            for i in range(0, len(e), 16):
                e[i + 1] = 2
            enc.append(bytes(e))
        fb, fs = 16, 28
    else:
        enc = [cd_encode(c) for c in chans]
        fb, fs = 0x24, 64
    n = len(enc[0]) // fb * fs
    p = ">" if big else "<"
    hdr = bytearray(0x800)
    put(hdr, 0, p + "IiII", rate, loop, n, ch)
    put(hdr, 0x38, p + "ff", 1.0, 1.0)
    body = bytearray()
    body += struct.pack(p + "II", 2, 0x20) + bytes(8) + b"\x11" * 0x20  # a non-audio block
    per = block_frames * fb
    for k in range(0, len(enc[0]), per):
        part = [e[k:k + per] for e in enc]
        data = b"".join(part)
        body += struct.pack(p + "II", 0, 0x10 + len(data)) + bytes(8)
        body += struct.pack(p + "I", len(data)) + bytes(12) + data
    return bytes(hdr) + bytes(body)


write(D, "MUL/LOKD_MUSIC.MUL", mul([sweep(22050, 0.4), tone(22050, 0.4, [440])], 22050, loop=1000))
write(D, "MUL/LOKD_AMB.MUL", mul([tone(24000, 0.3, [200, 250])], 24000, block_frames=33))
write(D, "MUL/TRL_PC.MUL", mul([tone(22050, 0.3, [300]), tone(22050, 0.3, [450])], 22050, codec="ima", block_frames=16))


# ---------------------------------------------------------------- SMP (Infernal Engine)
def smp(samples, rate, version=5):
    data = psx_encode(samples)
    n = len(data) // 16 * 28
    hdr = bytearray(0x100)
    put(hdr, 0, "<I", version)
    hdr[4:0x14] = bytes(range(0x10))  # guid
    put(hdr, 0x14, "<IiIII", 0, n, 0x100, len(data), 6)
    put(hdr, 0x28, "<III", 1, 4, rate)
    return bytes(hdr) + data


write(D, "SMP/GB_SLIMER.SMP", smp(tone(22050, 0.3, [600, 800]), 22050))
write(D, "SMP/CG_ARROW.SMP", smp(sweep(32000, 0.2, 3000, 200), 32000, version=8))


# ---------------------------------------------------------------- WD (Square)
def wd(waves, ffxi=False):
    """`waves`: (samples, key semitones) each; data aligned to 0x100."""
    n = len(waves)
    instruments = max(1, n - 1)
    waves_offset = 0x20 + ((instruments * 4 + 15) // 16) * 16
    data = bytearray()
    heads = bytearray()
    for smp_, semis in waves:
        rel = len(data)
        data += psx_encode(smp_)
        data += b"\0" * ((-len(data)) % 0x100)
        heads += struct.pack("<BBHIIIiI", 1, 0, 0, rel + (0x0C if ffxi else 0), 0, 0, semis * (1 << 24), 0).ljust(0x20, b"\0")
    hdr = bytearray(b"WD" + bytes([0x12, 0x00])) + struct.pack("<III", len(data), instruments, n) + bytes(0x10)
    table = b"".join(u32le(waves_offset + 0x20 * min(i, n - 1)) for i in range(instruments))
    hdr = (bytes(hdr) + table).ljust(waves_offset, b"\0")
    return hdr + bytes(heads) + bytes(data)


write(D, "WD/FFX2_SE.WD", wd([(tone(48000, 0.2, [1000]), 0), (tone(24000, 0.3, [300]), -12), (sweep(36000, 0.2, 400, 4000), -5)]))
write(D, "WD/FFXI_W01.WD", wd([(tone(24000, 0.2, [500]), -12), (tone(24000, 0.2, [700]), -12)], ffxi=True))


# ---------------------------------------------------------------- RWS (RenderWare Audio)
RWS_PSX, RWS_PCM, RWS_XBOX = 0xD9EA9798, 0xD01BD217, 0x632FA22B


def rws_string(s):
    b = s.encode() + b"\0"
    return b + b"\0" * ((-len(b)) % 16)


def rws(file_name, codec, layers, segments):
    """`layers`: (channels, rate, block_size, padded_block_size) each; `segments`: (name,
    [channel sample lists per layer]) each."""
    nl, ns = len(layers), len(segments)
    block_layers = sum(l[3] for l in layers)
    data, seg_info, usable = bytearray(), [], []
    for name, per_layer in segments:
        enc = []
        for (ch, rate, bs, pad), chans in zip(layers, per_layer):
            if codec == RWS_PSX:
                e = [psx_encode(c) for c in chans]
            elif codec == RWS_PCM:
                e = [pcm16le(c) for c in chans]
            else:
                e = [xbox_stereo(*chans) if ch == 2 else xbox_mono(chans[0])]
            enc.append(e)
        blocks = 0
        for (ch, rate, bs, pad), e in zip(layers, enc):
            per = bs // ch if codec != RWS_XBOX else bs
            blocks = max(blocks, max((len(x) + per - 1) // per for x in e))
        seg_off = len(data)
        for b in range(blocks):
            for (ch, rate, bs, pad), e in zip(layers, enc):
                blk = bytearray()
                if codec == RWS_XBOX:
                    blk += e[0][b * bs:(b + 1) * bs].ljust(bs, b"\0")
                else:
                    per = bs // ch
                    for x in e:
                        blk += x[b * per:(b + 1) * per].ljust(per, b"\0")
                data += bytes(blk).ljust(pad, b"\0")
        for (ch, rate, bs, pad), e in zip(layers, enc):
            usable.append(sum(len(x) for x in e))
        seg_info.append((blocks * block_layers, seg_off, name))
    h = bytearray(0x50)
    put(h, 0x20, "<I", ns)
    put(h, 0x28, "<I", nl)
    h += rws_string(file_name)
    for size, offset, name in seg_info:
        s = bytearray(0x20)
        put(s, 0x18, "<II", size, offset)
        h += s
    for u in usable:
        h += u32le(u)
    h += bytes(0x10 * ns)
    for _, _, name in seg_info:
        h += rws_string(name)
    start = 0
    for ch, rate, bs, pad in layers:
        l = bytearray(0x28)
        put(l, 0x0c, "<I", 0x1c if codec == RWS_PSX else 1)
        put(l, 0x10, "<I", pad)
        put(l, 0x18, "<HH", bs // ch if codec == RWS_PCM else 0, 0x10)
        put(l, 0x20, "<II", bs, start)
        start += pad
        h += l
    for ch, rate, bs, pad in layers:
        l = bytearray(0x30)
        put(l, 0, "<I", rate)
        l[0x0c], l[0x0d] = 16 if codec == RWS_PCM else 4, ch
        put(l, 0x1c, "<I", codec)
        h += l
    h += bytes(0x10 * nl)
    for i in range(nl):
        h += rws_string("SubStream%d" % i)
    put(h, 0, "<I", len(h))
    header = bytes(h).ljust(0x800 - 0x18, b"\0")
    body = struct.pack("<III", 0x80e, len(header), 0x1c020002) + header
    body += struct.pack("<III", 0x80f, len(data), 0x1c020002) + bytes(data)
    return struct.pack("<III", 0x80d, len(body), 0x1c020002) + body


write(D, "RWS/MP2_MUSIC.RWS", rws("MP2_MUSIC", RWS_PSX, [(2, 32000, 0x2000, 0x2000)],
      [("intro", [[sweep(32000, 0.3), tone(32000, 0.3, [440])]]), ("loop", [[tone(32000, 0.4, [220]), tone(32000, 0.4, [330])]])]))
write(D, "RWS/NANA_VOX.RWS", rws("voices", RWS_PSX, [(1, 22050, 0x800, 0x800), (1, 22050, 0x600, 0x800)],
      [("line1", [[tone(22050, 0.3, [300])], [tone(22050, 0.2, [500])]])]))
write(D, "RWS/KS_PCM.RWS", rws("KS_PCM", RWS_PCM, [(2, 22050, 0x1000, 0x1000)],
      [("pcm", [[tone(22050, 0.2, [400]), tone(22050, 0.2, [600])]])]))
write(D, "RWS/BO2_XBOX.RWS", rws("bo2", RWS_XBOX, [(2, 22050, 0x48 * 32, 0x48 * 32)],
      [("xb", [[tone(22050, 0.2, [400]), tone(22050, 0.2, [600])]])]))


# ---------------------------------------------------------------- AUDIOPKG (Inevitable)
def audiopkg(platform, version, hot, cold, idents):
    """`hot`/`cold`: samples as (kind, chans, rate, loop) with kind "mono", "dual" (two
    mono streams) or "inter" (interleaved stereo); `idents`: (name, [(temperature, index)])."""
    big = platform == b"Game"
    p = ">" if big else "<"
    data = bytearray()
    heads = {0: bytearray(), 2: bytearray()}
    idx = {0: [], 2: []}
    data_base = 0x4000

    def enc(c):
        if platform == b"Play":
            return psx_encode(c)
        return xbox_mono(c)

    for temp, samples in ((0, hot), (2, cold)):
        count = 0
        for kind, chans, rate, loop in samples:
            idx[temp].append(count)
            ls, le = loop or (0, 0)
            if kind == "inter":
                e = [enc(c) for c in chans]
                il = 0x8000 if platform != b"Wind" else 0x9000
                blob = interleave(e, il)
                offs = [len(data)] * 2
                data += blob
                n = len(chans[0])
                sizes = [len(blob)] * 2
            else:
                offs, sizes = [], []
                for c in chans:
                    e = enc(c)
                    offs.append(len(data))
                    sizes.append(len(e))
                    data += e
                    data += b"\0" * ((-len(data)) % 0x800)
                n = len(chans[0])
            for o, s in zip(offs, sizes):
                heads[temp] += struct.pack(p + "IIIIIiiiii", 0, data_base + o, s, 0xFFFFFFFF, 0xFFFFFFFF, 0, n, rate, ls, le)
                count += 1
            data += b"\0" * ((-len(data)) % 0x800)
        idx[temp].append(count)
    # descriptors: one simple cue per (temperature, index), plus complex ones from idents
    strings, str_off = bytearray(), []
    for name, _ in idents:
        str_off.append(len(strings))
        strings += name.encode() + b"\0"
    strings += b"\0" * ((-len(strings)) % 4)
    descs, desc_off = bytearray(), []
    for name, refs in idents:
        desc_off.append(len(descs))
        if len(refs) == 1:
            t, i = refs[0]
            descs += struct.pack(p + "HHHH", 0 << 14, 0, (t << 14) | i, 0)
        else:
            descs += struct.pack(p + "HHH", 1 << 14, 0, len(refs))
            for t, i in refs:
                descs += struct.pack(p + "HHH", 0, (t << 14) | i, 0)
    n_desc = len(idents)
    ident_tab = b"".join(struct.pack(p + "HHI", so, k, 0) for k, so in enumerate(str_off))
    desc_idx = b"".join(struct.pack(p + "I", o) for o in desc_off)
    counts = [len(idx[0]) - 1 if hot else 0, 0, len(idx[2]) - 1 if cold else 0]
    indices = b"".join(struct.pack(p + "H", v) for t in (0, 2) if (hot if t == 0 else cold) for v in idx[t])
    hdr_counts = [len(heads[0]) // 0x28, 0, len(heads[2]) // 0x28]
    pre = {5: 0x60, 6: 0x70, 7: 0x80, 8: 0x80}[version]
    head = bytearray(("v1.%d" % version).encode().ljust(0x10, b"\0") + platform + b"Station II".ljust(0x0c, b"\0"))
    head = head.ljust(0x40 + pre, b"\0")
    head += struct.pack(p + "iiIIIII", n_desc, len(idents), len(descs), len(strings), 0, 0, 0)
    head += struct.pack(p + "iii", *hdr_counts) + struct.pack(p + "iii", *counts) + bytes(0x0c)
    head += struct.pack(p + "iii", 0x28, 0x28, 0x28)
    if version >= 6:
        head += struct.pack(p + "I", 1)
    head += strings + ident_tab + desc_idx + descs + indices + heads[0] + heads[2]
    assert len(head) <= data_base
    return bytes(head).ljust(data_base, b"\0") + bytes(data)


write(D, "AUDIOPKG/A51_SFX.AUDIOPKG", audiopkg(b"Play", 7, [
    ("mono", [tone(22050, 0.2, [800])], 22050, None),
    ("dual", [tone(22050, 0.2, [300]), tone(22050, 0.2, [450])], 22050, None),
], [
    ("inter", [sweep(32000, 0.3), tone(32000, 0.3, [220])], 32000, (1000, 8000)),
], [("SFX_BEEP", [(0, 0)]), ("SFX_BEEP_ALT", [(0, 0)]), ("SFX_WIND", [(0, 1)]), ("MUS_TITLE", [(2, 0)]), ("CUE_MIX", [(0, 1), (2, 0)])]))
write(D, "AUDIOPKG/HOBBIT_VO.AUDIOPKG", audiopkg(b"Play", 5, [], [
    ("mono", [tone(24000, 0.3, [260, 390])], 24000, None),
], [("VO_BILBO_01", [(2, 0)])]))
write(D, "AUDIOPKG/HOBBIT_XB.AUDIOPKG", audiopkg(b"Xbox", 5, [
    ("mono", [tone(22050, 0.2, [500])], 22050, None),
    ("inter", [tone(22050, 0.3, [300]), tone(22050, 0.3, [350])], 22050, None),
], [], [("XB_A", [(0, 0)]), ("XB_B", [(0, 1)])]))
