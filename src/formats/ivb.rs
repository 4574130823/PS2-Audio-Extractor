//! IVB - from Metro PS2 games [Bomberman Jetters (PS2), Dance Summit 2001: Bust A Move (PS2)]
//! (vgmstream meta/ivb.c): N stereo PS-ADPCM tracks (subsongs) whose blocks (one interleave
//! per channel) take turns after a 0x800 header.

use std::io;

use super::{Ctx, Found, Parser, le32};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "IVB",
    magics: &[b"IVB\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x800)?;
    if &h[0..4] != b"IVB\0" || le32(&h, 0x0c) != 0 {
        return Ok(vec![]);
    }
    let total = le32(&h, 0x04) as u64;
    let interleave = le32(&h, 0x08) as u64;
    if !(1..=16).contains(&total) || interleave == 0 || interleave % 0x10 != 0 || interleave > 0x40000 {
        return Ok(vec![]);
    }
    let chunk = interleave * 2;
    let start = off + 0x800;
    let mut tracks = Vec::new();
    let mut end = start;
    for s in 0..total {
        let e = 0x10 + s as usize * 0x10;
        let chan_blocks = le32(&h, e + 0x04) as u64;
        let last_size = le32(&h, e + 0x08) as u64; // last block of one channel, without padding
        if le32(&h, e + 0x0c) != 0 || chan_blocks == 0 || last_size > interleave {
            return Ok(vec![]);
        }
        let stream_size = (chan_blocks - 1) * interleave * 2 + last_size * 2;
        let samples = psx::bytes_to_samples(stream_size, 2);
        // Blocks of this track: every `total`-th, starting with block `s`.
        let blocks: Vec<(u64, u64)> = (0..chan_blocks).map(|k| (start + (k * total + s) * chunk, chunk)).collect();
        let last = blocks.last().map(|b| b.0 + b.1).unwrap_or(start);
        if samples == 0 || last > ctx.size() || !psx::plausible(&ctx.bytes(blocks[0].0, 0x100)?) {
            return Ok(vec![]);
        }
        end = end.max(last);
        tracks.push((samples, blocks));
    }
    Ok(tracks
        .into_iter()
        .map(|(samples, blocks)| {
            let t = Track::new(ctx.entry, off, "IVB", 2, 44100, samples, Data::blocks(ctx.entry, blocks), Codec::Psx(psx::Params::interleaved(interleave)));
            Found::new(t, end)
        })
        .collect())
}
