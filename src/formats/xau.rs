//! XAU - XPEC Entertainment sound format [Beat Down (PS2/Xbox), Spectral Force Chronicle
//! (PS2)] (vgmstream meta/xau.c): a mini header over a modified VAGp (PS2) or RIFF (Xbox).

use std::io;

use super::{Ctx, Found, Parser, be32, le32, sane_rate};
use crate::codecs::{Codec, ima, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "XAU",
    magics: &[b"XAU\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x60)?;
    if &h[0..4] != b"XAU\0" || le32(&h, 0x08) != 0x40 {
        return Ok(vec![]);
    }
    let kind = be32(&h, 0x0c);
    let loop_start = le32(&h, 0x10) as i32;
    let loop_end = le32(&h, 0x14) as i32;
    let channels = h[0x18] as u16;
    if !(1..=8).contains(&channels) {
        return Ok(vec![]);
    }
    let ch = channels as u64;
    let (data_off, size, samples, rate, codec) = match kind {
        0x5053_3200 => {
            // "PS2\0"
            if &h[0x40..0x44] != b"VAGp" {
                return Ok(vec![]);
            }
            let per_ch = be32(&h, 0x4c) as u64;
            let data_off = off + 0x800;
            let size = if ch > 1 { per_ch.div_ceil(0x8000) * 0x8000 * ch } else { per_ch };
            if data_off >= ctx.size() || per_ch == 0 {
                return Ok(vec![]);
            }
            let probe = ctx.bytes(data_off, 0x100.min(per_ch) as usize)?;
            if !psx::plausible(&probe) {
                return Ok(vec![]);
            }
            let samples = psx::bytes_to_samples(per_ch * ch, channels);
            (data_off, size, samples, be32(&h, 0x50), Codec::Psx(psx::Params::interleaved(0x8000)))
        }
        0x5842_0000 => {
            // "XB\0\0"
            if &h[0x40..0x44] != b"RIFF" || channels > 2 {
                return Ok(vec![]);
            }
            // The "data" chunk, which may come after a "smpl".
            let mut pos = off + 0x4c;
            let mut found = None;
            for _ in 0..16 {
                if pos + 8 > ctx.size() {
                    break;
                }
                let c = ctx.bytes(pos, 8)?;
                let size = le32(&c, 4) as u64;
                if &c[0..4] == b"data" {
                    found = Some((pos + 8, size));
                    break;
                }
                pos += 8 + size;
            }
            let Some((data_off, size)) = found else { return Ok(vec![]) };
            if size == 0 || data_off + size > ctx.size() {
                return Ok(vec![]);
            }
            let probe = ctx.bytes(data_off, 0x900.min(size) as usize)?;
            if !ima::xbox_plausible(&probe, channels) {
                return Ok(vec![]);
            }
            let samples = ima::xbox_bytes_to_samples(size, channels);
            (data_off, size, samples, le32(&h, 0x58), Codec::Ima(ima::Params::new(ima::Kind::Xbox, 0)))
        }
        _ => return Ok(vec![]),
    };
    if !sane_rate(rate) || samples == 0 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, data_off, size);
    let mut t = Track::new(ctx.entry, off, "XAU", channels, rate, samples, data, codec);
    if loop_end > 0 {
        t = vgm_loop(t, loop_start as i64, loop_end as i64);
    }
    Ok(vec![Found::new(t, (data_off + size).min(ctx.size()))])
}

/// Loop points as vgmstream keeps them: dropped unless 0 <= start < end <= samples.
fn vgm_loop(t: Track, start: i64, end: i64) -> Track {
    if start >= 0 && start < end && end as u64 <= t.samples { t.looped(start as u64, end as u64) } else { t }
}
