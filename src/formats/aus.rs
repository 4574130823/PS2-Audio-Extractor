//! AUS - Atomic Planet games (APETEC Engine) [Jackie Chan Adventures (PS2), Mega Man
//! Anniversary Collection (PS2/Xbox)] (vgmstream meta/aus.c).

use std::io;

use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, ima, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "AUS",
    magics: &[b"AUS "],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    if &h[0..4] != b"AUS " {
        return Ok(vec![]);
    }
    let codec = le16(&h, 0x06);
    let samples = le32(&h, 0x08) as i32;
    let channels = le16(&h, 0x0c);
    let loop_flag = le16(&h, 0x0e) != 0 || le32(&h, 0x1c) == 1;
    let rate = le32(&h, 0x10);
    let (loop_start, loop_end) = (le32(&h, 0x14) as i32, le32(&h, 0x18) as i32);
    let xbox = codec == 0x02;
    if samples <= 0 || !sane_rate(rate) || !(1..=if xbox { 2 } else { 8 }).contains(&channels) {
        return Ok(vec![]);
    }
    let samples = samples as u64;
    let ch = channels as u64;
    let data_off = off + 0x800;
    if data_off >= ctx.size() {
        return Ok(vec![]);
    }
    // No data size: what the samples need (whole interleave rows for PS-ADPCM).
    let size = if xbox {
        samples.div_ceil(64) * 0x24 * ch
    } else {
        let per_ch = samples.div_ceil(28) * 16;
        if ch > 1 { per_ch.div_ceil(0x800) * 0x800 * ch } else { per_ch }
    };
    let avail = ctx.size() - data_off;
    let probe = ctx.bytes(data_off, 0x800.min(size.min(avail)) as usize)?;
    let codec = if xbox {
        if !ima::xbox_plausible(&probe, channels) {
            return Ok(vec![]);
        }
        Codec::Ima(ima::Params::new(ima::Kind::Xbox, 0))
    } else {
        if !psx::plausible(&probe) {
            return Ok(vec![]);
        }
        Codec::Psx(psx::Params::interleaved(0x800))
    };
    let data = Data::at(ctx.entry, data_off, size);
    let mut t = Track::new(ctx.entry, off, "AUS", channels, rate, samples, data, codec);
    if loop_flag {
        t = vgm_loop(t, loop_start as i64, loop_end as i64);
    }
    Ok(vec![Found::new(t, (data_off + size).min(ctx.size()))])
}

/// Loop points as vgmstream keeps them: dropped unless 0 <= start < end <= samples.
fn vgm_loop(t: Track, start: i64, end: i64) -> Track {
    if start >= 0 && start < end && end as u64 <= t.samples { t.looped(start as u64, end as u64) } else { t }
}
