//! LPCM - from Shade's 'Shade game library' (ShdLib) [Ah! My Goddess (PS2), Warship Gunner
//! (PS2)] (vgmstream meta/lpcm_shade.c): stereo 16-bit PCM at 48000 Hz after a 0x800 header.

use std::io;

use super::{Ctx, Found, Parser, le32};
use crate::codecs::{Codec, pcm};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "LPCM",
    magics: &[b"LPCM"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x14)?;
    if &h[0..4] != b"LPCM" || le32(&h, 0x10) != 0 {
        return Ok(vec![]);
    }
    let samples = le32(&h, 0x04) as i32;
    if samples <= 0 {
        return Ok(vec![]);
    }
    let samples = samples as u64;
    let start = off + 0x800;
    if start + samples * 4 > ctx.size() {
        return Ok(vec![]);
    }
    let loop_start = le32(&h, 0x08) as i32 as i64;
    let loop_end = le32(&h, 0x0c) as i32 as i64;
    let data = Data::at(ctx.entry, start, samples * 4);
    let mut t = Track::new(ctx.entry, off, "LPCM", 2, 48000, samples, data, Codec::Pcm(pcm::Params::le16(2)));
    if loop_start != 0 && loop_start >= 0 && loop_end > loop_start && loop_end as u64 <= samples {
        t = t.looped(loop_start as u64, loop_end as u64);
    }
    Ok(vec![Found::new(t, start + samples * 4)])
}
