//! .VS - from Melbourne House games [Men in Black II (PS2), Grand Prix Challenge (PS2)]
//! (vgmstream meta/vs.c, layout/blocked_vs.c "vs_mh"): stereo PS-ADPCM in blocks, each
//! channel's piece preceded by its size (0x1000 but the last).

use std::io;

use super::{Ctx, Found, Parser, le32};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "VS (MH)",
    magics: &[b"\xC8\x00\x00\x00"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

const BLOCK: u64 = 0x1000;

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x0c)?;
    if h[0..4] != [0xC8, 0, 0, 0] {
        return Ok(vec![]);
    }
    let rate = le32(&h, 0x04);
    if (rate != 48000 && rate != 44100) || le32(&h, 0x08) != 0x1000 {
        return Ok(vec![]);
    }
    let size = ctx.size();
    if off + 0x0c + BLOCK > size || !ps_check_format(&ctx.bytes(off + 0x0c, BLOCK as usize)?) {
        return Ok(vec![]);
    }
    // Block pairs: [size][left data][size][right data]. The samples of each pair come from
    // the second size (vgmstream reads that much of both channels).
    let mut pieces = Vec::new();
    let mut samples = 0u64;
    let mut b = off + 0x08;
    let mut last_short = false;
    while b + 8 <= size && !last_short {
        let size0 = ctx.u32le(b)? as u64;
        if size0 == 0 || size0 > BLOCK || size0 % 0x10 != 0 || b + 8 + size0 > size {
            break;
        }
        let size1 = ctx.u32le(b + 4 + size0)? as u64;
        if size1 == 0 || size1 > BLOCK || size1 % 0x10 != 0 || b + 8 + size0 + size1 > size {
            break;
        }
        pieces.push((b + 4, size1));
        pieces.push((b + 8 + size0, size1));
        samples += psx::bytes_to_samples(size1, 1);
        last_short = size1 < BLOCK; // only the last block can be shorter
        b += 8 + size0 + size1;
    }
    if samples == 0 {
        return Ok(vec![]);
    }
    let t = Track::new(ctx.entry, off, "VS", 2, rate, samples, Data::blocks(ctx.entry, pieces), Codec::Psx(psx::Params::interleaved(BLOCK)));
    Ok(vec![Found::new(t, b)])
}

/// vgmstream's `ps_check_format`: every frame has a valid predictor and flag.
fn ps_check_format(data: &[u8]) -> bool {
    data.chunks(16).all(|f| f[0] >> 4 <= 5 && f.get(1).is_none_or(|&flag| flag <= 7))
}
