//! .WD - Square wave banks [Final Fantasy XI (PS2), Final Fantasy X-2 (PS2/Vita), FF Crystal
//! Chronicles (GC)] (vgmstream meta/wd.c): a table of instruments, then wave headers
//! (0x20 bytes, little endian: PS-ADPCM; 0x60, big endian: GameCube DSP), then the data.

use std::io;

use super::{Ctx, Found, Parser, be32, le32};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "WD",
    magics: &[b"WD"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

/// Square's key (8.24 semitones) to a sample rate.
fn key_to_rate(key: i32, base: i32) -> i32 {
    let r = (base as f64 * 2f64.powf(key as f64 / 16777216.0 / 12.0)).round() as i32;
    r.min(base)
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x24)?;
    if &h[0..2] != b"WD" || h[0x14..0x20].iter().any(|&b| b != 0) {
        return Ok(vec![]);
    }
    let big = le32(&h, 0x04) > be32(&h, 0x04);
    let r32 = |b: &[u8], at: usize| if big { be32(b, at) } else { le32(b, at) };
    let data_size = r32(&h, 0x04) as u64;
    let instruments = r32(&h, 0x08) as i32;
    let waves = r32(&h, 0x0c) as i32;
    if instruments < 0 || instruments > waves || !(1..=0x200).contains(&waves) {
        return Ok(vec![]);
    }
    let waves = waves as u64;
    let entry = if big { 0x60 } else { 0x20 };
    let waves_offset = r32(&h, 0x20) as u64;
    let data_offset = waves_offset + waves * entry;
    if waves_offset < 0x24 || data_offset > 0x10000 || data_size == 0 {
        return Ok(vec![]);
    }
    // The end of the data: the file's end for standalone banks (like vgmstream), else
    // what the header says.
    let standalone = off == 0 && ctx.ext() == "wd";
    let file_size = if standalone { ctx.size() } else { data_offset + data_size };
    if off + data_offset + data_size > ctx.size() || (standalone && data_size < 0x40 && data_offset + data_size + 0x100 < file_size) {
        return Ok(vec![]);
    }
    let table = ctx.bytes(off + waves_offset, (waves * entry) as usize)?;
    if !big && !psx::plausible(&ctx.bytes(off + data_offset, 0x100.min(data_size) as usize)?) {
        return Ok(vec![]);
    }
    let mut found = Vec::new();
    for i in 0..waves as usize {
        let e = &table[i * entry as usize..(i + 1) * entry as usize];
        let (stream_offset, stream_size, rate) = if big {
            let so = be32(e, 0x04) as u64 + data_offset;
            (so, be32(e, 0x10) as u64, key_to_rate(be32(e, 0x14) as i32, 32000))
        } else {
            let mut so = le32(e, 0x04);
            if !so.is_multiple_of(0x10) {
                so &= 0xFFFF_FF00; // FF XI: all offsets add 0x0C
            }
            let so = so.wrapping_add(data_offset as u32);
            // No sizes: up to the next offset. (vgmstream compares the other waves'
            // relative offsets with this absolute one; kept as is.)
            let mut next = file_size as u32;
            for k in 0..waves as usize {
                let t = le32(&table, k * entry as usize + 0x04);
                if t > so && t < next {
                    next = t & 0xFFFF_FF00;
                }
            }
            (so as u64, next.wrapping_sub(so) as u64, key_to_rate(le32(e, 0x10) as i32, 48000))
        };
        let samples = if big { stream_size / 8 * 14 } else { psx::bytes_to_samples(stream_size, 1) } as i32;
        if samples <= 0 || rate <= 0 || rate > 96000 || stream_offset + stream_size > file_size {
            continue;
        }
        let data = Data::at(ctx.entry, off + stream_offset, stream_size);
        let t = if big {
            let mut t = Track::new(ctx.entry, off, "WD", 1, rate as u32, samples as u64, data, Codec::None);
            t.note = Some("GameCube DSP ADPCM audio isn't supported".into());
            t
        } else {
            Track::new(ctx.entry, off, "WD", 1, rate as u32, samples as u64, data, Codec::Psx(psx::Params::default()))
        };
        found.push(Found::new(t, off + file_size));
    }
    Ok(found)
}
