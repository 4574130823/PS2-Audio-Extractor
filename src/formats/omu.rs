//! IMU ("OMU ") - found in Alter Echo (PS2) (vgmstream meta/omu.c): 16-bit PCM in 0x200
//! byte blocks per channel, looping whole.

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, pcm};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "OMU",
    magics: &[b"OMU "],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x40)?;
    if &h[0..4] != b"OMU " || &h[8..12] != b"FRMT" {
        return Ok(vec![]);
    }
    let channels = h[0x14] as u16;
    let rate = le32(&h, 0x10);
    let data_size = le32(&h, 0x3c) as u64;
    if !(1..=8).contains(&channels) || !sane_rate(rate) {
        return Ok(vec![]);
    }
    let start = off + 0x40;
    if data_size == 0 || start + data_size > ctx.size() {
        return Ok(vec![]);
    }
    let samples = data_size / (channels as u64 * 2);
    if samples == 0 {
        return Ok(vec![]);
    }
    // Whole 0x200 blocks per channel are read, even in the last row.
    let size = if channels > 1 { data_size.next_multiple_of(0x200 * channels as u64) } else { data_size };
    let data = Data::at(ctx.entry, start, size);
    let t = Track::new(ctx.entry, off, "OMU", channels, rate, samples, data, Codec::Pcm(pcm::Params::le16(0x200))).looped(0, samples);
    Ok(vec![Found::new(t, (start + size).min(ctx.size()))])
}
