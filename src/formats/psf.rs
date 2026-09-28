//! PSF - Pivotal games [The Great Escape, Conflict series] (vgmstream meta/psf.c):
//! single-segment PSFs, segmented PSFs (music in segments linked into 4 tracks: subsongs
//! "full", "track1".."track4" and each segment), and .SCH containers of internal sounds
//! (PFSM). SCH entries pointing into other files (PFST/IMUS, whose PSFs are found where
//! they are) are skipped. PS-ADPCM and PCM16 are read; Pivotal's PS-ADPCM variant (PC/Xbox)
//! and GameCube DSP are listed with a note.

use std::io;

use super::ps2p::{merge, vgm_loop};
use super::{Ctx, Found, Parser, be16, be32, le16, le32};
use crate::codecs::{Codec, pcm, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "PSF",
    magics: &[
        b"PSF\xc0", b"PSF\x40", b"PSF\xa1", b"PSF\x21", b"PSF\x80", b"PSF\x81", b"PSF\x01", b"PSF\xd1", b"PSF\x60", b"PSF\x31",
        b"SCH\0", b"\0HCS",
    ],
    magic_at: 0,
    exts: &[],
    locate: Some(locate),
    parse,
};

/// SCH headers may start with "HDRSND" 0x0E before the "SCH\0".
fn locate(ctx: &mut Ctx, hit: u64) -> io::Result<Option<u64>> {
    if hit >= 0x0e && (ctx.is(hit, b"SCH\0")? || ctx.is(hit, b"\0HCS")?) && ctx.is(hit - 0x0e, b"HDRS")? {
        return Ok(Some(hit - 0x0e));
    }
    Ok(Some(hit))
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let id = ctx.bytes(off, 4)?;
    if &id[0..3] == b"PSF" {
        if id[3] == 0x60 || id[3] == 0x31 {
            return segmented(ctx, off);
        }
        return Ok(match single(ctx, off)? {
            Some(s) => {
                let end = s.end;
                vec![Found::new(s.track(ctx.entry, off, &[(s.start, s.data_size)], None), end)]
            }
            None => vec![],
        });
    }
    sch(ctx, off)
}

/// A single-segment PSF.
struct Single {
    channels: u16,
    rate: u32,
    samples: u64,
    start: u64,
    data_size: u64,
    end: u64,
    /// Codec, or why it can't be decoded.
    kind: Result<Codec, &'static str>,
}

impl Single {
    fn track(&self, entry: usize, off: u64, pieces: &[(u64, u64)], note: Option<String>) -> Track {
        let data = Data::blocks(entry, merge(pieces.to_vec()));
        let mut t = Track::new(entry, off, "PSF", self.channels, self.rate, self.samples, data, Codec::None);
        match (&self.kind, note) {
            (Ok(c), None) => t.codec = c.clone(),
            (Ok(_), Some(n)) => t.note = Some(n),
            (Err(n), _) => t.note = Some(n.to_string()),
        }
        t
    }
}

fn single(ctx: &mut Ctx, off: u64) -> io::Result<Option<Single>> {
    let h = ctx.bytes(off, 0x10)?;
    if &h[0..3] != b"PSF" {
        return Ok(None);
    }
    let flags = h[3];
    let (kind, channels, start): (Result<Codec, &'static str>, u64, u64) = match flags {
        0xc0 | 0x40 | 0xa1 | 0x21 => (Ok(Codec::Psx(psx::Params::interleaved(0x10))), if flags == 0x21 || flags == 0x40 { 1 } else { 2 }, 8),
        0x80 | 0x81 | 0x01 => (Err("Pivotal PS-ADPCM variant (not supported)"), if flags == 0x01 { 1 } else { 2 }, 8),
        0xd1 => (Err("NGC DSP audio (not supported)"), 2, 8 + 0x60 * 2),
        _ => return Ok(None),
    };
    let config = le32(&h, 4);
    let rate_value = (config >> 20) & 0xfff;
    let rate = match rate_value {
        3763 => 44100,
        1365 => 16000,
        940 => 11050,
        460 => 5000,
        v => (v as f64 * 11.72) as u32,
    };
    let interleave = if flags == 0xd1 { 8 } else { 0x10 };
    let data_size = (config & 0xfffff) as u64 * interleave * channels;
    let samples = match flags {
        0xd1 => be32(&h, 8) as i32 as i64 as u64,
        0x80 | 0x81 | 0x01 => data_size / channels / 0x10 * 30,
        _ => psx::bytes_to_samples(data_size, channels as u16),
    };
    let end = off + start + data_size;
    // (vgmstream's minimum rate is 300; real files use the few rates above)
    if rate < 300 || rate > 96000 || samples == 0 || samples > i32::MAX as u64 || end > ctx.size() {
        return Ok(None);
    }
    let head = ctx.bytes(off + start, 0x100.min(data_size as usize))?;
    let ok = match flags {
        0x80 | 0x81 | 0x01 => pivotal_plausible(&head),
        // DSP data can't be checked here: only take the rates real files use
        0xd1 => matches!(rate_value, 3763 | 1365 | 940 | 460),
        _ => psx::plausible(&head),
    };
    if !ok {
        return Ok(None);
    }
    Ok(Some(Single { channels: channels as u16, rate, samples, start: off + start, data_size, end, kind }))
}

/// Pivotal's PS-ADPCM variant has 0x10-byte frames whose first byte is the coefficient
/// index (0..4) and shift (0..12); random data rarely passes that over several frames.
fn pivotal_plausible(buf: &[u8]) -> bool {
    let frames: Vec<u8> = buf.chunks_exact(0x10).map(|f| f[0]).collect();
    !frames.is_empty() && frames.iter().all(|&b| b >> 4 <= 4 && b & 0xf <= 12)
}

fn segmented(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let tc = ctx.i32le(off + 4)?;
    if !(1..=1024).contains(&tc) || off + 8 + tc as u64 * 0x0c > ctx.size() {
        return Ok(vec![]);
    }
    let table = ctx.bytes(off + 8, tc as usize * 0x0c)?;
    let point = |seg: usize, track: usize| -> Option<usize> {
        let at = seg * 0x0c + 4 + 2 * track;
        (at + 2 <= table.len()).then(|| le16(&table, at) as usize)
    };
    let seg_offset = |seg: usize| -> Option<u64> { (seg * 0x0c + 4 <= table.len()).then(|| le32(&table, seg * 0x0c) as u64) };

    // Segments (checked once). vgmstream fails only the subsongs using a bad one, but a bad
    // one past segment 0 (which holds the tracks' entry points) means this isn't a PSF.
    let mut singles: Vec<Option<Single>> = Vec::new();
    let mut end = off + 8 + tc as u64 * 0x0c;
    for seg in 0..tc as usize {
        let s = single(ctx, off + seg_offset(seg).unwrap())?;
        match &s {
            Some(s) => end = end.max(s.end),
            None if seg > 0 => return Ok(vec![]),
            None => {}
        }
        singles.push(s);
    }

    let total = 1 + 4 + (tc as usize - 1);
    let mut found = Vec::new();
    for target in 1..=total {
        let mut seq: Vec<usize> = Vec::new();
        let (mut loop_flag, mut loop_start, mut loop_end) = (false, 0usize, 0usize);
        let mut track = [[0usize; 256]; 4];
        let mut count = [0usize; 4];
        let mut cur: i32;
        let name;
        if target == 1 {
            cur = 0;
            name = "full".to_string();
        } else if target <= 5 {
            cur = target as i32 - 2;
            name = format!("track{}", cur + 1);
        } else {
            let seg = target - 5;
            seq.push(seg);
            cur = -1;
            name = format!("segment{seg:03}");
        }
        let mut cur_point = 0usize;
        let mut bad = false;
        while seq.len() < 512 && cur >= 0 {
            let Some(mut next) = point(cur_point, cur as usize) else {
                bad = true;
                break;
            };
            let repeat = track[cur as usize][..count[cur as usize]].iter().position(|&p| p == next);
            if let Some(rp) = repeat {
                if target == 1 {
                    cur += 1;
                    if loop_flag {
                        loop_start = rp;
                        break;
                    }
                    if cur > 3 {
                        cur = 0;
                        loop_flag = true;
                    }
                    match point(cur_point, cur as usize) {
                        Some(n) => next = n,
                        None => {
                            bad = true;
                            break;
                        }
                    }
                    if loop_flag {
                        loop_end = seq.len();
                    }
                } else {
                    loop_flag = true;
                    loop_start = rp;
                    loop_end = seq.len().wrapping_sub(1);
                    break;
                }
            }
            let c = cur as usize;
            if count[c] >= 256 {
                bad = true;
                break;
            }
            track[c][count[c]] = next;
            count[c] += 1;
            seq.push(next);
            cur_point = next;
        }
        if bad || seq.is_empty() || seq.iter().any(|&s| s >= tc as usize || singles[s].is_none()) {
            continue;
        }
        // Segments played one after another.
        let first = singles[seq[0]].as_ref().unwrap();
        let (channels, rate) = (first.channels, first.rate);
        let mut pieces = Vec::new();
        let (mut samples, mut ls, mut le) = (0u64, 0u64, 0u64);
        let note = None;
        let mut ok = true;
        // Each segment is encoded on its own: the decoder starts afresh at each one.
        let mut resets = Vec::new();
        let mut joined = 0u64;
        for (i, &s) in seq.iter().enumerate() {
            let sg = singles[s].as_ref().unwrap();
            if sg.channels != channels || sg.rate != rate || sg.kind.is_err() != first.kind.is_err() {
                ok = false;
                break;
            }
            if loop_flag && i == loop_start {
                ls = samples;
            }
            // Only the samples it has (a segment's data is whole frames).
            let bytes = sg.samples / 28 * 16 * channels as u64;
            if i > 0 {
                resets.push(joined);
            }
            joined += bytes;
            pieces.push((sg.start, bytes));
            samples += sg.samples;
            if loop_flag && i == loop_end {
                le = samples;
            }
        }
        if !ok || samples == 0 || samples > i32::MAX as u64 {
            continue;
        }
        let mut t = Single { channels, rate, samples, start: 0, data_size: 0, end, kind: first.kind.clone() }.track(ctx.entry, off, &pieces, note);
        t.data.resets = Some(std::sync::Arc::new(resets));
        if loop_flag {
            t = vgm_loop(t, ls as i64, le as i64);
        }
        found.push(Found::new(t, end).label(Some(name)));
    }
    Ok(found)
}

/// SCH: chunks listing a game area's sounds; PFSM chunks hold one each.
fn sch(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let skip = if ctx.is(off, b"HDRS")? { 0x0e } else { 0 };
    let id = ctx.bytes(off + skip, 4)?;
    let be = match &id[..] {
        b"SCH\0" => false,
        b"\0HCS" => true,
        _ => return Ok(vec![]),
    };
    let r32 = |b: &[u8], at: usize| if be { be32(b, at) } else { le32(b, at) };
    let pad = if be { 0x18 } else { 0 };
    let size = ctx.size();
    let declared = r32(&ctx.bytes(off + skip, 8)?, 4) as u64 + skip + 8 + pad;
    let file_end = if off == 0 {
        if declared < size {
            return Ok(vec![]);
        }
        size
    } else {
        if off + declared > size {
            return Ok(vec![]);
        }
        off + declared
    };
    let mut pos = off + skip + 8 + pad;
    let mut chunks = Vec::new();
    while pos < file_end {
        let c = ctx.bytes(pos, 8)?;
        let csize = r32(&c, 4) as u64;
        match &c[0..4] {
            b"IMUS" | b"PFST" | b"TSFP" | b"PFSM" | b"MSFP" => chunks.push((pos, 8 + pad + csize, &c[0..4] == b"PFSM" || &c[0..4] == b"MSFP")),
            b"BANK" | b"KNAB" | b"BLOK" => {}
            _ => return Ok(vec![]),
        }
        pos += 8 + pad + csize;
    }
    let mut found = Vec::new();
    for (at, csize, is_pfsm) in chunks {
        if !is_pfsm || at + csize > file_end {
            continue;
        }
        if let Some(f) = pfsm(ctx, at, csize, off, file_end)? {
            found.push(f);
        }
    }
    Ok(found)
}

/// An internal sound (the sub-file [base, base + size)).
fn pfsm(ctx: &mut Ctx, base: u64, size: u64, off: u64, end: u64) -> io::Result<Option<Found>> {
    if size <= 0x18 {
        return Ok(None);
    }
    let h = ctx.bytes(base, 0x60.min(size as usize))?;
    let h = [h, vec![0; 0x60]].concat();
    let be = match &h[0..4] {
        b"PFSM" => false,
        b"MSFP" => true,
        _ => return Ok(None),
    };
    let r16 = |at: usize| if be { be16(&h, at) } else { le16(&h, at) };
    let r32 = |at: usize| if be { be32(&h, at) } else { le32(&h, at) };
    let language = r32(0x08) as i32;
    let resource = r32(0x0c);
    let (kind, start, pitch, rate0): (Result<Codec, &'static str>, u64, u32, u32) = if be && r32(0x50) != 0 {
        (Err("NGC DSP audio (not supported)"), 0x60 + 0x60, r16(0x48) as u32, 0)
    } else if be {
        (Ok(Codec::Pcm(pcm::Params::be16(0))), 0x60, r16(0x48) as u32, 0)
    } else if h[0x16] == 0xff {
        (Ok(Codec::Psx(psx::Params::default())), 0x18, r16(0x14) as u32, 0)
    } else {
        (Err("Pivotal PS-ADPCM variant (not supported)"), 0x18, 0, r16(0x14) as u32)
    };
    if size <= start {
        return Ok(None);
    }
    let data_size = size - start;
    let rate = if rate0 == 0 {
        let v = (48000 * pitch as i32) / 4096;
        let r = v % 10;
        (if r < 5 { v - r } else { v + (10 - r) }) as u32
    } else {
        rate0
    };
    let samples = match (&kind, be) {
        (Ok(Codec::Pcm(_)), _) => data_size / 2,
        (Ok(_), _) => psx::bytes_to_samples(data_size, 1),
        (Err(_), true) => be32(&h, 0x60) as i32 as i64 as u64,
        (Err(_), false) => data_size / 0x10 * 30,
    };
    if !(300..=96000).contains(&rate) || samples == 0 || samples > i32::MAX as u64 {
        return Ok(None);
    }
    if !be {
        let head = ctx.bytes(base + start, 0x100.min(data_size as usize))?;
        if !if kind.is_ok() { psx::plausible(&head) } else { pivotal_plausible(&head) } {
            return Ok(None);
        }
    }
    let data = Data::at(ctx.entry, base + start, data_size);
    let mut t = Track::new(ctx.entry, off, "PSF", 1, rate, samples, data, Codec::None);
    match kind {
        Ok(c) => t.codec = c,
        Err(n) => t.note = Some(n.to_string()),
    }
    let name = if language >= 0 { format!("R{resource}_L{language}_PFSM") } else { format!("R{resource}_PFSM") };
    Ok(Some(Found::new(t, end).label(Some(name))))
}
