//! MUSC - from Krome games [The Legend of Spyro (PS2), Ty the Tasmanian Tiger (PS2)]
//! (vgmstream meta/musc.c).

use std::io;

use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_start_ok, rows, vgm_loop};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "MUSC",
    magics: &[b"MUSC"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x1c)?;
    if &h[0..4] != b"MUSC" {
        return Ok(vec![]);
    }
    let rate = le16(&h, 0x06) as u32;
    let start = le32(&h, 0x10) as u64;
    let data_size = le32(&h, 0x14) as u64;
    let interleave = le32(&h, 0x18) as u64 / 2;
    let channels = 2u16;
    // The data ends the file.
    let end = off + start + data_size;
    let standalone = off == 0 && matches!(ctx.ext().as_str(), "mus" | "musc");
    if !sane_rate(rate) || start < 0x1c || data_size < 0x20 || end > ctx.size() || (standalone && end != ctx.size()) {
        return Ok(vec![]);
    }
    if interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000 || !psx_start_ok(ctx, off + start)? {
        return Ok(vec![]);
    }
    // Always full loops, unless it ends in silence.
    let looped = ctx.u32be(end - 0x10)? != 0x0C00_0000;
    let samples = psx::bytes_to_samples(data_size, channels);
    let data = Data::at(ctx.entry, off + start, rows(data_size.div_ceil(2), interleave, channels));
    let t = Track::new(ctx.entry, off, "MUSC", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    let t = vgm_loop(t, looped, 0, samples as i64);
    Ok(vec![Found::new(t, end)])
}
