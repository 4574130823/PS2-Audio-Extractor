//! BG00 - from Cave games [Ibara (PS2), Mushihime-sama (PS2)] (vgmstream meta/bg00.c).
//! Stereo, with a VAGp header per channel inside the header.

use std::io;

use super::{Ctx, Found, Parser, be32, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_start_ok, rows};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "BG00",
    magics: &[b"BG00"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x90)?;
    if &h[0..4] != b"BG00" || &h[0x40..0x44] != b"VAGp" || &h[0x70..0x74] != b"VAGp" {
        return Ok(vec![]);
    }
    let interleave = le32(&h, 0x10) as u64;
    let chan_size = be32(&h, 0x4c) as u64;
    let rate = be32(&h, 0x80);
    let (channels, start) = (2u16, off + 0x800);
    if !sane_rate(rate) || interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000 || chan_size < 0x10 {
        return Ok(vec![]);
    }
    if start + chan_size > ctx.size() || !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let size = rows(chan_size, interleave, channels);
    let samples = psx::bytes_to_samples(chan_size, 1);
    let data = Data::at(ctx.entry, start, size);
    let t = Track::new(ctx.entry, off, "BG00", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, (start + size).min(ctx.size()))])
}
