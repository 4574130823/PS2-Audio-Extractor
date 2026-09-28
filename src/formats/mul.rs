//! .MUL - from Crystal Dynamics games [Legacy of Kain: Defiance (PS2), Tomb Raider Legend /
//! Anniversary / Underworld (multi)] (vgmstream meta/mul.c + layout/blocked_mul.c).
//!
//! No signature (known by extension): a header of little or big endian values, then blocks
//! (audio or not) from 0x800 or 0x2000, each audio block holding every channel's data one
//! after the other. The codec isn't stored; it's guessed from the first audio block like
//! vgmstream does.

use std::io;

use super::{Ctx, Found, Parser};
use crate::codecs::{Codec, ima, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "MUL",
    magics: &[],
    magic_at: 0,
    exts: &["mul", "emff"],
    locate: None,
    parse,
};

#[derive(PartialEq, Eq)]
enum Guess {
    Psx,
    Ima,
    Other(&'static str),
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 || ctx.size() < 0x800 {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, 0x40)?;
    if h[0x10..0x20].iter().any(|&b| b != 0) {
        return Ok(vec![]);
    }
    let (le, be) = (u32::from_le_bytes(h[0..4].try_into().unwrap()), u32::from_be_bytes(h[0..4].try_into().unwrap()));
    let big = le > be;
    let r32 = |b: &[u8], at: usize| {
        let v: [u8; 4] = b[at..at + 4].try_into().unwrap();
        if big { u32::from_be_bytes(v) } else { u32::from_le_bytes(v) }
    };
    let rate = r32(&h, 0x00);
    let loop_start = r32(&h, 0x04) as i32;
    let samples = r32(&h, 0x08) as i32;
    let channels = r32(&h, 0x0c);
    if !(8000..=48000).contains(&rate) || !(1..=8).contains(&channels) || samples <= 0 {
        return Ok(vec![]);
    }
    let check1 = f32::from_bits(r32(&h, 0x38));
    let check2 = f32::from_bits(r32(&h, 0x3c));
    if !(1.0..=3000.0).contains(&check1) && check2 != 1.0 {
        return Ok(vec![]);
    }
    let size = ctx.size();
    let start = {
        let t = |ctx: &mut Ctx, at: u64| -> io::Result<bool> {
            let (a, b) = (read32(ctx, at, true)?, read32(ctx, at + 4, true)?);
            Ok((a != 0 && a != u32::MAX) || (b != 0 && b != u32::MAX))
        };
        if t(ctx, 0x800)? {
            0x800
        } else if t(ctx, 0x2000)? {
            0x2000
        } else {
            return Ok(vec![]);
        }
    };
    let ch = channels as u64;
    let guess = guess_codec(ctx, start, big, ch)?;
    let Some(guess) = guess else { return Ok(vec![]) };

    let (frame, fsamples) = match guess {
        Guess::Psx => (16u64, 28u64),
        Guess::Ima => (0x24, 64),
        Guess::Other(_) => (0, 0),
    };
    let mut t = if let Guess::Other(what) = guess {
        let mut t = Track::new(ctx.entry, 0, "MUL", channels as u16, rate, samples as u64, Data::at(ctx.entry, start, size - start), Codec::None);
        t.note = Some(format!("{what} audio isn't supported"));
        t
    } else {
        // Each block's channel data, frame by frame across the channels (one frame of
        // interleave), so the whole stream decodes as plain interleaved data.
        let mut pieces: Vec<(u64, u64)> = Vec::new();
        let mut push = |at: u64, len: u64| match pieces.last_mut() {
            Some(last) if last.0 + last.1 == at => last.1 += len,
            _ => pieces.push((at, len)),
        };
        let mut block = start;
        let mut got = 0u64;
        while block + 0x10 <= size && got < samples as u64 {
            let bh = ctx.bytes(block, 0x14)?;
            let kind = r32(&bh, 0);
            let block_size = r32(&bh, 4) as u64;
            if (kind as i32) < 0 {
                break;
            }
            if kind == 0 && block_size != 0 {
                let data_size = r32(&bh, 0x10) as u64;
                let per_ch = data_size / ch;
                let frames = per_ch / frame;
                let base = block + 0x20;
                for f in 0..frames {
                    for c in 0..ch {
                        push(base + per_ch * c + f * frame, frame);
                    }
                }
                got += frames * fsamples;
            }
            block += 0x10 + block_size;
        }
        if pieces.is_empty() {
            return Ok(vec![]);
        }
        let codec = if guess == Guess::Psx {
            Codec::Psx(psx::Params::interleaved(16))
        } else {
            Codec::Ima(ima::Params::new(ima::Kind::Cd, 0x24))
        };
        Track::new(ctx.entry, 0, "MUL", channels as u16, rate, samples as u64, Data::blocks(ctx.entry, pieces), codec)
    };
    if loop_start >= 0 && loop_start < samples {
        t = t.looped(loop_start as u64, samples as u64);
    }
    Ok(vec![Found::new(t, size)])
}

fn read32(ctx: &mut Ctx, at: u64, big: bool) -> io::Result<u32> {
    if at + 4 > ctx.size() {
        return Ok(u32::MAX);
    }
    Ok(if big { ctx.u32be(at)? } else { ctx.u32le(at)? })
}

/// vgmstream's `guess_codec`: known DSP coefficient spots on big endian platforms, then
/// the first audio block's contents.
fn guess_codec(ctx: &mut Ctx, start: u64, big: bool, ch: u64) -> io::Result<Option<Guess>> {
    if big {
        for at in [0xc8, 0xcc, 0x2d0] {
            if read32(ctx, at, true)? != 0 {
                return Ok(Some(Guess::Other("GameCube/Wii DSP ADPCM")));
            }
        }
    }
    let size = ctx.size();
    let mut off = start;
    while off < size {
        let kind = read32(ctx, off, big)?;
        let block_size = read32(ctx, off + 4, big)?;
        let data_size = read32(ctx, off + 0x10, big)?;
        if kind == u32::MAX || block_size == u32::MAX || data_size == u32::MAX {
            return Ok(None);
        }
        if kind != 0 {
            off += 0x10 + block_size as u64;
            continue;
        }
        if ctx.is(off + 0x10, b"FSB4")? || ctx.is(off + 0x20, b"FSB4")? {
            return Ok(Some(Guess::Other("FSB (MPEG)")));
        }
        if block_size as u64 == 0x810 * ch {
            let mut all = true;
            for i in 0..ch {
                let t = off + 0x10 + 0x810 * i;
                if read32(ctx, t, big)? != 0x800 || read32(ctx, t + 0x10, big)? != 0x0800_0000 {
                    all = false;
                    break;
                }
            }
            if all {
                return Ok(Some(Guess::Other("XMA")));
            }
        }
        let data = ctx.bytes(off + 0x20, data_size.min(0x100_0000) as usize)?;
        let byte = |at: u64| if off + 0x20 + at < size { data.get(at as usize).copied().unwrap_or(0xff) } else { 0xff };
        if (0..data_size as u64 / 0x10).all(|i| byte(0x10 * i + 1) == 0x02) {
            return Ok(Some(Guess::Psx));
        }
        if (0..data_size as u64 / 0x24).all(|i| byte(0x24 * i + 3) == 0x00) {
            return Ok(Some(Guess::Ima));
        }
        break;
    }
    Ok(None)
}
