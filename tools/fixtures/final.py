"""Last checks: ADS IMA "hijack" (Rockstar San Diego videos: PCM header with 12000 Hz and
0x200 interleave means 48000 Hz IMA with 0x40 interleave), and ADS codec 0 (PCM16 BE,
used by PS2 video audio headers; not read by vgmstream, so checked against the same
samples stored as PCM16 LE)."""
import os
import struct

from common import *  # noqa: F401,F403

D = out_dir("final")
own = out_dir("final_own")
random_ima = os.urandom(0x40 * 2 * 60)
write(D, "VIDEO/RSTAR.ADS", b"SShd" + struct.pack("<IIIII", 0x18, 0x01, 12000, 2, 0x200) + struct.pack("<ii", -1, -1)
      + b"SSbd" + u32le(len(random_ima)) + random_ima)
l, r = tone(48000, 0.3, [440]), tone(48000, 0.3, [550])
be = interleave([pcm16be(l), pcm16be(r)], 0x200)
le = interleave([pcm16le(l), pcm16le(r)], 0x200)
hdr = lambda codec, data: b"SShd" + struct.pack("<IIIII", 0x18, codec, 48000, 2, 0x200) + struct.pack("<ii", -1, -1) + b"SSbd" + u32le(len(data))
write(own, "BE.ADS", hdr(0x00, be) + be)
write(own, "LE.ADS", hdr(0x01, le) + le)
print("final fixtures in", D, "and", own)
