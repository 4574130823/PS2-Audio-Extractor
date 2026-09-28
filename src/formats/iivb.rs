//! IIVB - from Vingt-et-un Systems games [Langrisser III (PS2), Ururun Quest: Koiyuuki
//! (PS2)] (vgmstream meta/iivb.c).

use std::io;

use super::{Ctx, Found, Parser, be32, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "IIVB",
    magics: &[b"BVII"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x10)?;
    if &h[0..4] != b"BVII" {
        return Ok(vec![]);
    }
    let chan_size = le32(&h, 0x04) as u64;
    let rate = be32(&h, 0x08); // big endian, unlike the rest
    let start = off + 0x10;
    // Channel 1 has to start inside the file.
    if !sane_rate(rate) || chan_size < 0x10 || start + chan_size >= ctx.size() || !super::a2m::psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(chan_size, 1);
    let data = Data::at(ctx.entry, start, chan_size * 2);
    let t = Track::new(ctx.entry, off, "IIVB", 2, rate, samples, data, Codec::Psx(psx::Params::interleaved(chan_size)));
    Ok(vec![Found::new(t, (start + chan_size * 2).min(ctx.size()))])
}
