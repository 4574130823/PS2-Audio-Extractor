//! SL3 - Sirens Sound Library (Winky Soft / Atari Melbourne House) games [Test Drive
//! Unlimited (PS2), Transformers 2003/2004 (PS2)] (vgmstream meta/sl3.c).

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_end, psx_start_ok, rows};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "SL3",
    magics: &[b"SL3\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x24)?;
    if &h[0..4] != b"SL3\0" {
        return Ok(vec![]);
    }
    let channels = le32(&h, 0x14);
    let rate = le32(&h, 0x18);
    let interleave = le32(&h, 0x20) as u64;
    if !(1..=8).contains(&channels) || !sane_rate(rate) {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    if channels > 1 && (interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000) {
        return Ok(vec![]);
    }
    let start = off + 0x8000;
    if !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let end = psx_end(ctx, off, start, channels, interleave, &["ms"])?;
    let size = end - start;
    let samples = psx::bytes_to_samples(size, channels);
    if samples == 0 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, start, rows(size.div_ceil(channels as u64), interleave, channels));
    let t = Track::new(ctx.entry, off, "SL3", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, end)])
}
