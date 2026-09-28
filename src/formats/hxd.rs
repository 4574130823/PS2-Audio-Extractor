//! HXD - from Tecmo games [Tokobot Plus (PS2), Fatal Frame 2/3 (PS2), Gallop Racer 2004
//! (PS2)] (vgmstream meta/hxd.c). The .hxd header describes a bank of mono sounds (data
//! in a .bd or .str) or one stream of N channels (data in a .str or .at3).

use std::io;

use super::ps2p::{vgm_interleaved, vgm_loop};
use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::Track;

pub const PARSER: Parser = Parser {
    name: "HXD",
    magics: &[b"\0DXH"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let size = ctx.size();
    // The header is always its own file (bigfiles may store the data first).
    if off != 0 || size < 0x20 || size > 0x100000 {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, size as usize)?;
    if &h[0..4] != b"\0DXH" || le32(&h, 0x10) as u64 != size {
        return Ok(vec![]);
    }
    let mut total = le32(&h, 0x08) as u64;
    let bank = le32(&h, 0x0c) != 0;
    let interleave = le32(&h, 0x14) as u64;
    let channels = if bank { 1 } else { std::mem::replace(&mut total, 1) };
    if total < 1 || 0x20 + total * 0x1c > size || !(1..=8).contains(&channels) {
        return Ok(vec![]);
    }
    let body = if bank { ctx.sibling("bd").or_else(|| ctx.sibling("str")) } else { ctx.sibling("str").or_else(|| ctx.sibling("at3")) };
    let Some((body, mut br)) = body else { return Ok(vec![]) };
    let body_size = br.size;
    // Xbox versions keep RIFF/WBND data there instead.
    if body_size < 4 || br.bytes(0, 4)? != [0, 0, 0, 0] {
        return Ok(vec![]);
    }
    let entry = |i: u64| 0x20 + i as usize * 0x1c;
    let mut found = Vec::new();
    for i in 0..total {
        let e = entry(i);
        let rate = le32(&h, e) as i32;
        let stream_offset = le32(&h, e + 4) as u64;
        let flags = le16(&h, e + 0x10);
        let loop_start = le32(&h, e + 0x14) as u64 * 0x20;
        let loop_end = le32(&h, e + 0x18) as u64 * 0x20;
        let stream_size = if bank && i + 1 < total {
            let next = (i + 1..total).map(|k| le32(&h, entry(k) + 4) as u64).find(|&n| n > stream_offset).unwrap_or(body_size);
            next.wrapping_sub(stream_offset)
        } else {
            body_size.wrapping_sub(stream_offset)
        };
        if rate <= 0 || !sane_rate(rate as u32) || stream_offset >= body_size || stream_size > body_size {
            continue;
        }
        let samples = psx::bytes_to_samples(stream_size, channels as u16);
        if samples == 0 || (channels > 1 && interleave == 0) {
            continue;
        }
        let data = vgm_interleaved(body, stream_offset, stream_size, channels, interleave, body_size);
        let mut t = Track::new(ctx.entry, 0, "HXD", channels as u16, rate as u32, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
        if flags & 0x20 != 0 {
            let ls = psx::bytes_to_samples(loop_start, channels as u16) as i64;
            let mut le = psx::bytes_to_samples(loop_end, channels as u16) as i64;
            if le == 0 {
                le = samples as i64;
            }
            t = vgm_loop(t, ls, le);
        }
        found.push(Found::new(t, size));
    }
    Ok(found)
}
