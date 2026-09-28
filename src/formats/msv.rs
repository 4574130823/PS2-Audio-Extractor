//! MSV - from Sony MultiStream format [Fight Club (PS2), PoPcap Hits Vol. 1 (PS2)]
//! (vgmstream meta/msv.c).

use std::io;

use super::{Ctx, Found, Parser, be32, label, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::psx_start_ok;
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "MSV",
    magics: &[b"MSVp"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x30)?;
    if &h[0..4] != b"MSVp" {
        return Ok(vec![]);
    }
    let size = be32(&h, 0x0c) as u64;
    let rate = be32(&h, 0x10);
    let start = off + 0x30;
    if !sane_rate(rate) || size < 0x10 || start + size > ctx.size() + 0x800 || !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    // No looping; Sony's docs say the end frame is left out.
    let data = Data::at(ctx.entry, start, size);
    let t = Track::new(ctx.entry, off, "MSV", 1, rate, psx::bytes_to_samples(size, 1), data, Codec::Psx(psx::Params::default()));
    Ok(vec![Found::new(t, (start + size).min(ctx.size())).label(label(&h[0x20..0x30]))])
}
