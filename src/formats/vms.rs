//! VMS - from Davilex games [Autobahn Raser: Police Madness (PS2)] (vgmstream meta/vms.c).

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_end, psx_start_ok, rows};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "VMS",
    magics: &[b"VMS "],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    if &h[0..4] != b"VMS " {
        return Ok(vec![]);
    }
    let channels = h[0x08] as u16;
    let interleave = le32(&h, 0x10) as u64;
    let rate = le32(&h, 0x14);
    let stream_offset = le32(&h, 0x1c) as u64;
    if !(1..=8).contains(&channels) || !sane_rate(rate) || stream_offset < 0x20 {
        return Ok(vec![]);
    }
    if channels > 1 && (interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000) {
        return Ok(vec![]);
    }
    let start = off + stream_offset;
    if !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let end = psx_end(ctx, off, start, channels, interleave, &["vms"])?;
    let size = end - start;
    let samples = psx::bytes_to_samples(size, channels);
    if samples == 0 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, start, rows(size.div_ceil(channels as u64), interleave, channels));
    let t = Track::new(ctx.entry, off, "VMS", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, end)])
}
