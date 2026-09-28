//! WMW - from Artoon games [Ghost Vibration (PS2)] (vgmstream meta/wmw.c): AICA ADPCM,
//! high nibble first (left channel in the high nibble).

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, aica};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "WMW",
    magics: &[b"WMW "],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    if &h[0..4] != b"WMW " || h[0x04] != 0x02 {
        return Ok(vec![]);
    }
    let loop_flag = h[0x05] != 0;
    let channels = h[0x07] as u16;
    let rate = le32(&h, 0x08);
    let start = le32(&h, 0x0c) as u64;
    let size = le32(&h, 0x10) as u64;
    let (loop_start, loop_end) = (le32(&h, 0x14) as u64, le32(&h, 0x18) as u64);
    if !(1..=2).contains(&channels) || !sane_rate(rate) || start < 0x20 || size == 0 || off + start + size > ctx.size() {
        return Ok(vec![]);
    }
    let samples = aica::bytes_to_samples(size, channels);
    let data = Data::at(ctx.entry, off + start, size);
    let codec = Codec::Aica(aica::Params { high_first: true, interleave: 0 });
    let mut t = Track::new(ctx.entry, off, "WMW", channels, rate, samples, data, codec);
    if loop_flag {
        t = vgm_loop(t, aica::bytes_to_samples(loop_start, channels) as i64, aica::bytes_to_samples(loop_end, channels) as i64);
    }
    Ok(vec![Found::new(t, off + start + size)])
}

/// Loop points as vgmstream keeps them: dropped unless 0 <= start < end <= samples.
fn vgm_loop(t: Track, start: i64, end: i64) -> Track {
    if start >= 0 && start < end && end as u64 <= t.samples { t.looped(start as u64, end as u64) } else { t }
}
