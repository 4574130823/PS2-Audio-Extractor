//! .vig - from Konami/KCE Studio games [Pop'n Music 11~14 (PS2), Dance Dance Revolution
//! SuperNova/X (PS2)] (vgmstream meta/vig_kces.c): interleaved PS-ADPCM, header like GbTs.
//!
//! Later games encrypt the data (each frame's header byte XORed, its first data byte
//! shifted) [beatmaniaIIDX 14 GOLD (PS2), beatmaniaIIDX 16 (PS2)]; the PS-ADPCM codec can't
//! undo that yet, so those tracks are listed but not decoded.

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "VIG (KCES)",
    magics: &[b"\x01\x00\x64\x08"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x30)?;
    if h[0..4] != [0x01, 0x00, 0x64, 0x08] || le32(&h, 0x04) != 0 {
        return Ok(vec![]);
    }
    let data_offset = le32(&h, 0x08) as u64;
    let data_size = le32(&h, 0x0c) as u64; // without padding
    let loop_start = le32(&h, 0x10);
    let loop_end = le32(&h, 0x14);
    let rate = le32(&h, 0x18);
    let channels = le32(&h, 0x1c);
    let flags = le32(&h, 0x20);
    let interleave = le32(&h, 0x24) as u64; // 0 for mono
    if !(1..=8).contains(&channels) || !sane_rate(rate) || data_offset < 0x28 {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    if channels > 1 && (interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000) {
        return Ok(vec![]);
    }
    let start = off + data_offset;
    // Whole blocks are read, even in the last row.
    let size = if channels > 1 { data_size.next_multiple_of(interleave * channels as u64) } else { data_size };
    if data_size == 0 || start + size > ctx.size() {
        return Ok(vec![]);
    }
    let encrypted = flags == 1;
    let mut probe = ctx.bytes(start, 0x100.min(size as usize))?;
    // Encrypted: XOR on each frame's first byte, ADD on its third (keys from the first,
    // null frame), as vgmstream's meta/vig_kces_streamfile.h.
    let key = encrypted.then(|| (probe[0], (!probe.get(2).copied().unwrap_or(0)).wrapping_add(1)));
    if let Some((xor, add)) = key {
        for f in probe.chunks_mut(16) {
            f[0] ^= xor;
            if f.len() > 2 {
                f[2] = f[2].wrapping_add(add);
            }
        }
    }
    if !psx::plausible(&probe) {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(data_size, channels);
    if samples == 0 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, start, size);
    let codec = Codec::Psx(psx::Params { interleave, vig_key: key, ..Default::default() });
    let mut t = Track::new(ctx.entry, off, "VIG", channels, rate, samples, data, codec);
    if loop_end > 0 {
        // The loop region matches the PS-ADPCM flags.
        let ls = psx::bytes_to_samples(loop_start as u64, channels);
        let le = psx::bytes_to_samples(loop_end.wrapping_add(loop_start) as u64, channels);
        if ls < le && le <= samples {
            t = t.looped(ls, le);
        }
    }
    Ok(vec![Found::new(t, start + size)])
}
