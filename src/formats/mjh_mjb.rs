//! MJH+MJB - SCEE MultiStream? bank of MIH+MIB [Star Wars: Bounty Hunter (PS2)] (vgmstream
//! meta/mjh_mjb.c). The .MJH header lists streams that are interleaved PS-ADPCM in the
//! .MJB, back to back. Found by extension (the header has no signature).

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "MJH/MJB",
    magics: &[],
    magic_at: 0,
    exts: &["mjh"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let size = ctx.size();
    if off != 0 || size < 0x40 || size > 0x40 * 0x10000 {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, size as usize)?;
    let total = le32(&h, 0) as u64;
    if total * 0x40 + 0x40 != size || le32(&h, 0x10) != 0 || le32(&h, 0x20) != 0 || le32(&h, 0x30) != 0 || total < 1 {
        return Ok(vec![]);
    }
    let Some((body, br)) = ctx.sibling("mjb") else { return Ok(vec![]) };
    let body_size = br.size;
    let mut found = Vec::new();
    let mut start = 0u64;
    for i in 0..total as usize {
        let e = 0x40 + 0x40 * i;
        let channels = le32(&h, e + 0x08) as u64;
        let rate = le32(&h, e + 0x0c);
        let frame_size = le32(&h, e + 0x10) as u64;
        let frame_count = le32(&h, e + 0x14) as u64;
        let data_size = frame_count * frame_size * channels;
        let this = start;
        start += (channels * frame_size * frame_count) & 0xffff_ffff;
        if le32(&h, e) != 0x40 || !(1..=8).contains(&channels) || !sane_rate(rate) || frame_size == 0 || frame_size % 0x10 != 0 || data_size == 0 || this >= body_size {
            continue;
        }
        let samples = psx::bytes_to_samples(data_size, channels as u16);
        let data = Data::at(body, this, data_size);
        let t = Track::new(ctx.entry, 0, "MJH/MJB", channels as u16, rate, samples, data, Codec::Psx(psx::Params::interleaved(frame_size)));
        found.push(Found::new(t, size));
    }
    Ok(found)
}
