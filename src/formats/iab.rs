//! .IAB - from Runtime(?) games [Ueki no Housoku: Taosu ze Robert Juudan!! (PS2), RPG
//! Maker 3 (PS2)] (vgmstream meta/iab.c, layout/blocked_ps2_iab.c). Stereo PS-ADPCM in
//! blocks, each with a 0x10 header (0x48124812 id, ?, audio size, block size).
//!
//! The header's id (0x10000000) is too common to search for, so files are found by the
//! first block's id (either byte order), and .iab files by name.

use std::io;

use super::{Ctx, Found, Parser, be32, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::psx_blocks;
use crate::track::Track;

pub const PARSER: Parser = Parser {
    name: "IAB",
    magics: &[b"\x12\x48\x12\x48", b"\x48\x12\x48\x12"],
    magic_at: 0x40,
    exts: &["iab"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x40)?;
    if be32(&h, 0) != 0x1000_0000 {
        return Ok(vec![]);
    }
    let rate = le32(&h, 0x04);
    let file_size = le32(&h, 0x1c) as u64;
    let end = off + file_size;
    let standalone = off == 0 && ctx.ext() == "iab";
    if !sane_rate(rate) || file_size <= 0x40 || end > ctx.size() || (standalone && end != ctx.size()) {
        return Ok(vec![]);
    }
    let channels = 2u16;
    let mut blocks = Vec::new();
    let mut samples = 0u64;
    let mut b = off + 0x40;
    loop {
        let bh = ctx.bytes(b, 0x10)?;
        let chan_size = le32(&bh, 0x08) as u64 / channels as u64;
        let mut block_size = le32(&bh, 0x0c) as u64;
        if block_size == 0 {
            block_size = 0x10; // last block
        }
        if b + 0x10 + chan_size * channels as u64 > end {
            return Ok(vec![]);
        }
        blocks.push((b + 0x10, chan_size, chan_size));
        samples += psx::bytes_to_samples(chan_size, 1);
        b += block_size;
        if b >= end {
            break;
        }
    }
    if samples == 0 || !psx::plausible(&ctx.bytes(off + 0x50, blocks[0].2.min(0x100) as usize)?) {
        return Ok(vec![]);
    }
    let (data, interleave) = psx_blocks(ctx.entry, &blocks, channels);
    let t = Track::new(ctx.entry, off, "IAB", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, end)])
}
