//! .vs/STRx - from The Bouncer (PS2) (vgmstream meta/vs_str.c, layout/blocked_vs_str.c).
//! Blocks of 0x800 bytes per channel ("STRL" + "STRR", or "STRM" for mono voices), each
//! with a 0x20 header holding the size of its audio.

use std::io;

use super::{Ctx, Found, Parser};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::psx_blocks;
use crate::track::Track;

pub const PARSER: Parser = Parser {
    name: "VS/STR",
    magics: &[b"STRL", b"STRM"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let channels: u16 = if ctx.is(off, b"STRM")? {
        1
    } else if ctx.is(off, b"STRL")? && ctx.is(off + 0x800, b"STRR")? {
        2
    } else {
        return Ok(vec![]);
    };
    let ids: &[&[u8]] = if channels == 1 { &[b"STRM"] } else { &[b"STRL", b"STRR"] };
    let file_size = ctx.size();
    // A standalone file is all blocks (as vgmstream reads it); elsewhere, blocks run while
    // they have their ids.
    let standalone = off == 0 && matches!(ctx.ext().as_str(), "vs" | "str");
    let step = 0x800 * channels as u64;
    let mut blocks = Vec::new();
    let mut samples = 0u64;
    let mut b = off;
    while b < file_size {
        if !standalone {
            let mut ok = b + step <= file_size;
            for (i, id) in ids.iter().enumerate() {
                ok = ok && ctx.is(b + 0x800 * i as u64, id)?;
            }
            if !ok {
                break;
            }
        }
        let size = ctx.u32le(b + 0x04)? as u64; // can be smaller than 0x800
        if size > 0x800 - 0x20 {
            if blocks.is_empty() {
                return Ok(vec![]);
            }
            break;
        }
        blocks.push((b + 0x20, 0x800, size));
        samples += psx::bytes_to_samples(size, 1);
        b += step;
    }
    if samples == 0 {
        return Ok(vec![]);
    }
    let first = ctx.bytes(off + 0x20, blocks[0].2.min(0x100) as usize)?;
    if !psx::plausible(&first) {
        return Ok(vec![]);
    }
    let (data, interleave) = psx_blocks(ctx.entry, &blocks, channels);
    let t = Track::new(ctx.entry, off, "VS/STR", channels, 44100, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, b.min(file_size))])
}
