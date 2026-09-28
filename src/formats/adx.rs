//! CRI ADX (vgmstream meta/adx.c), found by its "(c)CRI" signature, which sits right
//! before the audio data. Versions 3, 4 (with AINF/CINF info) and 5, and encrypted ADX
//! (types 8 and 9) with vgmstream's key list, key derivation and key detection.

use std::io;

use super::vag::vgm_loop;
use super::{Ctx, Found, Parser, be16, be32};
use crate::codecs::{Codec, adx};
use crate::track::{Data, Track};

#[path = "adx_keys.rs"]
mod keys;

pub const PARSER: Parser = Parser {
    name: "ADX",
    magics: &[b"(c)CRI"],
    magic_at: 0,
    exts: &[],
    locate: Some(locate),
    parse,
};

/// ADX headers start with 0x8000 and the offset of the "(c)CRI" signature: find the header
/// a signature at `sig` belongs to.
fn locate(ctx: &mut Ctx, sig: u64) -> io::Result<Option<u64>> {
    let from = sig.saturating_sub(0x1000);
    let window = ctx.bytes(from, (sig - from) as usize)?;
    for j in (0..window.len().saturating_sub(3)).rev() {
        if window[j] == 0x80 && window[j + 1] == 0x00 {
            let h = from + j as u64;
            let copyright = u16::from_be_bytes([window[j + 2], window[j + 3]]) as u64;
            if h + copyright + 4 - 6 == sig {
                return Ok(Some(h));
            }
        }
    }
    Ok(None)
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let limit = ctx.size() - off;
    Ok(parse_at(ctx, off, limit, 0)?.into_iter().collect())
}

/// An ADX header at `off` whose (sub)file is `file_size` bytes: vgmstream's
/// `init_vgmstream_adx_subkey`.
pub(crate) fn parse_at(ctx: &mut Ctx, off: u64, file_size: u64, subkey: u16) -> io::Result<Option<Found>> {
    let h = ctx.bytes(off, 0x14)?;
    if be16(&h, 0) != 0x8000 {
        return Ok(None);
    }
    let start = be16(&h, 2) as u64 + 4;
    let encoding = h[4];
    let (frame, bits, channels) = (h[5], h[6], h[7] as u16);
    let rate = be32(&h, 0x08);
    let samples = be32(&h, 0x0c) as i32;
    let cutoff = be16(&h, 0x10);
    let mut version = be16(&h, 0x12);
    if !matches!(encoding, 2..=4) || frame != 0x12 || bits != 4 || !(1..=8).contains(&channels) || !(4000..=96000).contains(&rate) || samples <= 0 {
        return Ok(None);
    }
    if start < 0x14 || start > file_size {
        return Ok(None);
    }
    let encrypted = match version {
        0x0408 => Some(8u8),
        0x0409 => Some(9u8),
        _ => None,
    };
    if encrypted.is_some() {
        version = 0x0400;
    }
    if !matches!(version, 0x0300 | 0x0400 | 0x0500) {
        return Ok(None);
    }
    if !ctx.is(off + start - 6, b"(c)CRI")? {
        return Ok(None);
    }
    let header = ctx.bytes(off, start as usize)?;
    let s16 = |at: u64| header.get(at as usize..at as usize + 2).map(|b| i16::from_be_bytes([b[0], b[1]]) as i32).unwrap_or(0);
    let s32 = |at: u64| header.get(at as usize..at as usize + 4).map(|b| i32::from_be_bytes(b.try_into().unwrap())).unwrap_or(0);

    let mut loops = None;
    let mut hist = vec![];
    match version {
        0x0300 => {
            // early ADX: no history, loops if there's room
            if start - 6 >= 0x14 + 0x18 {
                loops = Some(0x14);
            }
        }
        0x0400 => {
            let hist_size = if channels > 1 { 4 * channels as u64 } else { 8 };
            hist = (0..channels as u64).map(|c| (s16(0x18 + c * 4), s16(0x1a + c * 4))).collect();
            let ainf = 0x18 + hist_size + 4;
            let ainf_size = if header.get(ainf as usize..ainf as usize + 4) == Some(b"AINF") { s32(ainf + 4) as u32 as u64 } else { 0 };
            // (unsigned math, like vgmstream: a huge AINF size means no loops)
            if start.wrapping_sub(ainf_size).wrapping_sub(6) >= 0x18 + hist_size + 0x18 && start >= ainf_size + 6 {
                loops = Some(0x18 + hist_size);
            }
        }
        _ => {} // v5 [Buggy Heat SFD]: no history or loops
    }

    let data_off = off + start;
    let frames = (samples as u64).div_ceil(32);
    let size = frames * 18 * channels as u64;
    if data_off >= ctx.size() {
        return Ok(None);
    }
    let mut key = None;
    let mut note = None;
    if let Some(kind) = encrypted {
        match find_key(ctx, data_off, channels, samples, kind, subkey)? {
            Some(k) => key = Some(k),
            None => note = Some("Encrypted ADX with an unknown key".to_string()),
        }
    }
    if encoding == 2 {
        note = Some("ADX with fixed coefficients (encoding 2) isn't supported".to_string());
    }
    let data = Data::at(ctx.entry, data_off, size);
    let mut t = Track::new(ctx.entry, off, "ADX", channels, rate, samples as u64, data, Codec::Adx(adx::Params {
        v3: version == 0x0300,
        exponential: encoding == 4,
        coef: adx::coefs(cutoff, rate),
        hist,
        key,
        interleave: 0x12,
    }));
    if note.is_some() {
        t.codec = Codec::None;
        t.note = note;
    }
    if let Some(l) = loops {
        if s32(l + 4) != 0 {
            t = vgm_loop(t, s32(l + 8) as i64, s32(l + 0x10) as i64);
        }
    }
    let end = (data_off + size).min(off + file_size).min(ctx.size());
    Ok(Some(Found::new(t, end)))
}

// ------------------------------------------------------------------------ encryption

/// vgmstream's `cri_key8_derive`: XOR parameters from a type 8 keystring.
fn key8_derive(key: &[u8]) -> (u16, u16, u16) {
    let p = &keys::KEY8_PRIMES;
    if key.is_empty() {
        return (0, 0, 0);
    }
    let (mut k1, mut k2, mut k3) = (p[0x100], p[0x200], p[0x300]);
    for &c in key {
        let m = p[(c as i8 as i32 + 0x80) as usize] as u32;
        k1 = p[(k1 as u32 * m % 0x400) as usize];
        k2 = p[(k2 as u32 * m % 0x400) as usize];
        k3 = p[(k3 as u32 * m % 0x400) as usize];
    }
    (k1, k2, k3)
}

/// vgmstream's `cri_key9_derive`: XOR parameters from a type 9 keycode (and subkey).
fn key9_derive(mut key: u64, subkey: u16) -> (u16, u16, u16) {
    if key == 0 {
        return (0, 0, 0);
    }
    if subkey != 0 {
        key = key.wrapping_mul(((subkey as u64) << 16) | ((!subkey) as u64 + 2));
    }
    key = key.wrapping_sub(1);
    (((key >> 27) & 0x7fff) as u16, (((key >> 12) & 0x7ffc) | 1) as u16, (((key << 1) & 0x7fff) | 1) as u16)
}

/// A key file next to the ADX ("NAME.ADXkey" or ".adxkey"), like vgmstream's
/// `read_external_key`.
fn external_key(ctx: &mut Ctx, kind: u8, subkey: u16) -> Option<(u16, u16, u16)> {
    let path = ctx.path().to_string();
    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
    let dot = name.rfind('.').map(|p| name[p..].to_string()).unwrap_or_default();
    let mut tries = vec![format!("{name}key")];
    if !dot.is_empty() {
        tries.push(format!("{dot}key"));
    }
    for t in tries {
        let Some((_, mut r)) = ctx.sibling_named(&t) else { continue };
        if r.size == 0 || r.size > 0x40 {
            continue;
        }
        let buf = r.bytes(0, r.size as usize).ok()?;
        let is_string = match kind {
            8 => buf.iter().all(|&b| (0x20..=0x8f).contains(&b)),
            _ => buf.len() <= 20 && buf.iter().all(|b| b.is_ascii_digit()),
        };
        if buf.len() == 6 && !is_string {
            return Some((be16(&buf, 0), be16(&buf, 2), be16(&buf, 4)));
        }
        if kind == 8 && is_string {
            return Some(key8_derive(&buf));
        }
        if kind == 9 && is_string {
            let code = std::str::from_utf8(&buf).ok()?.parse::<u64>().unwrap_or(u64::MAX);
            return Some(key9_derive(code, subkey));
        }
        if kind == 9 && buf.len() == 8 {
            return Some(key9_derive(u64::from_be_bytes(buf[..8].try_into().unwrap()), subkey));
        }
        if kind == 9 && buf.len() == 10 {
            return Some(key9_derive(u64::from_be_bytes(buf[..8].try_into().unwrap()), be16(&buf, 8)));
        }
        return None;
    }
    None
}

const MIN_TEST_FRAMES: usize = 128;
const BLANK: u16 = 0xFFFF;

/// vgmstream's `find_adx_key`: a key file, else the first listed key whose XOR sequence
/// matches the unused high bits of the first frames' scales.
fn find_key(ctx: &mut Ctx, data_off: u64, channels: u16, samples: i32, kind: u8, subkey: u16) -> io::Result<Option<(u16, u16, u16)>> {
    if let Some(k) = external_key(ctx, kind, subkey) {
        return Ok(Some(k));
    }
    // scales of the first frames, from the first non-blank one
    let frame_count = (samples as u64).div_ceil(32) * channels as u64;
    let (mut start, mut scales, mut valid) = (0usize, Vec::new(), 0usize);
    let mut pos = data_off;
    let mut i = 0u64;
    while i < frame_count {
        let buf = ctx.bytes(pos, 0x1FFE)?;
        pos += 0x1FFE;
        for f in buf.chunks_exact(0x12) {
            if i >= frame_count {
                break;
            }
            i += 1;
            let blank = f.iter().all(|&b| b == 0);
            if blank && scales.is_empty() {
                start += 1;
            } else if blank {
                scales.push(BLANK);
            } else {
                scales.push(be16(f, 0));
                valid += 1;
            }
            if valid >= MIN_TEST_FRAMES {
                i = frame_count;
                break;
            }
        }
    }
    if scales.is_empty() {
        return Ok(None);
    }
    let mask = if kind == 8 { 0x6000 } else { 0x1000 };
    let test = |(x, m, a): (u16, u16, u16)| {
        let mut x = x;
        for _ in 0..start {
            x = x.wrapping_mul(m).wrapping_add(a);
        }
        for &s in &scales {
            if s & mask != x & mask && s != BLANK {
                return false;
            }
            x = x.wrapping_mul(m).wrapping_add(a);
        }
        true
    };
    if kind == 8 {
        for &(x, m, a, s) in keys::KEYS8 {
            let k = if x != 0 || m != 0 || a != 0 { (x, m, a) } else { key8_derive(s) };
            if test(k) {
                return Ok(Some(k));
            }
        }
    } else {
        for &(x, m, a, code) in keys::KEYS9 {
            let k = if x != 0 || m != 0 || a != 0 {
                (x, m, a)
            } else if code != 0 {
                key9_derive(code, subkey)
            } else {
                continue;
            };
            if test(k) {
                return Ok(Some(k));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keystrings_derive_to_listed_keys() {
        for &(x, m, a, s) in keys::KEYS8 {
            assert_eq!(key8_derive(s), (x, m, a), "{}", String::from_utf8_lossy(s));
        }
        // keycodes with listed parameters (the rest are derived at run time)
        for &(x, m, a, code) in keys::KEYS9 {
            if code != 0 && (x, m, a) != (0, 0, 0) {
                assert_eq!(key9_derive(code, 0), (x, m, a), "{code}");
            }
        }
    }
}
