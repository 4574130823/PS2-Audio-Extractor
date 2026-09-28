//! SDF - from Beyond Reality games [Agent Hugo: Lemoon Twist (PS2), Crazy Golf World Tour
//! (PS2)] (vgmstream meta/sdf.c). Also its NDS PCM variant; the NDS IMA and Wii/3DS DSP
//! variants are listed with a note (codecs this app doesn't have).

use std::io;

use super::ps2p::{vgm_interleaved, vgm_loop};
use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, pcm, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "SDF",
    magics: &[b"SDF\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x30)?;
    if &h[0..4] != b"SDF\0" || le32(&h, 4) != 3 {
        return Ok(vec![]);
    }
    let mut data_size = le32(&h, 8) as u64;
    let size = ctx.size();
    // The header's size is whatever's left before the data: known for a standalone file;
    // inside something else, the 0x18 kinds: PS2 (channel count at 0x10) and NDS (a
    // channel count byte at 0x15, where the DSP kinds' 32-bit channel count has a zero).
    let header = if off == 0 {
        match size.checked_sub(data_size) {
            Some(s) => s,
            None => return Ok(vec![]),
        }
    } else if le32(&h, 0x10) <= 6 || (h[0x14] <= 2 && h[0x15] != 0) {
        0x18
    } else {
        return Ok(vec![]);
    };
    let start = off + header;
    if data_size == 0 || start + data_size > size {
        return Ok(vec![]);
    }
    enum Kind {
        Psx,
        Pcm16,
        Pcm8,
        Other(&'static str),
    }
    let (rate, channels, interleave, kind) = match header {
        0x18 if le32(&h, 0x10) > 6 => {
            let kind = match h[0x14] {
                0 => Kind::Pcm8,
                1 => Kind::Pcm16,
                2 => Kind::Other("IMA ADPCM"),
                _ => return Ok(vec![]),
            };
            (le32(&h, 0x10) as i32, h[0x15] as u64, le16(&h, 0x16) as u64, kind)
        }
        0x18 => (le32(&h, 0x0c) as i32, le32(&h, 0x10) as u64, le32(&h, 0x14) as u64, Kind::Psx),
        0x78 => (le32(&h, 0x10) as i32, le32(&h, 0x14) as u64, le32(&h, 0x18) as u64, Kind::Other("NGC DSP")),
        0x84 => {
            data_size = le32(&h, 0x20) as u64;
            (le32(&h, 0x10) as i32, le32(&h, 0x14) as u64, le32(&h, 0x18) as u64, Kind::Other("NGC DSP"))
        }
        _ => return Ok(vec![]),
    };
    if rate <= 0 || !sane_rate(rate as u32) || !(1..=8).contains(&channels) || (channels > 1 && interleave == 0 && !matches!(kind, Kind::Other(_))) {
        return Ok(vec![]);
    }
    let ch = channels as u16;
    let (samples, data, codec, note) = match kind {
        Kind::Psx => {
            if !psx::plausible(&ctx.bytes(start, 0x100.min(data_size as usize))?) {
                return Ok(vec![]);
            }
            // (a short last row is split evenly, as interleave_last_block_size does)
            (psx::bytes_to_samples(data_size, ch), Data::at(ctx.entry, start, data_size), Codec::Psx(psx::Params::interleaved(interleave)), None)
        }
        Kind::Pcm16 => (
            pcm::bytes_to_samples(data_size, ch, 16),
            vgm_interleaved(ctx.entry, start, data_size, channels, interleave, size),
            Codec::Pcm(pcm::Params::le16(interleave)),
            None,
        ),
        Kind::Pcm8 => (
            pcm::bytes_to_samples(data_size, ch, 8),
            vgm_interleaved(ctx.entry, start, data_size, channels, interleave, size),
            Codec::Pcm(pcm::Params::s8(interleave)),
            None,
        ),
        Kind::Other(name) => {
            if name == "IMA ADPCM" && channels != 1 {
                return Ok(vec![]);
            }
            let samples = if name == "IMA ADPCM" { (data_size - 4) * 2 } else { data_size / channels / 8 * 14 };
            (samples, Data::at(ctx.entry, start, data_size), Codec::None, Some(format!("{name} audio (not supported)")))
        }
    };
    if samples == 0 {
        return Ok(vec![]);
    }
    let mut t = Track::new(ctx.entry, off, "SDF", ch, rate as u32, samples, data, codec);
    t.note = note;
    // Songs simply repeat; short ones don't loop.
    if samples > 10 * rate as u64 {
        t = vgm_loop(t, 0, samples as i64);
    }
    Ok(vec![Found::new(t, start + data_size)])
}
