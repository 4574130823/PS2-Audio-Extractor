//! MSH+MSB - SCEE MultiStream flat bank [namCollection: Ace Combat 2 (PS2) sfx, EyeToy Play
//! (PS2)] (vgmstream meta/msh_msb.c). The .MSH lists mono PS-ADPCM sounds in the .MSB.
//! Found by extension (the header has no signature).

use std::io;

use super::ps2p::{Sub, ps_find_loop, vgm_loop};
use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "MSH/MSB",
    magics: &[],
    magic_at: 0,
    exts: &["msh"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let size = ctx.size();
    if off != 0 || size < 0x0c || size > 0x100000 {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, size as usize)?;
    let entries = le32(&h, 0x08) as usize;
    if le32(&h, 0) as u64 != size || 0x0c + entries * 0x10 > h.len() {
        return Ok(vec![]);
    }
    let Some((body, br)) = ctx.sibling("msb") else { return Ok(vec![]) };
    let mut found = Vec::new();
    for i in 0..entries {
        let e = 0x0c + 0x10 * i;
        let stream_size = le32(&h, e) as u64;
        let config = le32(&h, e + 4);
        let start = le32(&h, e + 8) as u64;
        let rate = le32(&h, e + 0x0c);
        if stream_size == 0 || rate == 0 {
            continue; // empty entry
        }
        if !sane_rate(rate) || start >= br.size {
            continue;
        }
        let samples = psx::bytes_to_samples(stream_size, 1);
        if samples == 0 {
            continue;
        }
        let data = Data::at(body, start, stream_size);
        let mut t = Track::new(ctx.entry, 0, "MSH/MSB", 1, rate, samples, data, Codec::Psx(psx::Params::default()));
        if config & 1 != 0 {
            // vgmstream looks for the loop flags in the header file (not the .MSB), at the
            // data's offset: kept as is for identical results.
            if let Some((a, b)) = ps_find_loop(&mut Sub::new(&mut ctx.r, 0, size), start, stream_size, 1, 0, false)? {
                t = vgm_loop(t, a, b);
            }
        }
        found.push(Found::new(t, size));
    }
    Ok(found)
}
