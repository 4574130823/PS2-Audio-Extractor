//! GbTs - from Konami/KCE Studio games [Pop'n Music 9/10 (PS2)] (vgmstream meta/gbts.c):
//! PS-ADPCM interleaved frame by frame, with byte loop points.

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "GbTs",
    magics: &[b"GbTs"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x30)?;
    if &h[0..4] != b"GbTs" || le32(&h, 0x04) != 0x24 {
        return Ok(vec![]);
    }
    let data_offset = le32(&h, 0x08) as u64;
    let data_size = le32(&h, 0x0c) as u64; // without padding
    let loop_start = le32(&h, 0x10);
    let loop_end = le32(&h, 0x14);
    let rate = le32(&h, 0x18);
    let channels = le32(&h, 0x1c);
    if !(1..=8).contains(&channels) || !sane_rate(rate) || data_offset < 0x28 {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    let start = off + data_offset;
    // Frame interleave: every row is one frame per channel.
    let size = data_size.next_multiple_of(0x10 * channels as u64);
    if data_size == 0 || start + size > ctx.size() {
        return Ok(vec![]);
    }
    if !psx::plausible(&ctx.bytes(start, 0x100.min(size as usize))?) {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(data_size, channels);
    if samples == 0 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, start, size);
    let mut t = Track::new(ctx.entry, off, "GbTs", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(0x10)));
    if loop_end > 0 {
        // The loop region matches the PS-ADPCM flags.
        let ls = psx::bytes_to_samples(loop_start as u64, channels);
        let le = psx::bytes_to_samples(loop_end.wrapping_add(loop_start) as u64, channels);
        if ls < le && le <= samples {
            t = t.looped(ls, le);
        }
    }
    Ok(vec![Found::new(t, start + size)])
}
