"""Dual-file stereo: mono files named as a left/right pair play as one stereo sound
(vgmstream's "dual file stereo"). Also pairs that must NOT join."""
import struct

from common import *  # noqa: F401,F403

D = out_dir("dual")


def vagp(samples, rate, name, loop=None):
    data = psx_encode(samples, loop=loop)
    return b"VAGp" + struct.pack(">IIII", 0x20, 0, len(data), rate) + bytes(12) + name.encode().ljust(16, b"\0")[:16] + data


def smpl(samples, rate, loop_start):
    data = psx_encode(samples)
    h = b"SMPL" + struct.pack(">III", 0x20, 0, len(data) + 0x10) + struct.pack(">I", rate) + bytes(12)
    h += b"HOMURA".ljust(16, b"\0") + struct.pack("<i", loop_start) + bytes(12)
    return h + data


write(D, "BGM/SONG.V0", smpl(tone(22050, 0.5, [300]), 22050, 2800))
write(D, "BGM/SONG.V1", smpl(tone(22050, 0.5, [450]), 22050, 0))
write(D, "BGM/BATTLE_L.VAG", vagp(tone(32000, 0.4, [220]), 32000, "BATTLE_L", loop=(10, 400)))
write(D, "BGM/BATTLE_R.VAG", vagp(tone(32000, 0.4, [330]), 32000, "BATTLE_R", loop=(10, 400)))
write(D, "BGM/THEME.L", vagp(sweep(22050, 0.4, 200, 800), 22050, "THEME"))
write(D, "BGM/THEME.R", vagp(sweep(22050, 0.4, 800, 200), 22050, "THEME"))
# not pairs: no partner, and a partner with another sample rate
write(D, "SE/SOLO_L.VAG", vagp(tone(22050, 0.3, [600]), 22050, "SOLO"))
write(D, "SE/ODD_L.VAG", vagp(tone(22050, 0.3, [700]), 22050, "ODD"))
write(D, "SE/ODD_R.VAG", vagp(tone(11025, 0.6, [700]), 11025, "ODD"))
print("dual fixtures in", D)
