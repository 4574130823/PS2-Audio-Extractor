//! VBK - from High Voltage games [Disney's Stitch: Experiment 626 (PS2)] (vgmstream
//! meta/vbk.c): a bank of PS-ADPCM streams (one subsong each), loop points from the frame
//! flags of long ones.

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "VBK",
    magics: &[b".VBK"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x14)?;
    if &h[0..4] != b".VBK" {
        return Ok(vec![]);
    }
    let total = le32(&h, 0x08) as i32;
    let start = le32(&h, 0x0c) as u64;
    let file_size = le32(&h, 0x10) as u64;
    if total < 1 || total > 10000 || file_size > ctx.size() - off || start < 0x14 + 0x18 * total as u64 || start > file_size {
        return Ok(vec![]);
    }
    let table = ctx.bytes(off + 0x14, 0x18 * total as usize)?;
    let mut found = Vec::new();
    for i in 0..total as usize {
        let e = i * 0x18;
        let stream_size = le32(&table, e) as u64;
        let stream_offset = le32(&table, e + 0x08) as u64;
        let rate = le32(&table, e + 0x0c);
        let interleave = le32(&table, e + 0x10) as u64;
        let channels = le32(&table, e + 0x14).wrapping_add(1); // 4ch is common, 1ch sfx too
        if !(1..=8).contains(&channels) || !sane_rate(rate) || stream_size == 0 {
            return Ok(vec![]);
        }
        let channels = channels as u16;
        if channels > 1 && (interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000) {
            return Ok(vec![]);
        }
        let data_off = start + stream_offset;
        let size = if channels > 1 { stream_size.next_multiple_of(interleave * channels as u64) } else { stream_size };
        if data_off + stream_size > file_size || !psx::plausible(&ctx.bytes(off + data_off, 0x100.min(stream_size as usize))?) {
            return Ok(vec![]);
        }
        let samples = psx::bytes_to_samples(stream_size, channels);
        if samples == 0 {
            return Ok(vec![]);
        }
        let data = Data::at(ctx.entry, off + data_off, size);
        let mut t = Track::new(ctx.entry, off, "VBK", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
        // Only longer sounds loop (over 10 seconds).
        if let Some((ls, le)) = find_loop_offsets(ctx, off + data_off, stream_size, channels, interleave)? {
            if samples > 10 * rate as u64 && ls >= 0 && le > ls && le as u64 <= samples {
                t = t.looped(ls as u64, le as u64);
            }
        }
        found.push(Found::new(t, off + file_size));
    }
    Ok(found)
}

/// vgmstream's `ps_find_loop_offsets`: loop start/end samples from the 0x06/0x03 frame flags
/// of the first channel.
fn find_loop_offsets(ctx: &mut Ctx, start: u64, data_size: u64, channels: u16, interleave: u64) -> io::Result<Option<(i64, i64)>> {
    if data_size == 0 || (channels > 1 && interleave == 0) {
        return Ok(None);
    }
    let buf = ctx.bytes(start, data_size as usize + 0x20)?;
    let max = data_size as usize;
    let (mut num_samples, mut loop_start, mut loop_end) = (0i64, 0i64, 0i64);
    let (mut start_found, mut end_found) = (false, false);
    let mut pos = 0usize;
    let mut consumed = 0u64;
    while pos < max {
        let flag = buf[pos + 1] & 0x0f;
        if flag == 0x06 && !start_found {
            loop_start = num_samples;
            start_found = true;
        }
        if flag == 0x03 && loop_end == 0 {
            loop_end = num_samples + 28;
            end_found = true;
            // Commandos (PS2) has many loop starts and ends.
            if channels == 1 && pos + 0x10 < max && buf[pos + 0x11] & 0x0f == 0x06 {
                loop_end = 0;
                end_found = false;
            }
            if start_found && end_found {
                break;
            }
        }
        num_samples += 28;
        pos += 0x10;
        consumed += 0x10;
        if consumed == interleave {
            consumed = 0;
            pos += (interleave * (channels as u64 - 1)) as usize;
        }
    }
    Ok((start_found && end_found).then_some((loop_start, loop_end)))
}
