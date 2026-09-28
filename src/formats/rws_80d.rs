//! RWS - RenderWare Stream (RenderWare Audio) [Max Payne 2, Silent Hill Origins, Ghost Rider,
//! Nana, kill.switch (PS2), Burnout 2 (GC/Xbox)...] (vgmstream meta/rws_80d.c +
//! layout/blocked_rws.c).
//!
//! Chunks 0x80d (file) > 0x80e (header) + 0x80f (data). The data is split in "segments"
//! (parts played in turn, or separate sounds) and "layers" (streams whose blocks take
//! turns); each layer of each segment is a track, in vgmstream's order.

use std::io;

use super::{Ctx, Found, Parser, be16, be32, le16, le32, sane_rate, split_ext};
use crate::codecs::{Codec, ima, pcm, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "RWS",
    magics: &[b"\x0d\x08\x00\x00"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

const PCM: u32 = 0xD01B_D217;
const PSX: u32 = 0xD9EA_9798;
const DSP: u32 = 0xF862_15B0;
const XBOX_PC: u32 = 0xEF38_6593;
const XBOX: u32 = 0x632F_A22B;

struct Layer {
    interleave: u64,
    block_size: u64,
    start: u64,
    rate: u32,
    channels: u16,
    codec: u32,
    name: String,
}

struct Segment {
    layers_size: u64,
    offset: u64,
    name: String,
}

/// The header chunk, read with vgmstream's rules (reads past the end give 0xFF bytes).
struct H {
    b: Vec<u8>,
    big: bool,
}

impl H {
    fn u8(&self, at: u64) -> u8 {
        self.b.get(at as usize).copied().unwrap_or(0xff)
    }
    fn u32(&self, at: u64) -> u32 {
        match self.b.get(at as usize..at as usize + 4) {
            Some(s) if self.big => be32(s, 0),
            Some(s) => le32(s, 0),
            None => u32::MAX,
        }
    }
    fn u16(&self, at: u64) -> u16 {
        match self.b.get(at as usize..at as usize + 2) {
            Some(s) if self.big => be16(s, 0),
            Some(s) => le16(s, 0),
            None => u16::MAX,
        }
    }
    /// Strings are NUL-terminated and padded to 0x10.
    fn string(&self, at: u64) -> (String, u64) {
        for i in 0..255u64 {
            if self.u8(at + i) == 0 {
                let s = (0..i).map(|k| self.u8(at + k) as char).collect();
                return (s, i + (0x10 - i % 0x10));
            }
        }
        (String::new(), 0)
    }
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let c = ctx.bytes(off, 0x18)?;
    if le32(&c, 0) != 0x80d || le32(&c, 0x0c) != 0x80e {
        return Ok(vec![]);
    }
    let file_size = le32(&c, 0x04) as u64;
    let header_size = le32(&c, 0x10) as u64;
    let end = off + 0x0c + file_size;
    let data_offset = 0x18 + header_size;
    if end > ctx.size() || !(0x60..=0x10_0000).contains(&header_size) || data_offset + 0x0c > file_size + 0x0c {
        return Ok(vec![]);
    }
    let d = ctx.bytes(off + data_offset, 8)?;
    if le32(&d, 0) != 0x80f || le32(&d, 4) as u64 + 0x0c + data_offset != file_size + 0x0c {
        return Ok(vec![]);
    }
    let raw = ctx.bytes(off + 0x18, header_size as usize)?;
    let big = le32(&raw, 0) > be32(&raw, 0);
    let h = H { b: raw, big };

    let total_segments = h.u32(0x20) as i32;
    let total_layers = h.u32(0x28) as i32;
    if total_segments <= 0 || total_layers <= 0 || total_segments > 0x1000 || total_layers > 0x100 {
        return Ok(vec![]);
    }
    let (nseg, nlay) = (total_segments as u64, total_layers as u64);
    let mut o = 0x50u64;
    let (file_name, n) = h.string(o);
    o += n;
    let mut segments = Vec::new();
    for _ in 0..nseg {
        segments.push(Segment { layers_size: h.u32(o + 0x18) as u64, offset: h.u32(o + 0x1c) as u64, name: String::new() });
        o += 0x20;
    }
    let usable: Vec<u64> = (0..nseg * nlay).map(|i| h.u32(o + 4 * i) as u64).collect();
    o += 4 * nseg * nlay;
    o += 0x10 * nseg;
    for s in segments.iter_mut() {
        let (name, n) = h.string(o);
        s.name = name;
        o += n;
    }
    let mut layers = Vec::new();
    let mut block_layers_size = 0u64;
    for _ in 0..nlay {
        layers.push(Layer {
            interleave: h.u16(o + 0x18) as u64,
            block_size: h.u32(o + 0x20) as u64,
            start: h.u32(o + 0x24) as u64,
            rate: 0,
            channels: 0,
            codec: 0,
            name: String::new(),
        });
        block_layers_size += h.u32(o + 0x10) as u64;
        o += 0x28;
    }
    for l in layers.iter_mut() {
        l.rate = h.u32(o);
        l.channels = h.u8(o + 0x0d) as u16;
        l.codec = h.u32(o + 0x1c);
        o += 0x2c;
        if l.codec == DSP {
            o += 0x60;
        }
        o += 4;
    }
    o += 0x10 * nlay;
    for l in layers.iter_mut() {
        let (name, n) = h.string(o);
        l.name = name;
        o += n;
    }
    if o > header_size || block_layers_size == 0 {
        return Ok(vec![]);
    }

    let stem = split_ext(ctx.path()).0.rsplit('/').next().unwrap_or("").to_string();
    let data_start = off + data_offset + 0x0c;
    let mut found = Vec::new();
    for sub in 0..(nseg * nlay) as usize {
        let (seg, lay) = (&segments[sub / nlay as usize], &layers[sub % nlay as usize]);
        let ch = lay.channels as u64;
        if !(1..=8).contains(&lay.channels) || !sane_rate(lay.rate) || lay.block_size == 0 {
            continue;
        }
        let start = data_start + seg.offset + lay.start;
        let expected = (seg.layers_size / block_layers_size) * (lay.block_size * nlay) / nlay;
        let stream_size = usable[sub].min(expected);
        let cbs = lay.block_size / ch; // bytes per channel per block
        // (frame bytes, samples per frame) as the blocked layout counts them per channel.
        let (codec, samples, fb, fs, note) = match lay.codec {
            PCM => {
                let p = if lay.interleave == 2 {
                    pcm::Params { big_endian: big, ..pcm::Params::le16(0) }
                } else if big {
                    pcm::Params::be16(0)
                } else {
                    pcm::Params::le16(0)
                };
                (Codec::Pcm(p), stream_size / 2 / ch, 2, 1, None)
            }
            PSX => (Codec::Psx(psx::Params::default()), psx::bytes_to_samples(stream_size, lay.channels), 16, 28, None),
            XBOX | XBOX_PC => {
                (Codec::Ima(ima::Params::new(ima::Kind::Xbox, 0)), ima::xbox_bytes_to_samples(stream_size, lay.channels), 0x24, 64, None)
            }
            DSP => (Codec::None, stream_size / ch / 8 * 14, 8, 14, Some("GameCube/Wii DSP ADPCM audio isn't supported")),
            _ => continue,
        };
        let frames = cbs / fb;
        if samples == 0 || frames == 0 {
            continue;
        }
        let per_block = frames * fs;
        let blocks = samples.div_ceil(per_block);
        // Where each channel's data is in each block, laid out as plain interleaved data.
        let il = match lay.codec {
            PSX => lay.block_size / 2,
            _ => lay.interleave,
        };
        let mut pieces = Vec::new();
        let mut params_il = frames * fb;
        for b in 0..blocks {
            let block = start + b * block_layers_size;
            match lay.codec {
                XBOX | XBOX_PC => pieces.push((block, frames * if ch > 1 { 0x48 } else { 0x24 })),
                PCM if lay.interleave == 2 => {
                    pieces.push((block, frames * fb * ch));
                    params_il = 2;
                }
                _ => {
                    for c in 0..ch {
                        pieces.push((block + il * c, frames * fb));
                    }
                }
            }
        }
        let codec = match codec {
            Codec::Psx(_) => Codec::Psx(psx::Params::interleaved(params_il)),
            Codec::Pcm(mut p) => {
                p.interleave = params_il;
                Codec::Pcm(p)
            }
            other => other,
        };
        let data = Data::blocks(ctx.entry, pieces);
        let mut t = Track::new(ctx.entry, off, "RWS", lay.channels, lay.rate, samples, data, codec);
        t.note = note.map(str::to_string);
        let name = match (file_name.eq_ignore_ascii_case(&stem), nlay > 1) {
            (true, true) => format!("{}/{}", seg.name, lay.name),
            (true, false) => seg.name.clone(),
            (false, true) => format!("{file_name}/{}/{}", seg.name, lay.name),
            (false, false) => format!("{file_name}/{}", seg.name),
        };
        found.push(Found::new(t, end).label(super::label(name.as_bytes())));
    }
    Ok(found)
}
