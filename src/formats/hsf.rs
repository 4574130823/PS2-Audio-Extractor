//! HSF - 'SoundBox' driver games (by CAPS?) [EX Jinsei Game (PS2), Lowrider (PS2),
//! Professional Drift: D1 Grand Prix Series (PS2)] (vgmstream meta/hsf.c).

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_end, psx_start_ok, rows, spu2_rate_rounded};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "HSF",
    magics: &[b"HSF\0", b"HSF "],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x10)?;
    let version = match &h[0..4] {
        b"HSF\0" => 1, // SBX driver 1.0.0 / 2.1.0: stores a pitch
        b"HSF " => 3,  // SBX driver 3.2.0: stores the rate
        _ => return Ok(vec![]),
    };
    let raw = le32(&h, 0x08) as i32;
    let rate = if version < 3 { spu2_rate_rounded(raw) } else { raw as i64 };
    let interleave = le32(&h, 0x0c) as u64;
    if !(0..=u32::MAX as i64).contains(&rate) || !sane_rate(rate as u32) {
        return Ok(vec![]);
    }
    if interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000 {
        return Ok(vec![]);
    }
    let (channels, start) = (2u16, off + 0x10);
    if !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let end = psx_end(ctx, off, start, channels, interleave, &["hsf"])?;
    let size = end - start;
    let samples = psx::bytes_to_samples(size, channels);
    if samples == 0 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, start, rows(size.div_ceil(2), interleave, channels));
    let t = Track::new(ctx.entry, off, "HSF", channels, rate as u32, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, end)])
}
