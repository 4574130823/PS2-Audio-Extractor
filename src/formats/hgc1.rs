//! hgC1 - from Knights of the Temple 2 (PS2) (vgmstream meta/hgc1.c).

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::psx_start_ok;
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "hgC1",
    magics: &[b"hgC1strm"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    if &h[0..8] != b"hgC1strm" {
        return Ok(vec![]);
    }
    let channels = le32(&h, 0x08);
    let frames = le32(&h, 0x0c) as u64; // mono frames
    let rate = le32(&h, 0x10);
    let start = off + 0x20;
    if !(1..=8).contains(&channels) || !sane_rate(rate) || frames == 0 || !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    let size = frames * 0x10 * channels as u64;
    if start + size > ctx.size() + 0x800 {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(frames * 0x10, 1);
    let data = Data::at(ctx.entry, start, size);
    let t = Track::new(ctx.entry, off, "hgC1", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(0x10)));
    Ok(vec![Found::new(t, (start + size).min(ctx.size()))])
}
