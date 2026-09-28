//! EXST - from Sony games [Shadow of the Colossus (PS2/PS3), Ape Escape 3 (PS2), Gacha
//! Mecha Stadium Saru Battle (PS2)] (vgmstream meta/exst.c). A 0x78 header (.sts) with
//! the data in a .int, or both joined (.x, some .sts).

use std::io;

use super::ps2p::{vgm_interleaved, vgm_loop};
use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::Track;

pub const PARSER: Parser = Parser {
    name: "EXST",
    magics: &[b"EXST"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x80)?;
    if &h[0..4] != b"EXST" {
        return Ok(vec![]);
    }
    let ext = ctx.ext();
    let is_cp3 = off == 0 && ext == "sts_cp3";
    let channels = le16(&h, 0x06) as u64;
    let rate = le32(&h, 0x08);
    let loop_flag = le32(&h, 0x0c);
    let loop_start = le32(&h, 0x10) as u64;
    let loop_end = le32(&h, 0x14) as u64;
    if !(1..=8).contains(&channels) || !sane_rate(rate) {
        return Ok(vec![]);
    }
    let body = if off == 0 { ctx.sibling(if is_cp3 { "int_cp3" } else { "int" }) } else { None };
    let (entry, start, data_size, file_end, end) = match body {
        Some((e, r)) => (e, 0, r.size, r.size, 0x78.min(ctx.size())),
        None if off == 0 => {
            // Joined header and body, to the end of the file (header padded to 0x80 when
            // the size is a multiple of 0x10 [Gacharoku 2]).
            let size = ctx.size();
            let start = if size % 0x10 == 0 { 0x80 } else { 0x78 };
            if size <= start {
                return Ok(vec![]);
            }
            (ctx.entry, start, size - start, size, size)
        }
        None => {
            // Inside something else: no file size to go by, but the loop end (in blocks)
            // is the stream's end or very close to it.
            if loop_end == 0 {
                return Ok(vec![]);
            }
            let size = loop_end * 0x400 * channels;
            let start = if h[0x78..0x80].iter().all(|&b| b == 0) { off + 0x80 } else { off + 0x78 };
            if start + size > ctx.size() {
                return Ok(vec![]);
            }
            (ctx.entry, start, size, start + size, start + size)
        }
    };
    let (interleave, samples, ls, le, looping) = if !is_cp3 {
        (
            0x400u64,
            psx::bytes_to_samples(data_size, channels as u16),
            psx::bytes_to_samples(loop_start * 0x400 * channels, channels as u16),
            psx::bytes_to_samples(loop_end * 0x400 * channels, channels as u16),
            loop_flag == 1,
        )
    } else {
        (
            0x10,
            psx::bytes_to_samples(data_size, channels as u16),
            psx::bytes_to_samples(loop_start, channels as u16),
            psx::bytes_to_samples(loop_end, channels as u16),
            !(loop_start == 0 && loop_end == data_size),
        )
    };
    if samples == 0 {
        return Ok(vec![]);
    }
    let probe = if entry == ctx.entry { ctx.bytes(start, 0x100.min(data_size as usize))? } else { ctx.game.reader(&ctx.entries[entry])?.bytes(0, 0x100.min(data_size as usize))? };
    if !psx::plausible(&probe) {
        return Ok(vec![]);
    }
    let data = vgm_interleaved(entry, start, data_size, channels, interleave, file_end);
    let mut t = Track::new(ctx.entry, off, "EXST", channels as u16, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    if looping {
        t = vgm_loop(t, ls as i64, le as i64);
    }
    Ok(vec![Found::new(t, end)])
}
