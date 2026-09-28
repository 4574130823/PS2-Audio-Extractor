//! STER - from Silicon Studios/Vicarious Visions's ALCHEMY middleware [Baroque (PS2), Star
//! Soldier (PS2)] (vgmstream meta/ster.c). Stereo only (mono files are plain VAGs).

use std::io;

use super::{Ctx, Found, Parser, be32, label, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_start_ok, vgm_loop};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "STER",
    magics: &[b"STER"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x30)?;
    if &h[0..4] != b"STER" {
        return Ok(vec![]);
    }
    let chan_size = le32(&h, 0x04) as u64;
    let loop_start = le32(&h, 0x08); // absolute (0x50 = full loop)
    // 0x0c: data size, big endian
    let rate = be32(&h, 0x10);
    let (channels, start) = (2u16, off + 0x30);
    if !sane_rate(rate) || chan_size < 0x10 {
        return Ok(vec![]);
    }
    let size = chan_size.div_ceil(0x10) * 0x20;
    if start + size > ctx.size() + 0x800 || !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(chan_size, 1);
    let data = Data::at(ctx.entry, start, size);
    let t = Track::new(ctx.entry, off, "STER", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(0x10)));
    let ls = if loop_start >= 0x30 { psx::bytes_to_samples(loop_start as u64 - 0x30, channels) as i64 } else { -1 };
    let t = vgm_loop(t, loop_start != 0xFFFF_FFFF, ls, samples as i64);
    Ok(vec![Found::new(t, (start + size).min(ctx.size())).label(label(&h[0x20..0x30]))])
}
