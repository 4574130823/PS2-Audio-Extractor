//! VDS/VDM - from Procyon Studio games [Grafitti Kingdom / Rakugaki Oukoku 2 (PS2),
//! Tsukiyo ni Saraba (PS2)] (vgmstream meta/vds_vdm.c).

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_end, psx_start_ok, rows, vgm_loop};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "VDS/VDM",
    magics: &[b"VDS ", b"VDM "],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x24)?;
    if &h[0..4] != b"VDS " && &h[0..4] != b"VDM " {
        return Ok(vec![]);
    }
    let data_size = le32(&h, 0x04) as u64;
    let rate = le32(&h, 0x0c);
    let channels = le32(&h, 0x10); // VDM = mono, VDS = stereo
    let interleave = le32(&h, 0x14) as u64;
    let loop_start = le32(&h, 0x18) as u64; // absolute offsets
    let loop_end = le32(&h, 0x1c) as u64;
    let looped = h[0x20] != 0;
    if !(1..=8).contains(&channels) || !sane_rate(rate) {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    if channels > 1 && (interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000) {
        return Ok(vec![]);
    }
    let start = off + 0x800;
    if !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    // Looping files run to the end of the file (their size field can be short); others
    // have the size.
    let (samples, end) = if looped {
        let end = psx_end(ctx, off, start, channels, interleave, &["vds", "vdm"])?;
        (psx::bytes_to_samples(end - start, channels), end)
    } else {
        if data_size == 0 || start + data_size > ctx.size() + interleave.max(0x800) * 2 {
            return Ok(vec![]);
        }
        (psx::bytes_to_samples(data_size, channels), (start + data_size).min(ctx.size()))
    };
    if samples == 0 {
        return Ok(vec![]);
    }
    let per_channel = (samples / 28 * 16).max((end - start).div_ceil(channels as u64));
    let data = Data::at(ctx.entry, start, rows(per_channel, interleave, channels));
    let t = Track::new(ctx.entry, off, "VDS/VDM", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    let lp = |o: u64| if o >= 0x800 { psx::bytes_to_samples(o - 0x800, channels) as i64 } else { -1 };
    let t = vgm_loop(t, looped, lp(loop_start), lp(loop_end));
    Ok(vec![Found::new(t, end)])
}
