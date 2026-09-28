//! AHV - from Amuze games [Headhunter (PS2)] (vgmstream meta/ahv.c).

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::psx_start_ok;
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "AHV",
    magics: &[b"AHV\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x14)?;
    if &h[0..4] != b"AHV\0" {
        return Ok(vec![]);
    }
    let chan_size = le32(&h, 0x08) as u64;
    let rate = le32(&h, 0x0c);
    let interleave = le32(&h, 0x10) as u64;
    let channels: u16 = if interleave != 0 { 2 } else { 1 };
    let start = off + 0x800;
    if !sane_rate(rate) || chan_size < 0x10 || interleave % 0x10 != 0 || interleave > 0x10000 {
        return Ok(vec![]);
    }
    if start + chan_size * channels as u64 > ctx.size() + 0x800 || !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    // The data runs to the end of the file, and its last block is shorter (split evenly
    // between the channels); inside other files, the channel size tells where it ends.
    let data_size = if off == 0 && ctx.ext() == "ahv" { ctx.size() - start } else { chan_size * channels as u64 };
    let samples = psx::bytes_to_samples(chan_size, 1);
    let data = Data::at(ctx.entry, start, data_size);
    let t = Track::new(ctx.entry, off, "AHV", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, (start + data_size).min(ctx.size()))])
}
