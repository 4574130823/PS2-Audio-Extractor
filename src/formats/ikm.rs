//! IKM - MiCROViSiON container [Zwei!! (PS2)] (vgmstream meta/ikm.c). Only the PS2 kind
//! (PS-ADPCM) is read; the PC (Ogg Vorbis) and PSP (ATRAC3) kinds need codecs this app
//! doesn't have and aren't PS2 audio, so they're skipped.

use std::io;

use super::ps2p::{vgm_interleaved, vgm_loop};
use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::Track;

pub const PARSER: Parser = Parser {
    name: "IKM",
    magics: &[b"IKM\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x60)?;
    if &h[0..4] != b"IKM\0" || le32(&h, 0x20) != 3 || &h[0x40..0x44] != b"AST\0" {
        return Ok(vec![]);
    }
    let loop_start = le32(&h, 0x14) as i32;
    let loop_end = le32(&h, 0x18) as i32;
    let rate = le32(&h, 0x44);
    let size = le32(&h, 0x4c) as i32;
    let channels = le32(&h, 0x50) as i32;
    if !(1..=8).contains(&channels) || !sane_rate(rate) || size <= 0 {
        return Ok(vec![]);
    }
    let (channels, size) = (channels as u64, size as u64);
    let start = off + 0x800;
    let samples = psx::bytes_to_samples(size, channels as u16);
    if samples == 0 || start + size > ctx.size() || !psx::plausible(&ctx.bytes(start, 0x100.min(size as usize))?) {
        return Ok(vec![]);
    }
    let data = vgm_interleaved(ctx.entry, start, size, channels, 0x10, ctx.size());
    let mut t = Track::new(ctx.entry, off, "IKM", channels as u16, rate, samples, data, Codec::Psx(psx::Params::interleaved(0x10)));
    if loop_start > 0 {
        t = vgm_loop(t, loop_start as i64, loop_end as i64);
    }
    Ok(vec![Found::new(t, start + size)])
}
