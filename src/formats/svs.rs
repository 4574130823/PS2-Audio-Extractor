//! SVS - SeqVagStream from Square games [Unlimited Saga (PS2) music] (vgmstream
//! meta/svs.c).

use std::io;

use super::{Ctx, Found, Parser, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_end, psx_start_ok, spu2_rate_rounded};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "SVS",
    magics: &[b"SVS\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    if &h[0..4] != b"SVS\0" {
        return Ok(vec![]);
    }
    // 0x04: flags, 0x08/0x0c: loop start/end frames (vgmstream doesn't loop these)
    let pitch = i32::from_le_bytes(h[0x10..0x14].try_into().unwrap());
    let rate = spu2_rate_rounded(pitch);
    if !(0..=u32::MAX as i64).contains(&rate) || !sane_rate(rate as u32) {
        return Ok(vec![]);
    }
    let (channels, interleave) = (2u16, 0x10u64);
    let start = off + 0x20;
    if !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let end = psx_end(ctx, off, start, channels, interleave, &["bgm", "svs"])?;
    let size = end - start;
    let samples = psx::bytes_to_samples(size, channels);
    if samples == 0 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, start, size.div_ceil(0x20) * 0x20);
    let t = Track::new(ctx.entry, off, "SVS", channels, rate as u32, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, end)])
}
