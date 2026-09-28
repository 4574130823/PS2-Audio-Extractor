//! SPM - Seq-PCM stream Square Sounds Co. games [Lethal Skies Elite Pilot: Team SW (PS2)]
//! (vgmstream meta/spm.c): stereo 16-bit PCM at 48000 Hz after a 0x20 header.

use std::io;

use super::{Ctx, Found, Parser, le32};
use crate::codecs::{Codec, pcm};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "SPM",
    magics: &[b"SPM\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    if &h[0..4] != b"SPM\0" || h[0x14..0x20].iter().any(|&b| b != 0) {
        return Ok(vec![]);
    }
    // The size includes the header.
    let data_size = le32(&h, 0x04) as u64;
    if data_size <= 0x20 || off + data_size > ctx.size() {
        return Ok(vec![]);
    }
    let samples = pcm::bytes_to_samples(data_size - 0x20, 2, 16);
    if samples == 0 {
        return Ok(vec![]);
    }
    let loop_start = le32(&h, 0x08) as i32 as i64;
    let loop_end = le32(&h, 0x0c) as i32 as i64;
    let data = Data::at(ctx.entry, off + 0x20, samples * 4);
    let mut t = Track::new(ctx.entry, off, "SPM", 2, 48000, samples, data, Codec::Pcm(pcm::Params::le16(2)));
    if loop_start >= 0 && loop_end > loop_start && loop_end as u64 <= samples {
        t = t.looped(loop_start as u64, loop_end as u64);
    }
    Ok(vec![Found::new(t, off + data_size)])
}
