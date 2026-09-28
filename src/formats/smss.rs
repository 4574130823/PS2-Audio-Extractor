//! SMSS - from Tiny Toon Adventures: Defenders of the Universe (PS2) (vgmstream
//! meta/smss.c).

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_end, psx_start_ok, rows, vgm_loop};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "SMSS",
    magics: &[b"SMSS"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    if &h[0..4] != b"SMSS" {
        return Ok(vec![]);
    }
    let interleave = le32(&h, 0x08) as u64;
    let channels = le32(&h, 0x0c);
    let rate = le32(&h, 0x10);
    let loop_start = le32(&h, 0x18) as u64;
    let loop_end = le32(&h, 0x1c) as u64;
    if !(1..=8).contains(&channels) || !sane_rate(rate) {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    if channels > 1 && (interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000) {
        return Ok(vec![]);
    }
    let start = off + 0x800;
    if !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let end = psx_end(ctx, off, start, channels, interleave, &["vsf"])?;
    let size = end - start;
    let samples = psx::bytes_to_samples(size, channels);
    if samples == 0 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, start, rows(size.div_ceil(channels as u64), interleave, channels));
    let t = Track::new(ctx.entry, off, "SMSS", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    // Loop offsets are per channel.
    let t = vgm_loop(t, loop_start > 0, psx::bytes_to_samples(loop_start, 1) as i64, psx::bytes_to_samples(loop_end, 1) as i64);
    Ok(vec![Found::new(t, end)])
}
