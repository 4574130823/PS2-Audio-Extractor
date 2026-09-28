//! .SVAG - from SNK games [World Heroes Anthology (PS2), Fatal Fury Battle Archives 2
//! (PS2)] (vgmstream meta/svag_snk.c).

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_start_ok, vgm_loop};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "SVAG",
    magics: &[b"VAGm"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    if &h[0..4] != b"VAGm" {
        return Ok(vec![]);
    }
    let rate = le32(&h, 0x08);
    let channels = le32(&h, 0x0c);
    let frames = le32(&h, 0x10) as u64; // per channel
    let loop_start = le32(&h, 0x18) as i32 as i64;
    let loop_end = le32(&h, 0x1c) as i32 as i64;
    let start = off + 0x20;
    if !(1..=8).contains(&channels) || !sane_rate(rate) || frames == 0 || !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    let size = frames * 0x10 * channels as u64;
    if start + size > ctx.size() + 0x800 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, start, size);
    let t = Track::new(ctx.entry, off, "SVAG", channels, rate, frames * 28, data, Codec::Psx(psx::Params::interleaved(0x10)));
    // Loop start can be block 0.
    let t = vgm_loop(t, loop_end > 0, loop_start * 28, loop_end * 28);
    Ok(vec![Found::new(t, (start + size).min(ctx.size()))])
}
