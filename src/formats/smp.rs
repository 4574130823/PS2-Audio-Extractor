//! .smp - Terminal Reality's Infernal Engine 'samples' [Ghostbusters: The Video Game
//! (PS2/PS3/X360/PC/PSP), Chandragupta (PS2/PSP)] (vgmstream meta/smp.c). No signature
//! (a version number): known by extension, and the data must end the file.

use std::io;

use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "SMP",
    magics: &[],
    magic_at: 0,
    exts: &["smp", "snb"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 || ctx.size() < 0x40 {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, 0x40)?;
    let version = le32(&h, 0x00);
    if !(5..=8).contains(&version) || le32(&h, 0x14) != 0 {
        return Ok(vec![]);
    }
    let samples = le32(&h, 0x18) as i32;
    let start = le32(&h, 0x1c) as u64;
    let size = le32(&h, 0x20) as u64;
    let codec = le32(&h, 0x24);
    if start + size != ctx.size() || samples <= 0 {
        return Ok(vec![]);
    }
    let (channels, bps, rate) = if version == 8 && start == 0x80 {
        (h[0x28] as u32, h[0x29] as u32, le16(&h, 0x2a) as u32)
    } else {
        (le32(&h, 0x28), le32(&h, 0x2c), le32(&h, 0x30))
    };
    if !(1..=8).contains(&channels) || !sane_rate(rate) {
        return Ok(vec![]);
    }
    let (codec, note) = match (codec, bps) {
        (0x06, 4) if channels == 1 => (Codec::Psx(psx::Params::default()), None),
        (0x02, 4) if channels == 1 => (Codec::None, Some("GameCube/Wii DSP ADPCM")),
        (0x04, 4) => (Codec::None, Some("MS ADPCM")),
        (0x01, 16) => (Codec::None, Some("ATRAC3")),
        (0x07, 16) => (Codec::None, Some("XMA2")),
        _ => return Ok(vec![]),
    };
    let data = Data::at(ctx.entry, start, size);
    let mut t = Track::new(ctx.entry, 0, "SMP", channels as u16, rate, samples as u64, data, codec);
    t.note = note.map(|n| format!("{n} audio isn't supported"));
    Ok(vec![Found::new(t, start + size)])
}
