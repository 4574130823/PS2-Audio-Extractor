//! FILp - from Resident Evil: Dead Aim (PS2) (vgmstream meta/filp.c,
//! layout/blocked_filp.c). Several FILp blocks pasted together, each a 0x800 header (with
//! two VAGp headers, sized for the whole stream) and the channels' data one after the other.

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_blocks, vgm_loop};
use crate::track::Track;

pub const PARSER: Parser = Parser {
    name: "FILp",
    magics: &[b"FILp"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x140)?;
    if &h[0..4] != b"FILp" || &h[0x100..0x104] != b"VAGp" || &h[0x130..0x134] != b"VAGp" {
        return Ok(vec![]);
    }
    let channels = le32(&h, 0x04); // stereo only, in practice
    let file_size = le32(&h, 0x0c) as u64;
    let looped = le32(&h, 0x34) == 0; // 00/01/02
    let chan_size = le32(&h, 0x10c) as u64; // whole stream
    let rate = le32(&h, 0x110);
    let end = off + file_size;
    let standalone = off == 0 && ctx.ext() == "fil";
    if !(1..=8).contains(&channels) || !sane_rate(rate) || chan_size < 0x10 {
        return Ok(vec![]);
    }
    if file_size <= 0x800 || end > ctx.size() || (standalone && end != ctx.size()) {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    let samples = psx::bytes_to_samples(chan_size, 1);
    // Blocks until the samples are covered (or the file ends).
    let mut blocks = Vec::new();
    let mut have = 0u64;
    let mut b = off;
    while have < samples && b < end {
        if !ctx.is(b, b"FILp")? {
            break;
        }
        let size = ctx.u32le(b + 0x18)? as u64;
        if size < 0x800 || b + size > end {
            break;
        }
        let per = (size - 0x800) / channels as u64;
        blocks.push((b + 0x800, per, per));
        have += psx::bytes_to_samples(per, 1);
        b += size;
    }
    if blocks.is_empty() ||!psx::plausible(&ctx.bytes(off + 0x800, blocks[0].2.min(0x100) as usize)?) {
        return Ok(vec![]);
    }
    let (data, interleave) = psx_blocks(ctx.entry, &blocks, channels);
    let t = Track::new(ctx.entry, off, "FILp", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    let t = vgm_loop(t, looped, 0, samples as i64);
    Ok(vec![Found::new(t, end)])
}
