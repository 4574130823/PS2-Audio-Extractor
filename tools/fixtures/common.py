"""Shared helpers for building test files in real PS2 audio formats.

Encoders are simple but produce valid streams: PS-ADPCM picks the best predictor and shift
per frame by simulating the decoder; ADX picks the scale per frame the same way. A fixture
script builds files into tools/testdata/<name>/ with `out_dir(name)` and `write(...)`.
"""
import math
import os
import random
import struct

TOOLS = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
random.seed(1234)


def out_dir(name):
    d = os.path.join(TOOLS, "testdata", name)
    os.makedirs(d, exist_ok=True)
    return d


def write(folder, rel, data):
    p = os.path.join(folder, *rel.split("/"))
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "wb") as f:
        f.write(data)
    return p


# ---------------------------------------------------------------- signals
def tone(rate, secs, freqs, amp=9000):
    n = int(rate * secs)
    return [int(sum(amp * math.sin(2 * math.pi * f * i / rate) for f in freqs) / len(freqs)) for i in range(n)]


def sweep(rate, secs, f0=200, f1=4000, amp=8000):
    n = int(rate * secs)
    out, ph = [], 0.0
    for i in range(n):
        f = f0 + (f1 - f0) * i / n
        ph += 2 * math.pi * f / rate
        out.append(int(amp * math.sin(ph)))
    return out


# ---------------------------------------------------------------- byte helpers
def u16le(v): return struct.pack("<H", v & 0xFFFF)
def u16be(v): return struct.pack(">H", v & 0xFFFF)
def u32le(v): return struct.pack("<I", v & 0xFFFFFFFF)
def u32be(v): return struct.pack(">I", v & 0xFFFFFFFF)


def pad(b, align, fill=b"\0"):
    return b + fill * ((-len(b)) % align)


# ---------------------------------------------------------------- PS-ADPCM
COEF = [(0.0, 0.0), (0.9375, 0.0), (1.796875, -0.8125), (1.53125, -0.859375), (1.90625, -0.9375)]


def f32(x):
    return struct.unpack("<f", struct.pack("<f", x))[0]


def _psx_sample(nib, shift, c, h1, h2):
    s = nib << (20 - shift)
    s += int(f32(f32(f32(COEF[c][0]) * h1) + f32(f32(COEF[c][1]) * h2)) * 256.0)
    return s >> 8


def psx_encode(samples, lead=True, end=True, loop=None):
    """PS-ADPCM frames for `samples`. `lead`: a silent frame first (Sony's tools do);
    `end`: a final 0x07 frame; `loop`: (start_frame, end_frame) indexes (of the data
    frames, lead frame excluded) that get the 0x06 / 0x03 loop flags."""
    out = bytearray(16) if lead else bytearray()
    h1 = h2 = 0
    frames = [samples[i:i + 28] for i in range(0, len(samples), 28)] or [[0] * 28]
    for fi, fr in enumerate(frames):
        fr = fr + [0] * (28 - len(fr))
        best = None
        for c in range(5):
            for shift in range(13):
                a, b, err, nibs = h1, h2, 0, []
                for x in fr:
                    pred = int(COEF[c][0] * a + COEF[c][1] * b)
                    q = max(-8, min(7, round((x - pred) * (1 << shift) / 4096)))
                    s = _psx_sample(q, shift, c, a, b)
                    err += (s - x) ** 2
                    b, a = a, s
                    nibs.append(q & 0xF)
                if best is None or err < best[0]:
                    best = (err, c, shift, nibs, a, b)
                if err == 0:
                    break
        _, c, shift, nibs, h1, h2 = best
        flag = 0x00
        if loop and fi == loop[0]:
            flag = 0x06
        elif loop and fi == loop[1]:
            flag = 0x03
        elif fi == len(frames) - 1 and not loop:
            flag = 0x01
        out += bytes([(c << 4) | shift, flag]) + bytes(nibs[i] | (nibs[i + 1] << 4) for i in range(0, 28, 2))
    if end:
        out += bytes([0x00, 0x07]) + bytes(14)
    return bytes(out)


def interleave(chans, il, fill=b"\0"):
    """Channel data blocks of `il` bytes in turn, each channel padded to whole blocks."""
    size = max(len(c) for c in chans)
    size = (size + il - 1) // il * il
    chans = [c.ljust(size, fill) for c in chans]
    out = bytearray()
    for i in range(0, size, il):
        for c in chans:
            out += c[i:i + il]
    return bytes(out)


def pcm16le(samples):
    return b"".join(struct.pack("<h", s) for s in samples)


def pcm16be(samples):
    return b"".join(struct.pack(">h", s) for s in samples)


# ---------------------------------------------------------------- ADX
def adx_coefs(rate, cutoff=500):
    x, y = f32(cutoff), f32(rate)
    z = f32(math.cos(f32(2.0 * math.pi * x / y)))
    a = f32(math.sqrt(2) - z)
    b = f32(math.sqrt(2) - 1.0)
    c = f32(f32(a - f32(math.sqrt(f32(f32(a + b) * f32(a - b))))) / b)
    return int(f32(c * 8192.0)), int(f32(f32(c * c) * -4096.0))


def adx_encode_channel(samples, c1, c2):
    frames, h1, h2 = [], 0, 0
    for i in range(0, len(samples), 32):
        fr = samples[i:i + 32]
        fr = fr + [0] * (32 - len(fr))
        best = None
        for scale in list(range(1, 64)) + list(range(64, 4096, 16)):
            a, b, err, nibs = h1, h2, 0, []
            for x in fr:
                pred = (c1 * a + c2 * b) >> 12
                q = max(-8, min(7, round((x - pred) / scale)))
                s = max(-32768, min(32767, q * scale + pred))
                err += (s - x) ** 2
                b, a = a, s
                nibs.append(q & 0xF)
            if best is None or err < best[0]:
                best = (err, scale, nibs, a, b)
        _, scale, nibs, h1, h2 = best
        frames.append(struct.pack(">h", scale - 1) + bytes((nibs[k] << 4) | nibs[k + 1] for k in range(0, 32, 2)))
    return frames


def adx(chans, rate, loop=None):
    """A version 4 ADX file. `loop`: (start_sample, end_sample)."""
    c1, c2 = adx_coefs(rate)
    enc = [adx_encode_channel(c, c1, c2) for c in chans]
    n = len(chans[0])
    hist = max(8, 4 * len(chans))
    cpo = 0x18 + hist + 0x18 + 2 + 6 - 4  # header, history, loop info, "(c)CRI"
    header = bytearray(struct.pack(">HHBBBBIIHH", 0x8000, cpo, 3, 0x12, 4, len(chans), rate, n, 500, 0x0400))
    header += bytes(4)
    header += bytes(hist)
    if loop:
        header += struct.pack(">HHIIIII", 0, 1, 1, loop[0], 0, loop[1], 0)
    header = header.ljust(cpo + 4 - 6, b"\0") + b"(c)CRI"
    data = bytearray()
    for i in range(len(enc[0])):
        for e in enc:
            data += e[i]
    return bytes(header) + bytes(data)


# ---------------------------------------------------------------- RIFF
def wav(samples, rate, channels=1):
    data = pcm16le(samples)
    return (b"RIFF" + u32le(36 + len(data)) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 1, channels, rate, rate * 2 * channels, 2 * channels, 16)
            + b"data" + u32le(len(data)) + data)
