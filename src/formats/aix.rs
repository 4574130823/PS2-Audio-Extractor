//! .aix: CRI's container of ADX streams (vgmstream meta/aix.c + aix_streamfile.h): N
//! segments played in a row, each made of M layers (usually stereo ADX) played together,
//! their data cut in "AIXP" blocks.
//!
//! One segment (any number of layers) decodes here: the layers' frames are gathered row by
//! row into one multichannel ADX stream, which is exactly what vgmstream's layered layout
//! outputs. Several segments (intro + loop...) need each segment decoded on its own
//! (fresh ADX history, cut mid-frame at the segment's sample count), which the shared
//! codec layer can't express yet: those are listed with a note.

use std::io;

use super::{Ctx, Found, Parser, be16, be32};
use crate::codecs::{Codec, adx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "AIX",
    magics: &[b"AIXF"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

const MAX_SEGMENTS: usize = 120;

/// One layer's ADX stream inside a segment, deblocked: its pieces in the file.
struct Layer {
    pieces: Vec<(u64, u64)>,
}

impl Layer {
    /// File pieces for `len` bytes at logical `pos`.
    fn slice(&self, mut pos: u64, mut len: u64, out: &mut Vec<(u64, u64)>) {
        for &(o, s) in &self.pieces {
            if len == 0 {
                break;
            }
            if pos >= s {
                pos -= s;
                continue;
            }
            let n = (s - pos).min(len);
            match out.last_mut() {
                Some(last) if last.0 + last.1 == o + pos => last.1 += n,
                _ => out.push((o + pos, n)),
            }
            len -= n;
            pos = 0;
        }
        if len > 0 {
            // past the end: vgmstream reads nothing there (zeros); keep the layout
            out.push((u64::MAX / 2, len));
        }
    }
    fn bytes(&self, ctx: &mut Ctx, len: usize) -> io::Result<Vec<u8>> {
        let mut v = Vec::new();
        for &(o, s) in &self.pieces {
            if v.len() >= len {
                break;
            }
            v.extend(ctx.bytes(o, s.min((len - v.len()) as u64) as usize)?);
        }
        v.resize(len, 0);
        Ok(v)
    }
}

/// vgmstream's AIX deblocker for layer `layer` of the segment at `start`/`size`.
fn deblock(ctx: &mut Ctx, start: u64, size: u64, layer: u8) -> io::Result<Layer> {
    let mut pieces = Vec::new();
    let mut logical = 0u64;
    let mut p = start;
    while p < start + size {
        let b = ctx.bytes(p, 0x10)?;
        let block_size = be32(&b, 4) as u64 + 8;
        let (mut data_off, mut data_size) = (0u64, 0u64);
        if &b[0..4] == b"AIXP" {
            if b[8] as i8 == layer as i8 {
                data_size = i16::from_be_bytes([b[0x0a], b[0x0b]]) as i64 as u64;
                data_off = 0x10;
            }
            // Tetris Collection (PS2): padding before the ADX header
            if logical == 0 && ctx.u32be(p + 0x10)? == 0 && block_size >= 0x28 && ctx.bytes(p + block_size - 0x28, 2)? == [0x80, 0x00] {
                data_size = 0x28;
                data_off = block_size - 0x28;
            }
        }
        if block_size <= 8 || data_size > block_size {
            break;
        }
        if data_size > 0 {
            pieces.push((p + data_off, data_size));
            logical += data_size;
        }
        p += block_size;
    }
    Ok(Layer { pieces })
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    if &h[0..4] != b"AIXF" || be32(&h, 0x08) != 0x0100_0014 || be32(&h, 0x0c) != 0x800 {
        return Ok(vec![]);
    }
    let data_offset = be32(&h, 0x04) as i32 as i64 + 8;
    let count = be16(&h, 0x18) as usize;
    if count == 0 || count > MAX_SEGMENTS || data_offset <= 0 {
        return Ok(vec![]);
    }
    let data_offset = data_offset as u64;
    let subtable = 0x20 + count as u64 * 0x10;
    if subtable >= data_offset {
        return Ok(vec![]);
    }
    let table = ctx.bytes(off + 0x20, count * 0x10)?;
    let mut segs = Vec::new();
    let rate0 = be32(&table, 0x0c) as i32;
    for i in 0..count {
        let e = &table[i * 0x10..];
        let (so, ss, sn) = (be32(e, 0) as i32 as i64 as u64, be32(e, 4) as u64, be32(e, 8) as i32);
        let mut rate = be32(e, 0x0c) as i32;
        if i > 0 && rate == 0 {
            rate = rate0;
        }
        if rate != rate0 {
            return Ok(vec![]);
        }
        segs.push((so, ss, sn));
    }
    if segs[0].0 != data_offset || !(4000..=96000).contains(&rate0) {
        return Ok(vec![]);
    }
    let file_size = ctx.size() - off;
    // Metroid: Other M: truncated 3-segment file (and then it doesn't loop)
    let mut force_no_loop = false;
    if count == 3 && segs[1].0 + segs[1].1 > file_size {
        force_no_loop = true;
        segs.truncate(2);
        segs[1].1 = file_size.saturating_sub(segs[1].0);
    }
    if ctx.u8(off + subtable)? != 0x01 {
        return Ok(vec![]);
    }
    let layer_list = subtable + 0x10;
    if layer_list >= data_offset {
        return Ok(vec![]);
    }
    let layers = ctx.u8(off + layer_list)? as usize;
    if layers == 0 || layer_list + 8 + layers as u64 * 8 >= data_offset {
        return Ok(vec![]);
    }
    for i in 0..layers as u64 {
        if ctx.u32be(off + layer_list + 8 + i * 8)? as i32 != rate0 {
            return Ok(vec![]);
        }
    }
    let end = segs.iter().map(|s| off + s.0 + s.1).max().unwrap_or(off).min(ctx.size());
    if segs.iter().any(|s| s.0 >= file_size || s.1 == 0) {
        return Ok(vec![]);
    }

    // Segment 0's layers: the channels and the first samples.
    let mut infos = Vec::new();
    for seg in segs.iter() {
        let mut seg_layers = Vec::new();
        for l in 0..layers {
            let layer = deblock(ctx, off + seg.0, seg.1, l as u8)?;
            let Some(info) = adx_info(ctx, &layer)? else { return Ok(vec![]) };
            seg_layers.push((layer, info));
        }
        infos.push(seg_layers);
    }
    let channels: u16 = infos[0].iter().map(|l| l.1.channels).sum();
    let total: i64 = segs.iter().map(|s| s.2 as i64).sum();
    if total <= 0 || channels == 0 || channels > 16 {
        return Ok(vec![]);
    }
    let first = &infos[0];
    let rate = first[0].1.rate;
    let coef = adx::coefs(first[0].1.cutoff, rate);
    let same = first.iter().all(|(_, i)| i.cutoff == first[0].1.cutoff && i.rate == rate && i.version == first[0].1.version && i.encoding == first[0].1.encoding);

    let mut note = None;
    let usable = |seg: &Vec<(Layer, AdxInfo)>| {
        seg.iter().all(|(_, i)| {
            i.cutoff == first[0].1.cutoff && i.rate == rate && i.version == first[0].1.version && i.encoding == first[0].1.encoding && !i.encrypted && i.encoding != 2
        }) && seg.iter().map(|l| l.1.channels).sum::<u16>() == channels
    };
    if !same || !infos.iter().all(usable) {
        note = Some("AIX with mixed or encrypted ADX layers isn't supported".to_string());
    }

    // One ADX track per segment: its layers' frames gathered row by row (layer 0's
    // channels, layer 1's, ...).
    let part = |seg: &Vec<(Layer, AdxInfo)>, samples: u64| -> Track {
        let frames = samples.div_ceil(32);
        let mut pieces = Vec::new();
        let hist: Vec<(i32, i32)> = seg.iter().flat_map(|(_, i)| i.hist.iter().copied()).collect();
        if seg.len() == 1 {
            let (layer, info) = &seg[0];
            layer.slice(info.start, frames * 0x12 * info.channels as u64, &mut pieces);
        } else {
            for f in 0..frames {
                for (layer, info) in seg {
                    let row = 0x12 * info.channels as u64;
                    layer.slice(info.start + f * row, row, &mut pieces);
                }
            }
        }
        let data = if pieces.is_empty() { Data::at(ctx.entry, off, 0) } else { Data::blocks(ctx.entry, pieces) };
        Track::new(ctx.entry, off, "AIX", channels, rate, samples, data, Codec::Adx(adx::Params {
            v3: first[0].1.version == 0x0300,
            exponential: first[0].1.encoding == 4,
            coef,
            hist,
            key: None,
            interleave: 0x12,
        }))
    };
    let samples = total as u64;
    let mut t = if note.is_some() {
        Track::new(ctx.entry, off, "AIX", channels, rate, samples, Data::at(ctx.entry, off, 0), Codec::None)
    } else if segs.len() == 1 {
        part(&infos[0], samples)
    } else {
        // Segments play in a row, each decoded afresh (vgmstream's segmented layout).
        let parts: Vec<Track> = infos.iter().zip(&segs).map(|(seg, s)| part(seg, s.2.max(0) as u64)).collect();
        let mut t = part(&infos[0], samples);
        t.codec = Codec::Segmented(parts);
        t
    };
    if let Some(n) = note {
        t.codec = Codec::None;
        t.note = Some(n);
    }
    // No loop info in the header; vgmstream loops by segment count: intro + loop (+ end).
    let n = segs.len();
    if !force_no_loop && (1..=5).contains(&n) {
        let (ls, le) = if n > 3 { (2, n - 2) } else { (1, 1) };
        if ls < n && le < n && ls <= le {
            let before = |i: usize| segs[..i].iter().map(|s| s.2.max(0) as u64).sum::<u64>();
            t = t.looped(before(ls), before(le + 1));
        }
    }
    Ok(vec![Found::new(t, end)])
}

struct AdxInfo {
    channels: u16,
    rate: u32,
    cutoff: u16,
    version: u16,
    encoding: u8,
    encrypted: bool,
    start: u64,
    hist: Vec<(i32, i32)>,
}

/// The ADX header at the start of a layer (vgmstream's `init_vgmstream_adx` checks).
fn adx_info(ctx: &mut Ctx, layer: &Layer) -> io::Result<Option<AdxInfo>> {
    let h = layer.bytes(ctx, 0x14)?;
    if be16(&h, 0) != 0x8000 {
        return Ok(None);
    }
    let start = be16(&h, 2) as u64 + 4;
    let (encoding, frame, bits, channels) = (h[4], h[5], h[6], h[7] as u16);
    let rate = be32(&h, 8);
    let cutoff = be16(&h, 0x10);
    let mut version = be16(&h, 0x12);
    if !(2..=4).contains(&encoding) || frame != 0x12 || bits != 4 || channels == 0 || channels > 8 || start < 0x14 {
        return Ok(None);
    }
    let encrypted = version == 0x0408 || version == 0x0409;
    if encrypted {
        version = 0x0400;
    }
    if !matches!(version, 0x0300 | 0x0400 | 0x0500) {
        return Ok(None);
    }
    let header = layer.bytes(ctx, start as usize)?;
    if &header[start as usize - 6..] != b"(c)CRI" {
        return Ok(None);
    }
    let hist = if version == 0x0400 {
        (0..channels as usize)
            .map(|c| {
                let at = 0x18 + c * 4;
                let g = |a: usize| header.get(a..a + 2).map(|b| i16::from_be_bytes([b[0], b[1]]) as i32).unwrap_or(0);
                (g(at), g(at + 2))
            })
            .collect()
    } else {
        vec![(0, 0); channels as usize]
    };
    Ok(Some(AdxInfo { channels, rate, cutoff, version, encoding, encrypted, start, hist }))
}
