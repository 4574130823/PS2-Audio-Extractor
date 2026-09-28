//! XABp - cavia PS2 bank format [Drakengard 1/2 (PS2), Ghost in the Shell: SAC (PS2),
//! Resident Evil: Dead Aim (PS2)] (vgmstream meta/xabp.c): a .HD2 header ("pBAX") listing
//! mono PS-ADPCM sounds in the .BD next to it. Sizes aren't stored: a sound runs to its end
//! (or loop end) frame flag.

use std::io;

use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, Stream, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "XABp",
    magics: &[],
    magic_at: 0,
    exts: &["hd2"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 || !ctx.is(0, b"pBAX")? {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, 0x10)?;
    let bd_size = le32(&h, 0x04) as u64;
    let total = le16(&h, 0x0c) as i16;
    if total < 1 || 0x10 + 0x20 * total as u64 > ctx.size() {
        return Ok(vec![]);
    }
    let Some((bd_entry, mut bd)) = ctx.sibling("bd") else { return Ok(vec![]) };
    if bd.size < bd_size {
        return Ok(vec![]);
    }
    let table = ctx.bytes(0x10, 0x20 * total as usize)?;
    let mut found = Vec::new();
    for i in 0..total as usize {
        let e = i * 0x20;
        let pitch = le16(&table, e + 0x0e) as u64;
        let rate = 48000 * pitch / 4096; // spu2_pitch_to_sample_rate
        let stream_offset = le32(&table, e + 0x18) as u64;
        if rate > 96000 || !sane_rate(rate as u32) || stream_offset >= bd_size {
            return Ok(vec![]);
        }
        let all = Data::at(bd_entry, stream_offset, bd_size - stream_offset);
        let (stream_size, loops) = find_stream_info(&mut Stream::new(&mut bd, &all))?;
        let samples = psx::bytes_to_samples(stream_size, 1);
        if samples == 0 {
            continue;
        }
        let data = Data::at(bd_entry, stream_offset, stream_size);
        let mut t = Track::new(ctx.entry, 0, "XABp", 1, rate as u32, samples, data, Codec::Psx(psx::Params::default()));
        if let Some((ls, le)) = loops {
            if ls < le && le <= samples {
                t = t.looped(ls, le);
            }
        }
        found.push(Found::new(t, ctx.size()));
    }
    Ok(found)
}

/// vgmstream's `ps_find_stream_info` for mono data: the size up to the first frame with an
/// end flag (plus one frame, as vgmstream counts it), and loop points from the flags.
fn find_stream_info(s: &mut Stream) -> io::Result<(u64, Option<(u64, u64)>)> {
    let max = s.len();
    let (mut num_samples, mut loop_start, mut loop_end) = (0u64, 0u64, 0u64);
    let (mut start_found, mut end_found) = (false, false);
    let mut frames = 0u64;
    let mut pos = 0u64;
    let chunk = 0x10000u64;
    let mut buf = Vec::new();
    let mut buf_at = 0u64;
    'outer: while pos < max {
        if pos + 0x20 > buf_at + buf.len() as u64 {
            buf_at = pos;
            buf = s.bytes(pos, (chunk + 0x20) as usize)?;
        }
        let f = &buf[(pos - buf_at) as usize..];
        let flag = f[1] & 0x0f;
        frames += 1;
        if flag == 0x06 && !start_found {
            loop_start = num_samples;
            start_found = true;
        }
        if flag == 0x03 && loop_end == 0 {
            loop_end = num_samples + 28;
            end_found = true;
            // Commandos (PS2) has many loop starts and ends.
            if pos + 0x10 < max && f[0x11] & 0x0f == 0x06 {
                loop_end = 0;
                end_found = false;
            }
            if start_found && end_found {
                break 'outer;
            }
        }
        num_samples += 28;
        pos += 0x10;
        // Stream done.
        if flag & 0x01 != 0 {
            frames += 1;
            break;
        }
    }
    let loops = (start_found && end_found).then_some((loop_start, loop_end));
    Ok((frames * 0x10, loops))
}
