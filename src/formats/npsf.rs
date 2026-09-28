//! NPSF - found in Namco NuSound v1 games [Tekken 5 (PS2), Venus & Braves (PS2), Ridge Racer
//! (PSP)] (vgmstream meta/npsf.c): PS-ADPCM with a 0x800 interleave and a stored name.

use std::io;

use super::{Ctx, Found, Parser, label, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "NPSF",
    magics: &[b"NPSF"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x54)?;
    if &h[0..4] != b"NPSF" {
        return Ok(vec![]);
    }
    let channel_size = le32(&h, 0x08) as i32;
    let channels = le32(&h, 0x0c) as i32;
    let start = le32(&h, 0x10) as i32;
    let loop_start = le32(&h, 0x14) as i32;
    let rate = le32(&h, 0x18);
    // 0x28/0x2c: null, 0x30: always 0x40, 0x34: name.
    if !(1..=8).contains(&channels) || !sane_rate(rate) || channel_size <= 0 || start < 0x34 {
        return Ok(vec![]);
    }
    if le32(&h, 0x28) != 0 || le32(&h, 0x2c) != 0 || le32(&h, 0x30) != 0x40 {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    let channel_size = channel_size as u64;
    let start = off + start as u64;
    let size = if channels > 1 { channel_size.next_multiple_of(0x800) * channels as u64 } else { channel_size };
    if start + size > ctx.size() || !psx::plausible(&ctx.bytes(start, 0x100.min(size as usize))?) {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(channel_size, 1);
    if samples == 0 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, start, size);
    let mut t = Track::new(ctx.entry, off, "NPSF", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(0x800)));
    if loop_start != -1 && loop_start >= 0 && (loop_start as u64) < samples {
        t = t.looped(loop_start as u64, samples);
    }
    Ok(vec![Found::new(t, start + size).label(label(&h[0x34..0x54]))])
}
