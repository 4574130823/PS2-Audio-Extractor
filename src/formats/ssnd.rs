//! SSND - The 3DS Company games [Warriors of Might & Magic (PS2), Portal Runner (PS2)]
//! (vgmstream meta/ssnd.c): a small header, then PCM or DVI IMA, interleaved.

use std::io;

use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, ima, pcm};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "SSND",
    magics: &[b"SSND"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x1a)?;
    if &h[0..4] != b"SSND" {
        return Ok(vec![]);
    }
    let start = le32(&h, 0x04) as u64 + 8;
    let codec = le16(&h, 0x08);
    let channels = le16(&h, 0x0a);
    let bits = le16(&h, 0x0c);
    let rate = le32(&h, 0x0e);
    let interleave = le32(&h, 0x12) as u64;
    let samples = le32(&h, 0x16) as i32;
    if !(1..=8).contains(&channels) || !sane_rate(rate) || samples <= 0 || !(0x1a..=0x1000).contains(&start) || bits != 16 {
        return Ok(vec![]);
    }
    if (channels > 1 && interleave == 0) || interleave > 0x10000 {
        return Ok(vec![]);
    }
    let samples = samples as u64;
    let data_off = off + start;
    if data_off >= ctx.size() {
        return Ok(vec![]);
    }
    // Bytes per channel the samples take.
    let (per_ch, codec) = match codec {
        0x00 => (samples * 2, Codec::Pcm(pcm::Params::le16(interleave))),
        0x01 => (samples.div_ceil(2), Codec::Ima(ima::Params::new(ima::Kind::Dvi, interleave))),
        _ => return Ok(vec![]),
    };
    // vgmstream takes the data to the end of the file (it matters for the last, shorter
    // interleave block); inside other files, what the samples need.
    let needed = if interleave > 0 && channels > 1 {
        per_ch / interleave * interleave * channels as u64 + per_ch % interleave * channels as u64
    } else {
        per_ch * channels as u64
    };
    let standalone = off == 0 && ctx.ext() == "snd";
    let avail = ctx.size() - data_off;
    let size = if standalone { avail } else { needed.min(avail) };
    if size == 0 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, data_off, size);
    let t = Track::new(ctx.entry, off, "SSND", channels, rate, samples, data, codec).looped(0, samples);
    Ok(vec![Found::new(t, data_off + size)])
}
