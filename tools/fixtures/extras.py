"""Test files for what the scanner does beyond vgmstream: audio in videos (PS2 .PSS, CRI
.SFD), zlib/gzip-compressed data, and headerless PS-ADPCM. Each sound also goes into
tools/testdata/extras_ref/ as a standalone file vgmstream can decode, for comparison.
"""
import gzip
import os
import struct
import zlib

from common import *  # noqa: F401,F403

D = out_dir("extras")
REF = out_dir("extras_ref")


def vagp(data, rate, name=b"SFX"):
    return b"VAGp" + struct.pack(">IIII", 0x20, 0, len(data), rate) + bytes(12) + name.ljust(16, b"\0") + data


def ads(chans, rate, il=0x200):
    data = interleave([psx_encode(c) for c in chans], il)
    return b"SShd" + struct.pack("<IIIII", 0x18, 0x10, rate, len(chans), il) + struct.pack("<ii", -1, -1) + b"SSbd" + u32le(len(data)) + data


def mpeg2_pack():
    return b"\x00\x00\x01\xba\x44\x00\x04\x00\x04\x01\x01\x89\xc3\xf8"


def mpeg1_pack():
    return b"\x00\x00\x01\xba\x21\x00\x01\x00\x01\x80\x00\x01"


def pes(stream_id, payload, mpeg2=True):
    header = (b"\x81\x80\x05\x21\x00\x01\x00\x01") if mpeg2 else b"\x21\x00\x01\x00\x01"
    body = header + payload
    return b"\x00\x00\x01" + bytes([stream_id]) + struct.pack(">H", len(body)) + body


def program_stream(audio, audio_id, mpeg2, sub_header=b""):
    """Packs of video (random) and audio packets, like a video file."""
    out = bytearray()
    pack = mpeg2_pack() if mpeg2 else mpeg1_pack()
    chunk = 0x7e0 - len(sub_header)
    for i in range(0, len(audio), chunk):
        out += pack + pes(0xE0, os.urandom(1500), mpeg2)
        out += pack + pes(audio_id, sub_header + audio[i:i + chunk], mpeg2)
    out += b"\x00\x00\x01\xb9"
    return bytes(out)


# PS2 video: an ADS stream in private stream 1, with a 4-byte sub-stream header per packet.
movie_audio = ads([tone(48000, 0.6, [440]), tone(48000, 0.6, [660])], 48000)
write(D, "MOVIE/OPEN.PSS", program_stream(movie_audio, 0xBD, True, b"\xa0\x00\x00\x00"))
write(REF, "open.ads", movie_audio)

# CRI Sofdec video: ADX in MPEG audio stream 0xC0.
sfd_audio = adx([sweep(44100, 0.7, 300, 2000), tone(44100, 0.7, [500])], 44100)
write(D, "MOVIE/INTRO.SFD", program_stream(sfd_audio, 0xC0, False))
write(REF, "intro.adx", sfd_audio)

# zlib-compressed archive holding a VAG and an ADS, inside other data.
v1 = vagp(psx_encode(tone(22050, 0.4, [880])), 22050, b"ZVAG")
a1 = ads([sweep(22050, 0.5, 200, 1500)], 22050, 0x800)
write(D, "DATA/PACKED.BIN", os.urandom(0x333) + zlib.compress(v1 + os.urandom(100) + a1) + os.urandom(0x200))
write(REF, "zvag.vag", v1)
write(REF, "zads.ads", a1)

# gzip file holding a VAG.
v2 = vagp(psx_encode(tone(32000, 0.5, [300, 450])), 32000, b"GZVAG")
write(D, "DATA/SOUND.GZ", gzip.compress(v2))
write(REF, "gzvag.vag", v2)

# Headerless PS-ADPCM: a .BD bank with no .HD. Each sample: lead frame, data frames, the
# last flagged as the end. The reference VAGs hold the same bytes (up to the end flag).
bank = bytearray()
for k, sig in enumerate([tone(22050, 0.3, [600]), sweep(22050, 0.4, 100, 3000), tone(22050, 0.25, [1200, 300])]):
    frames = psx_encode(sig, end=False)
    bank += frames + bytes(16 * 3)  # some padding between samples
    write(REF, f"raw{k}.vag", vagp(frames, 22050, b"RAW"))
write(D, "SOUND/VOICE.BD", bytes(bank))
print("extras fixtures in", D)
