//! .VGV - from Rune: Viking Warlord (PS2) (vgmstream meta/vgv.c). No signature: found by
//! extension only.

use std::io;

use super::{Ctx, Found, Parser, le32};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "VGV",
    magics: &[],
    magic_at: 0,
    exts: &["vgv"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x10)?;
    let rate = le32(&h, 0x00);
    let duration = f32::from_le_bytes(h[4..8].try_into().unwrap());
    if !(22050..=48000).contains(&rate) || duration == 0.0 || duration > 500.0 {
        return Ok(vec![]);
    }
    if le32(&h, 0x08) != 0 || le32(&h, 0x0c) != 0 || off != 0 {
        return Ok(vec![]);
    }
    let start = 0x10;
    if !super::a2m::psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    // vgmstream counts the whole file (header included) as data: the last frame reads past
    // the end, as silence.
    let size = ctx.size();
    let samples = psx::bytes_to_samples(size, 1);
    if samples == 0 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, start, size);
    let t = Track::new(ctx.entry, off, "VGV", 1, rate, samples, data, Codec::Psx(psx::Params::default()));
    Ok(vec![Found::new(t, size)])
}
