//! VGS - from Harmonix games (vgmstream meta/vgs.c): "VgS!" multistream files [Guitar
//! Hero II (PS2), Guitar Hero Encore: Rocks the 80s (PS2)], and the older headerless-ish
//! .vgs [Karaoke Revolution (PS2), EyeToy: AntiGrav (PS2)], found by extension.

use std::io;

use super::ps2p::vgm_interleaved;
use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "VGS",
    magics: &[b"VgS!"],
    magic_at: 0,
    exts: &["vgs"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if ctx.is(off, b"VgS!")? {
        return parse_new(ctx, off);
    }
    if off == 0 && ctx.ext() == "vgs" {
        return parse_old(ctx);
    }
    Ok(vec![])
}

/// "VgS!": up to 8 streams (channels), one PS-ADPCM frame of each in turn; frames of
/// streams at another rate are skipped (their flag byte has their number).
fn parse_new(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x80)?;
    let (mut rate, mut chan_size, mut channels) = (0i32, 0u32, 0u64);
    for i in 0..8 {
        let r = le32(&h, 0x08 + 8 * i) as i32;
        let frames = le32(&h, 0x0c + 8 * i);
        let size = frames.wrapping_mul(0x10);
        if r == 0 {
            break;
        }
        if rate == 0 || chan_size == 0 {
            rate = r;
            chan_size = size;
        }
        if chan_size.wrapping_sub(0x10) == size {
            chan_size -= 0x10;
        }
        if rate != r {
            break;
        }
        channels += 1;
    }
    let samples = psx::bytes_to_samples(chan_size as u64, 1);
    if channels == 0 || rate <= 0 || !sane_rate(rate as u32) || samples == 0 {
        return Ok(vec![]);
    }
    let file_size = ctx.size();
    let frames = chan_size as u64 / 0x10;
    let block = 0x10 * channels;
    let mut pieces: Vec<(u64, u64)> = Vec::new();
    let mut b = off + 0x80;
    let mut buf = Vec::new();
    let mut buf_off = 0u64;
    for n in 0..frames {
        if b + block > file_size {
            return Ok(vec![]);
        }
        match pieces.last_mut() {
            Some(l) if l.0 + l.1 == b => l.1 += block,
            _ => pieces.push((b, block)),
        }
        b += block;
        if n + 1 == frames {
            break;
        }
        // Skip frames of other streams.
        loop {
            if b >= file_size {
                break;
            }
            if b < buf_off || b + 2 > buf_off + buf.len() as u64 {
                buf_off = b;
                buf = ctx.bytes(b, 0x10000)?;
            }
            if buf[(b - buf_off) as usize + 1] & 0x0f == 0 {
                break;
            }
            b += 0x10;
        }
    }
    // Every frame must be PS-ADPCM (the flag byte is ignored: it holds the stream number).
    let probe = ctx.bytes(off + 0x80, 0x200.min((b - off - 0x80) as usize))?;
    if probe.chunks_exact(16).any(|f| f[0] >> 4 > 4 || f[0] & 0x0f > 12) {
        return Ok(vec![]);
    }
    let data = Data::blocks(ctx.entry, pieces);
    let codec = Codec::Psx(psx::Params { interleave: 0x10, badflags: true, ..Default::default() });
    let t = Track::new(ctx.entry, off, "VGS", channels as u16, rate as u32, samples, data, codec);
    Ok(vec![Found::new(t, b)])
}

/// Old .vgs: channels, rate, frame count, then interleaved PS-ADPCM to the end of the file.
fn parse_old(ctx: &mut Ctx) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(0, 0x10)?;
    let channels = le32(&h, 0) as i32;
    let rate = le32(&h, 4) as i32;
    let frame_count = le32(&h, 8) as u64;
    if !(1..=4).contains(&channels) || ctx.size() < 0x10 {
        return Ok(vec![]);
    }
    let channels = channels as u64;
    let stream_size = ctx.size() - 0x10;
    let samples = psx::bytes_to_samples(stream_size, channels as u16);
    if frame_count * channels * 0x10 > stream_size || rate <= 0 || !sane_rate(rate as u32) || samples == 0 {
        return Ok(vec![]);
    }
    if !psx::plausible(&ctx.bytes(0x10, 0x100.min(stream_size as usize))?) {
        return Ok(vec![]);
    }
    let data = vgm_interleaved(ctx.entry, 0x10, stream_size, channels, 0x2000, ctx.size());
    let t = Track::new(ctx.entry, 0, "VGS", channels as u16, rate as u32, samples, data, Codec::Psx(psx::Params::interleaved(0x2000)));
    Ok(vec![Found::new(t, ctx.size())])
}
