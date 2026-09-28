//! A2M - from Artificial Mind & Movement games [Scooby-Doo! Unmasked (PS2)] (vgmstream
//! meta/a2m.c).
//!
//! Also holds small helpers the simple PS-ADPCM header formats share (sizes only the file's
//! end gives, row-rounded interleaved sizes, vgmstream's loop sanity rule, blocked data).

use std::io;

use super::{Ctx, Found, Parser, be32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "A2M",
    magics: &[b"A2M\0PS2\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

const EXTS: &[&str] = &["int"];

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x30)?;
    if &h[0..8] != b"A2M\0PS2\0" {
        return Ok(vec![]);
    }
    let rate = be32(&h, 0x10);
    let (start, channels, interleave) = (off + 0x30, 2u16, 0x6000u64);
    if !sane_rate(rate) || !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let end = psx_end(ctx, off, start, channels, interleave, EXTS)?;
    let size = end.saturating_sub(start);
    let samples = psx::bytes_to_samples(size, channels);
    if samples == 0 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, start, rows(size.div_ceil(channels as u64), interleave, channels));
    let t = Track::new(ctx.entry, off, "A2M", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, end)])
}

// ---------------------------------------------------------------------------------------
// Shared helpers.

/// Whether PS-ADPCM data starts at `start`: inside the file, and plausible.
pub fn psx_start_ok(ctx: &mut Ctx, start: u64) -> io::Result<bool> {
    if start >= ctx.size() {
        return Ok(false);
    }
    let n = 0x100.min(ctx.size() - start) as usize;
    Ok(psx::plausible(&ctx.bytes(start, n)?))
}

/// End of PS-ADPCM data whose size only the file's size gives (vgmstream takes everything
/// to the end of the file). For a standalone file (header at 0, named with one of the
/// format's extensions) that's the file's end. Anywhere else the data is followed in rows
/// (`interleave` bytes per channel) while every frame is plausible, up to the row where
/// the sound ends (an end flag, 0x01 or 0x07), plus any silent / 0x07 rows after it.
pub fn psx_end(ctx: &mut Ctx, off: u64, start: u64, channels: u16, interleave: u64, exts: &[&str]) -> io::Result<u64> {
    let size = ctx.size();
    if off == 0 && exts.contains(&ctx.ext().as_str()) {
        return Ok(size);
    }
    let row = if channels <= 1 { 0x10 } else { interleave * channels as u64 };
    if row == 0 || row % 0x10 != 0 || interleave % 0x10 != 0 && channels > 1 {
        return Ok(start);
    }
    let per_read = (0x10000 / row).max(1) * row;
    let mut pos = start;
    let mut ended = false;
    while pos + row <= size {
        let n = per_read.min((size - pos) / row * row);
        let buf = ctx.bytes(pos, n as usize)?;
        for r in buf.chunks_exact(row as usize) {
            let frames = || r.chunks_exact(16);
            if frames().any(|f| f[0] >> 4 > 4 || f[0] & 0x0f > 12 || f[1] > 7) {
                return Ok(pos);
            }
            if ended {
                if !frames().all(|f| f[1] == 0x07 || f.iter().all(|&b| b == 0)) {
                    return Ok(pos);
                }
            } else if frames().any(|f| f[1] == 0x01 || f[1] == 0x07) {
                ended = true;
            }
            pos += row;
        }
    }
    Ok(pos)
}

/// Bytes of interleaved data holding `per_channel` bytes per channel, in whole rows (the
/// way vgmstream reads a last, partial block: at full size).
pub fn rows(per_channel: u64, interleave: u64, channels: u16) -> u64 {
    if channels <= 1 || interleave == 0 {
        return per_channel * channels.max(1) as u64;
    }
    per_channel.div_ceil(interleave) * interleave * channels as u64
}

/// Sets loop points the way vgmstream accepts them: dropped unless
/// 0 <= start < end <= samples.
pub fn vgm_loop(t: Track, flag: bool, start: i64, end: i64) -> Track {
    if !flag || start < 0 || end <= start || end as u64 > t.samples {
        return t;
    }
    t.looped(start as u64, end as u64)
}

/// PS-ADPCM spread over blocks, as vgmstream's blocked layouts read it: per block, the
/// offset of channel 0's data, the distance between channels' data and the bytes per
/// channel (whole frames are decoded). Returns the data (cut into pieces that alternate
/// channels) and the interleave to decode it with.
pub fn psx_blocks(entry: usize, blocks: &[(u64, u64, u64)], channels: u16) -> (Data, u64) {
    let ch = channels.max(1) as u64;
    if ch == 1 {
        let pieces = blocks.iter().map(|&(o, _, n)| (o, n / 16 * 16)).filter(|p| p.1 > 0).collect();
        return (Data::blocks(entry, pieces), 0x10);
    }
    // Pieces of one common size (the gcd of the block sizes) keep the channels in turn.
    let mut il = 0u64;
    for &(_, _, n) in blocks {
        let n = n / 16 * 16;
        if n > 0 {
            il = gcd(il, n);
        }
    }
    let il = il.max(16);
    let mut pieces = Vec::new();
    for &(o, stride, n) in blocks {
        let n = n / 16 * 16;
        let mut p = 0;
        while p < n {
            for c in 0..ch {
                pieces.push((o + c * stride + p, il));
            }
            p += il;
        }
    }
    (Data::blocks(entry, pieces), il)
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// SPU2 pitch to sample rate, rounded to tens (vgmstream's spu2_pitch_to_sample_rate_rounded).
pub fn spu2_rate_rounded(pitch: i32) -> i64 {
    let v = (48000i64 * pitch as i64) / 4096;
    let r = v % 10;
    if r < 5 { v - r } else { v + (10 - r) }
}
