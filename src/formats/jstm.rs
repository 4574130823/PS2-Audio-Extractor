//! JSTM - from Tantei Jinguji Saburo - Kind of Blue (PS2) (vgmstream meta/jstm.c): 16-bit
//! PCM after a 0x20 header, every data byte XORed with 0x5A.
//!
//! The PCM codec has no XOR option yet, so these tracks are listed but not decoded.

use std::io;

use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, pcm};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "JSTM",
    magics: &[b"JSTM"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    if &h[0..4] != b"JSTM" {
        return Ok(vec![]);
    }
    let channels = le16(&h, 0x04);
    let rate = le32(&h, 0x08);
    let size = le32(&h, 0x0c) as u64;
    let loop_at = le32(&h, 0x14) as i32;
    if channels != le16(&h, 0x06) || !(1..=8).contains(&channels) || !sane_rate(rate) {
        return Ok(vec![]);
    }
    let start = off + 0x20;
    if size == 0 || start + size > ctx.size() {
        return Ok(vec![]);
    }
    let samples = size / channels as u64 / 2;
    if samples == 0 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, start, samples * 2 * channels as u64);
    // The PCM is scrambled with XOR 0x5A (vgmstream's meta/jstm_streamfile.h).
    let mut t = Track::new(ctx.entry, off, "JSTM", channels, rate, samples, data, Codec::Pcm(pcm::Params { xor: 0x5A, ..pcm::Params::le16(2) }));
    if loop_at != -1 && loop_at >= 0 {
        let loop_start = loop_at as u64 / channels as u64 / 2;
        if loop_start < samples {
            t = t.looped(loop_start, samples);
        }
    }
    Ok(vec![Found::new(t, start + size)])
}
