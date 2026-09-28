//! SVGp - from High Voltage games [Hunter: The Reckoning - Wayward (PS2)] (vgmstream
//! meta/svgp.c).

use std::io;

use super::{Ctx, Found, Parser, be32, label, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_start_ok, rows, vgm_loop};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "SVGp",
    magics: &[b"SVGp"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x30)?;
    if &h[0..4] != b"SVGp" {
        return Ok(vec![]);
    }
    let interleave = le32(&h, 0x14) as u64;
    let data_size = le32(&h, 0x18) as u64;
    let rate = be32(&h, 0x2c);
    let (channels, start) = (2u16, off + 0x800);
    if !sane_rate(rate) || interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000 || data_size < 0x20 {
        return Ok(vec![]);
    }
    if start + data_size > ctx.size() + interleave * 2 ||!psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let lp = find_loop(ctx, start, data_size, channels, interleave)?;
    let samples = psx::bytes_to_samples(data_size, channels);
    let size = rows(data_size.div_ceil(2), interleave, channels);
    let data = Data::at(ctx.entry, start, size);
    let t = Track::new(ctx.entry, off, "SVGp", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    let (ls, le) = lp.unwrap_or((0, 0));
    let t = vgm_loop(t, lp.is_some(), ls, le);
    Ok(vec![Found::new(t, (start + data_size).min(ctx.size())).label(label(&h[0x04..0x14]))])
}

/// Loop points from the PS-ADPCM frame flags, exactly as vgmstream's ps_find_loop_offsets
/// (first 0x06 = start, first 0x03 = end; channel 0's frames only).
fn find_loop(ctx: &mut Ctx, start: u64, data_size: u64, channels: u16, interleave: u64) -> io::Result<Option<(i64, i64)>> {
    if data_size == 0 || channels == 0 || (channels > 1 && interleave == 0) {
        return Ok(None);
    }
    let max = start + data_size;
    let buf = ctx.bytes(start, (data_size + 0x10) as usize)?;
    let flag_at = |o: u64| buf[(o - start + 1) as usize] & 0x0f;
    let (mut offset, mut consumed, mut samples) = (start, 0u64, 0i64);
    let (mut loop_start, mut loop_end) = (0i64, 0i64);
    let (mut start_found, mut end_found) = (false, false);
    while offset < max {
        let flag = flag_at(offset);
        if flag == 0x06 && !start_found {
            loop_start = samples;
            start_found = true;
        }
        if flag == 0x03 && loop_end == 0 {
            loop_end = samples + 28;
            end_found = true;
            // Commandos (PS2): many loop starts and ends
            if channels == 1 && offset + 0x10 < max && flag_at(offset + 0x10) == 0x06 {
                loop_end = 0;
                end_found = false;
            }
            if start_found && end_found {
                break;
            }
        }
        samples += 28;
        offset += 0x10;
        consumed += 0x10;
        if consumed == interleave {
            consumed = 0;
            offset += interleave * (channels as u64 - 1);
        }
    }
    Ok((start_found && end_found).then_some((loop_start, loop_end)))
}
