//! .PCM - from KCE Japan East PS2 games [Ephemeral Fantasia (PS2), Yu-Gi-Oh! The Duelists of
//! the Roses (PS2), 7 Blades (PS2)] (vgmstream meta/pcm_kceje.c): stereo 16-bit PCM at
//! 24000 Hz after a 0x800 header of data size, sample count and loop points. Known only by
//! extension.

use std::io;

use super::{Ctx, Found, Parser, le32};
use crate::codecs::{Codec, pcm};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "PCM (KCEJE)",
    magics: &[],
    magic_at: 0,
    exts: &["pcm"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 || ctx.size() < 0x800 {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, 0x10)?;
    let data_size = le32(&h, 0x00) as u64;
    let samples = le32(&h, 0x04) as i32;
    if data_size / 4 != le32(&h, 0x04) as u64 || samples <= 0 {
        return Ok(vec![]);
    }
    let samples = samples as u64;
    // The data must be there (the file is the header, the data and some padding).
    if 0x800 + samples * 4 > ctx.size() {
        return Ok(vec![]);
    }
    let loop_start = le32(&h, 0x08) as i32 as i64;
    let loop_end = le32(&h, 0x0c) as i32 as i64;
    let data = Data::at(ctx.entry, 0x800, samples * 4);
    let mut t = Track::new(ctx.entry, 0, "PCM", 2, 24000, samples, data, Codec::Pcm(pcm::Params::le16(2)));
    if loop_end != 0 && loop_start >= 0 && loop_end > loop_start && loop_end as u64 <= samples {
        t = t.looped(loop_start as u64, loop_end as u64);
    }
    Ok(vec![Found::new(t, ctx.size())])
}
