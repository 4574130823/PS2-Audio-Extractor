//! .MIC - from KOEI games [Crimson Sea 2 (PS2), Dynasty Tactics 2 (PS2)] (vgmstream
//! meta/mic_koei.c). No signature: found by extension only.

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_start_ok, vgm_loop};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "MIC",
    magics: &[],
    magic_at: 0,
    exts: &["mic"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    let start = le32(&h, 0x00) as u64;
    let rate = le32(&h, 0x04);
    let channels = le32(&h, 0x08);
    let interleave = le32(&h, 0x0c) as u64;
    let loop_end = le32(&h, 0x10) as i32; // in blocks of interleave * channels
    let loop_start = le32(&h, 0x14) as i32;
    if start != 0x800 || !(1..=4).contains(&channels) || (interleave != 0x10 && interleave != 0x20) {
        return Ok(vec![]);
    }
    if le32(&h, 0x18) != 0 || le32(&h, 0x1c) != 0 || !sane_rate(rate) || loop_end <= 0 {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    let block = interleave * channels as u64;
    let data_off = off + start;
    let size = loop_end as u64 * block;
    if data_off + size > ctx.size() + 0x800 || !psx_start_ok(ctx, data_off)? {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(size, channels);
    let data = Data::at(ctx.entry, data_off, size);
    let t = Track::new(ctx.entry, off, "MIC", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    let ls = psx::bytes_to_samples((loop_start as i64 * block as i64).max(0) as u64, channels) as i64;
    let t = vgm_loop(t, loop_start != 1, if loop_start < 0 { -1 } else { ls }, samples as i64);
    Ok(vec![Found::new(t, (data_off + size).min(ctx.size()))])
}
