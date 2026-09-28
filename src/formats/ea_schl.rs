//! EA SCHl / BNK engine (vgmstream meta/ea_schl.c + layout/blocked_ea_schl.c): the
//! variable "PT"/"GSTR" header EA's sx.exe writes, SCHl block streams and BNK banks.
//! It finds nothing by itself: the containers using it are `ea_schl_standard` (SCHl
//! streams, BNK banks, videos), `ea_schl_abk`, `ea_schl_hdr_dat` and
//! `ea_schl_map_mpf_mus`.

use std::io;
use std::sync::Arc;

use super::{Parser, sane_rate};
use crate::codecs::{Codec, ea_xa};
use crate::disc::Reader;
use crate::track::{Data, Track};

/// Nothing to find here on its own (see the module docs).
pub const PARSER: Parser = super::NONE;

const PLATFORM_PC: i32 = 0x00;
const PLATFORM_PSX: i32 = 0x01;
const PLATFORM_N64: i32 = 0x02;
const PLATFORM_MAC: i32 = 0x03;
const PLATFORM_SAT: i32 = 0x04;
const PLATFORM_PS2: i32 = 0x05;
const PLATFORM_GC: i32 = 0x06;
const PLATFORM_XBOX: i32 = 0x07;
const PLATFORM_GENERIC: i32 = 0x08;
const PLATFORM_X360: i32 = 0x09;
const PLATFORM_PSP: i32 = 0x0A;
const PLATFORM_PS3: i32 = 0x0E;
const PLATFORM_WII: i32 = 0x10;
const PLATFORM_3DS: i32 = 0x14;

const CODEC2_S16LE_INT: i32 = 0x00;
const CODEC2_S16BE_INT: i32 = 0x01;
const CODEC2_S8_INT: i32 = 0x02;
const CODEC2_EAXA_INT: i32 = 0x03;
const CODEC2_MT10: i32 = 0x04;
const CODEC2_VAG: i32 = 0x05;
const CODEC2_N64: i32 = 0x06;
const CODEC2_S16BE: i32 = 0x07;
const CODEC2_S16LE: i32 = 0x08;
const CODEC2_S8: i32 = 0x09;
const CODEC2_EAXA: i32 = 0x0A;
const CODEC2_IMA_INT: i32 = 0x0D;
const CODEC2_LAYER2: i32 = 0x0F;
const CODEC2_LAYER3: i32 = 0x10;
const CODEC2_GCADPCM: i32 = 0x12;
const CODEC2_XBOXADPCM: i32 = 0x14;
const CODEC2_MT5: i32 = 0x16;
const CODEC2_EALAYER3: i32 = 0x17;
const CODEC2_ATRAC3PLUS: i32 = 0x1B;

const FLAG_SIZE_BE: u32 = 0x01;
const FLAG_ADPCM: u32 = 0x02;
const FLAG_OFFSETS: u32 = 0x04;

/// Multi-language header ids ("SH" + language), used in videos.
pub const LANGS: [&[u8; 2]; 13] = [b"EN", b"FR", b"GE", b"DE", b"IT", b"SP", b"ES", b"MX", b"RU", b"JA", b"JP", b"PL", b"BR"];

/// Big-endian reads that give 0 past the end of the file, like the others.
pub struct Rd<'a>(pub &'a mut Reader);

impl Rd<'_> {
    pub fn size(&self) -> u64 {
        self.0.size
    }
    pub fn b(&mut self, off: u64, n: usize) -> io::Result<Vec<u8>> {
        self.0.bytes(off, n)
    }
    pub fn u8(&mut self, off: u64) -> io::Result<u8> {
        Ok(self.0.bytes(off, 1)?[0])
    }
    pub fn u16(&mut self, off: u64, be: bool) -> io::Result<u16> {
        let b = self.0.bytes(off, 2)?;
        Ok(if be { u16::from_be_bytes([b[0], b[1]]) } else { u16::from_le_bytes([b[0], b[1]]) })
    }
    pub fn u32(&mut self, off: u64, be: bool) -> io::Result<u32> {
        let b: [u8; 4] = self.0.bytes(off, 4)?.try_into().unwrap();
        Ok(if be { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
    }
    pub fn id(&mut self, off: u64) -> io::Result<u32> {
        self.u32(off, true)
    }
    /// vgmstream's guess_endian32: big endian when the LE reading is the bigger one.
    pub fn guess_be(&mut self, off: u64) -> io::Result<bool> {
        Ok(self.u32(off, false)? > self.u32(off, true)?)
    }
}

fn id(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

#[derive(Debug, Clone, Default)]
pub struct EaHeader {
    pub num_samples: i32,
    pub sample_rate: i32,
    pub channels: i32,
    pub platform: i32,
    pub version: i32,
    pub bps: i32,
    pub codec1: i32,
    pub codec2: i32,
    pub loop_start: i32,
    pub loop_end: i32,
    pub flag_value: u32,
    pub offsets: [u64; 6],
    pub big_endian: bool,
    pub loop_flag: bool,
    pub use_pcm_blocks: bool,
    pub block_config: u32,
}

fn read_patch(r: &mut Rd, off: &mut u64) -> io::Result<u32> {
    let count = r.u8(*off)?;
    *off += 1;
    if count == 0xFF {
        *off += 4 + r.u32(*off, true)? as u64;
        return Ok(0);
    }
    if count > 4 {
        *off += count as u64;
        return Ok(0);
    }
    let mut v = 0u32;
    for _ in 0..count {
        v = (v << 8) | r.u8(*off)? as u32;
        *off += 1;
    }
    Ok(v)
}

/// parse_variable_header: the PT/GSTR header at `begin`. None if it isn't one.
pub fn parse_variable_header(r: &mut Rd, begin: u64, max_length: i64, is_bnk: bool) -> io::Result<Option<EaHeader>> {
    let mut ea = EaHeader { version: -1, codec1: -1, codec2: -1, ..Default::default() };
    let mut off = begin;
    let mut platform_id = r.id(off)?;
    if platform_id != id(b"GSTR") && platform_id & 0xFFFF0000 != 0x50540000 {
        off += 4; // unknown field in "nbapsstream"
        platform_id = r.id(off)?;
    }
    if platform_id == id(b"GSTR") {
        ea.platform = PLATFORM_GENERIC;
        off += 8;
    } else if platform_id & 0xFFFF0000 == 0x50540000 {
        ea.platform = r.u16(off + 2, false)? as i32;
        off += 4;
    } else {
        return Ok(None);
    }

    let mut end = false;
    // A header never runs past the file (vgmstream would read garbage; we give up).
    let limit = r.size();
    while !end && ((off - begin) as i64) < max_length {
        if off >= limit {
            return Ok(None);
        }
        let patch = r.u8(off)?;
        off += 1;
        match patch {
            0x00 => {
                read_patch(r, &mut off)?;
            }
            0x03..=0x25 if !matches!(patch, 0x16 | 0x17 | 0x18 | 0x1A) => {
                read_patch(r, &mut off)?;
            }
            0xFC | 0xFD => {}
            0x83 => ea.codec1 = read_patch(r, &mut off)? as i32,
            0xA0 => ea.codec2 = read_patch(r, &mut off)? as i32,
            0x80 => ea.version = read_patch(r, &mut off)? as i32,
            0x81 => ea.bps = read_patch(r, &mut off)? as i32,
            0x82 => ea.channels = read_patch(r, &mut off)? as i32,
            0x84 => ea.sample_rate = read_patch(r, &mut off)? as i32,
            0x85 => ea.num_samples = read_patch(r, &mut off)? as i32,
            0x86 => ea.loop_start = read_patch(r, &mut off)? as i32,
            0x87 => ea.loop_end = read_patch(r, &mut off)?.wrapping_add(1) as i32,
            0x88 => ea.offsets[0] = read_patch(r, &mut off)? as u64,
            0x89 => ea.offsets[1] = read_patch(r, &mut off)? as u64,
            0x94 => ea.offsets[2] = read_patch(r, &mut off)? as u64,
            0x95 => ea.offsets[3] = read_patch(r, &mut off)? as u64,
            0xA2 => ea.offsets[4] = read_patch(r, &mut off)? as u64,
            0xA3 => ea.offsets[5] = read_patch(r, &mut off)? as u64,
            0x8F | 0x90 | 0x91 | 0xAB | 0xAC | 0xAD => {
                read_patch(r, &mut off)?; // DSP/N64 coefs
            }
            0x1A | 0x26..=0x2A => {
                read_patch(r, &mut off)?; // EA-MT/EA-XA loop offsets
            }
            0x8C => ea.flag_value = read_patch(r, &mut off)?,
            0x8A | 0x8B | 0x8D | 0x8E | 0x92 | 0x93 | 0x98 | 0x99 | 0x9C | 0x9D | 0x9E | 0x9F | 0xA6 | 0xA7 | 0xA1 => {
                read_patch(r, &mut off)?;
            }
            0xFF | 0xFE => end = true,
            _ => return Ok(None),
        }
    }
    // 0x16/0x17/0x18 aren't known patches (0x16..0x18 fall outside the list vgmstream accepts).
    if !(0..=6).contains(&ea.channels) {
        return Ok(None);
    }
    if ea.channels == 0 {
        ea.channels = 1;
    }
    ea.loop_flag = ea.loop_end != 0;
    let p = ea.platform;
    if matches!(p, PLATFORM_N64 | PLATFORM_MAC | PLATFORM_SAT | PLATFORM_GC | PLATFORM_X360 | PLATFORM_PS3 | PLATFORM_WII | PLATFORM_GENERIC) {
        ea.big_endian = true;
    }
    if ea.version == -1 {
        ea.version = match p {
            PLATFORM_PC | PLATFORM_PSX | PLATFORM_N64 | PLATFORM_MAC | PLATFORM_SAT => 0,
            PLATFORM_PS2 => 1,
            PLATFORM_GC | PLATFORM_XBOX | PLATFORM_GENERIC => 2,
            PLATFORM_X360 | PLATFORM_PSP | PLATFORM_PS3 | PLATFORM_WII | PLATFORM_3DS => 3,
            _ => return Ok(None),
        };
    }
    if ea.codec1 == -1 && ea.version == 0 {
        ea.codec1 = match p {
            PLATFORM_PC | PLATFORM_MAC | PLATFORM_SAT => 0x00,
            PLATFORM_PSX => 0x06,
            PLATFORM_N64 => 0x05,
            _ => return Ok(None),
        };
    }
    if ea.codec1 != -1 && ea.codec2 == -1 {
        ea.codec2 = match ea.codec1 {
            0x00 => {
                if p == PLATFORM_PC {
                    if ea.bps == 8 { CODEC2_S8_INT } else if ea.big_endian { CODEC2_S16BE_INT } else { CODEC2_S16LE_INT }
                } else if ea.bps == 8 {
                    CODEC2_S8
                } else if ea.big_endian {
                    CODEC2_S16BE
                } else {
                    CODEC2_S16LE
                }
            }
            0x02 => CODEC2_IMA_INT,
            0x05 => CODEC2_N64,
            0x06 => CODEC2_VAG,
            0x07 => {
                if p == PLATFORM_PC || p == PLATFORM_MAC { CODEC2_EAXA_INT } else { CODEC2_EAXA }
            }
            0x09 => CODEC2_MT10,
            _ => return Ok(None),
        };
    }
    if ea.codec2 == -1 {
        ea.codec2 = match p {
            PLATFORM_GENERIC | PLATFORM_PC | PLATFORM_MAC | PLATFORM_X360 | PLATFORM_PSP | PLATFORM_PS3 => CODEC2_EAXA,
            PLATFORM_PSX | PLATFORM_PS2 => CODEC2_VAG,
            PLATFORM_N64 => CODEC2_N64,
            PLATFORM_GC => CODEC2_S16BE,
            PLATFORM_XBOX => CODEC2_S16LE,
            PLATFORM_WII | PLATFORM_3DS => CODEC2_GCADPCM,
            _ => return Ok(None),
        };
    }
    if ea.sample_rate == 0 {
        ea.sample_rate = match p {
            PLATFORM_GENERIC => 48000,
            PLATFORM_PC | PLATFORM_PSX | PLATFORM_N64 | PLATFORM_MAC | PLATFORM_SAT | PLATFORM_PS2 | PLATFORM_PSP => 22050,
            PLATFORM_GC | PLATFORM_XBOX => 24000,
            PLATFORM_X360 | PLATFORM_PS3 => 44100,
            PLATFORM_WII | PLATFORM_3DS => 32000,
            _ => return Ok(None),
        };
    }
    ea.use_pcm_blocks = ea.version == 3 || (ea.version == 2 && matches!(p, PLATFORM_PC | PLATFORM_MAC | PLATFORM_GENERIC));
    if !is_bnk {
        if ea.codec2 == CODEC2_GCADPCM {
            if p == PLATFORM_3DS {
                ea.block_config |= FLAG_ADPCM;
            }
        } else if ea.codec2 == CODEC2_EAXA && !ea.use_pcm_blocks {
            ea.block_config |= FLAG_ADPCM;
        }
    }
    if ea.version > 0 {
        ea.block_config |= FLAG_OFFSETS;
    }
    Ok(Some(ea))
}

/// How a codec2 value decodes here: our decoder, or the name of one we don't have.
fn kind_of(ea: &EaHeader) -> Option<Result<ea_xa::Kind, &'static str>> {
    use ea_xa::Kind::*;
    Some(match ea.codec2 {
        CODEC2_EAXA_INT => Ok(EaXa),
        CODEC2_EAXA => Ok(if ea.use_pcm_blocks { EaXaV2 } else { EaXaInt }),
        CODEC2_S8_INT => Ok(Pcm8Int),
        CODEC2_S16LE_INT | CODEC2_S16BE_INT => Ok(Pcm16Int { big_endian: ea.big_endian }),
        CODEC2_S8 => Ok(Pcm8),
        CODEC2_S16LE => Ok(Pcm16 { big_endian: false }),
        CODEC2_S16BE => Ok(Pcm16 { big_endian: true }),
        CODEC2_VAG => Ok(Psx),
        CODEC2_IMA_INT => Err("EA DVI IMA"),
        CODEC2_XBOXADPCM => Err("Xbox IMA"),
        CODEC2_GCADPCM => Err("Nintendo DSP ADPCM"),
        CODEC2_N64 => Err("N64 VADPCM"),
        CODEC2_LAYER2 | CODEC2_LAYER3 => Err("MPEG audio"),
        CODEC2_EALAYER3 => Err("EALayer3"),
        CODEC2_MT10 | CODEC2_MT5 => Err("EA MicroTalk"),
        CODEC2_ATRAC3PLUS => Err("ATRAC3plus"),
        _ => return None,
    })
}

/// A decodable (or at least described) EA sound.
pub struct Sound {
    pub channels: u16,
    pub rate: u32,
    pub samples: u64,
    pub loops: Option<(u64, u64)>,
    pub data: Data,
    pub codec: Codec,
    pub note: Option<String>,
    /// End of the sound's header+data in its file.
    pub end: u64,
}

impl Sound {
    pub fn track(self, entry: usize, offset: u64, format: &'static str) -> Track {
        let mut t = Track::new(entry, offset, format, self.channels, self.rate, self.samples, self.data, self.codec);
        if let Some((a, b)) = self.loops {
            t = t.looped(a, b);
        }
        t.note = self.note;
        t
    }

    /// First blocks of a segmented sound followed by `next`'s (vgmstream's segmented layout:
    /// each segment decodes on its own). Both must be in the same file, same codec setup.
    pub fn append(mut self, next: Sound) -> Option<Sound> {
        let (Codec::EaXa(a), Codec::EaXa(b)) = (&self.codec, &next.codec) else { return None };
        if a.kind != b.kind || self.channels != next.channels || self.rate != next.rate || self.data.entry != next.data.entry {
            return None;
        }
        // Rebase both onto one range of the file.
        let start = self.data.offset.min(next.data.offset);
        let end = (self.data.offset + self.data.size).max(next.data.offset + next.data.size);
        let mut blocks = Vec::new();
        let mut left = self.samples;
        for bl in a.blocks.iter() {
            let n = (bl.samples as u64).min(left);
            left -= n;
            blocks.push(ea_xa::Block { samples: n as u32, starts: bl.starts.iter().map(|s| s + self.data.offset - start).collect(), reset: bl.reset });
        }
        for (k, bl) in b.blocks.iter().enumerate() {
            blocks.push(ea_xa::Block { samples: bl.samples, starts: bl.starts.iter().map(|s| s + next.data.offset - start).collect(), reset: k == 0 || bl.reset });
        }
        let kind = a.kind;
        self.codec = Codec::EaXa(ea_xa::Params { kind, blocks: Arc::new(blocks) });
        self.data = Data::at(self.data.entry, start, end - start);
        self.samples += next.samples;
        self.end = self.end.max(next.end);
        Some(self)
    }
}

/// vgmstream drops loops that don't fit.
pub fn vgm_loop(start: i64, end: i64, samples: u64) -> Option<(u64, u64)> {
    (start >= 0 && end > start && end as u64 <= samples).then_some((start as u64, end as u64))
}

fn basic_checks(ea: &EaHeader) -> bool {
    ea.num_samples > 0 && sane_rate(ea.sample_rate as u32)
}

/// load_vgmstream_ea_schl: a SCHl (or "SHxx") block stream at `offset`.
pub fn load_schl(r: &mut Reader, entry: usize, offset: u64) -> io::Result<Option<Sound>> {
    let mut r = Rd(r);
    let size = r.size();
    if offset + 0x10 > size {
        return Ok(None);
    }
    let header_id = r.id(offset)?;
    let mut ea_lang = 0u32;
    if header_id & 0xFFFF0000 == 0x53480000 {
        ea_lang = header_id & 0xFFFF;
    } else if header_id != id(b"SCHl") {
        return Ok(None);
    }
    let size_be = r.guess_be(offset + 4)?;
    let header_size = r.u32(offset + 4, size_be)? as u64;
    if header_size < 0x10 || offset + header_size > size {
        return Ok(None);
    }
    let Some(mut ea) = parse_variable_header(&mut r, offset + 8, header_size as i64 - 8, false)? else { return Ok(None) };
    ea.block_config |= ea_lang << 16;
    if size_be {
        ea.block_config |= FLAG_SIZE_BE;
    }
    if !basic_checks(&ea) {
        return Ok(None);
    }
    let Some(kind) = kind_of(&ea) else { return Ok(None) };
    let start = offset + header_size;
    let ch = ea.channels as usize;

    // Walk the blocks (block_update_ea_schl).
    let flag_offsets = ea.block_config & FLAG_OFFSETS != 0;
    let flag_adpcm = ea.block_config & FLAG_ADPCM != 0;
    let lang_data = 0x53440000 | ea_lang;
    let mut blocks: Vec<ea_xa::Block> = Vec::new();
    let mut total = 0u64;
    let mut pos = start;
    let mut audio_blocks = 0;
    let mut end = start;
    while pos < size {
        let bh = r.b(pos, 0x0c)?;
        let block_type = u32::from_be_bytes(bh[0..4].try_into().unwrap());
        let bsize = if size_be { u32::from_be_bytes(bh[4..8].try_into().unwrap()) } else { u32::from_le_bytes(bh[4..8].try_into().unwrap()) } as u64;
        if block_type == 0 || block_type == 0xFFFFFFFF || block_type == id(b"SCEl") {
            end = if block_type == id(b"SCEl") { (pos + bsize.max(8)).min(size) } else { pos };
            break;
        }
        if bsize < 8 || pos + bsize > size {
            // vgmstream would stall here or read past the end: that's where the stream ends.
            end = pos;
            break;
        }
        let is_audio = block_type == id(b"SCDl") || (ea_lang != 0 && block_type == lang_data);
        let samples = if !is_audio || bsize < 0x10 {
            0
        } else if kind == Ok(ea_xa::Kind::Psx) {
            bsize.saturating_sub(0x10) / ch as u64 / 0x10 * 28
        } else {
            r.u32(pos + 8, ea.big_endian)? as u64
        };
        if samples > 0 && total < ea.num_samples as u64 {
            let mut starts = Vec::with_capacity(ch);
            if !flag_offsets {
                for i in 0..ch as u64 {
                    let s = match kind {
                        Ok(ea_xa::Kind::Pcm8Int) => pos + 0x0c + i,
                        Ok(ea_xa::Kind::Pcm16Int { .. }) => pos + 0x0c + 2 * i,
                        Ok(ea_xa::Kind::Pcm8) => pos + 0x0c + i * samples,
                        Ok(ea_xa::Kind::Pcm16 { .. }) => pos + 0x0c + 2 * i * samples,
                        Ok(ea_xa::Kind::Psx) => pos + 0x10 + (bsize - 0x10) / ch as u64 * i,
                        Ok(ea_xa::Kind::EaXa) => pos + 0x0c + 8,
                        Ok(ea_xa::Kind::EaXaInt) => pos + 0x0c + 8 + samples / 28 * 0x0f * i,
                        // coding_EA_XA_V2 has no v0 layout in vgmstream (the stream fails)
                        Ok(ea_xa::Kind::EaXaV2) | Ok(ea_xa::Kind::Pcm8UInt) | Ok(ea_xa::Kind::Pcm16Group { .. }) => return Ok(None),
                        Err(_) => pos,
                    };
                    starts.push(s);
                }
            } else {
                for i in 0..ch as u64 {
                    let rel = r.u32(pos + 0x0c + 4 * i, ea.big_endian)? as u64;
                    let mut s = pos + 0x0c + 4 * ch as u64 + rel;
                    if flag_adpcm {
                        s += 4;
                    }
                    starts.push(s);
                }
            }
            // Offsets must stay inside the file.
            if starts.iter().any(|&s| s >= size) {
                return Ok(None);
            }
            blocks.push(ea_xa::Block { samples: samples.min(u32::MAX as u64) as u32, starts, reset: false });
            total += samples;
            audio_blocks += 1;
        }
        pos += bsize;
        end = pos;
    }
    if audio_blocks == 0 {
        return Ok(None);
    }
    let data_start = start;
    let data_len = end.max(start + 1) - data_start;
    for b in blocks.iter_mut() {
        for s in b.starts.iter_mut() {
            *s -= data_start;
        }
    }
    let samples = ea.num_samples as u64;
    let loops = if ea.loop_flag { vgm_loop(ea.loop_start as i64, ea.loop_end as i64, samples) } else { None };
    let (codec, note) = match kind {
        Ok(k) => {
            if k == ea_xa::Kind::Psx {
                // reject garbage: the first frames of the first block
                let first = blocks[0].starts[0] + data_start;
                let probe = r.b(first, 0x40)?;
                if !crate::codecs::psx::plausible(&probe) {
                    return Ok(None);
                }
            }
            (Codec::EaXa(ea_xa::Params { kind: k, blocks: Arc::new(blocks) }), None)
        }
        Err(name) => (Codec::None, Some(format!("{name} audio isn't supported"))),
    };
    Ok(Some(Sound {
        channels: ch as u16,
        rate: ea.sample_rate as u32,
        samples,
        loops,
        data: Data::at(entry, data_start, data_len),
        codec,
        note,
        end,
    }))
}

/// Bytes a flat (bank) channel needs for `samples`.
fn flat_need(kind: ea_xa::Kind, samples: u64, channels: u64) -> u64 {
    use ea_xa::Kind::*;
    let frames = samples.div_ceil(28);
    match kind {
        EaXa => frames * if channels > 1 { 0x1e } else { 0x0f },
        EaXaInt => frames * 0x0f,
        EaXaV2 => frames * 0x3d,
        Psx => frames * 0x10,
        Pcm16 { .. } => samples * 2,
        Pcm16Int { .. } => samples * 2 * channels,
        Pcm16Group { group, .. } => samples * 2 * group as u64,
        Pcm8 => samples,
        Pcm8Int | Pcm8UInt => samples * channels,
    }
}

/// Result of a BNK lookup.
pub struct BnkSound {
    pub sound: Sound,
    /// Offset of the sound's header (for naming/ordering).
    pub header: u64,
}

/// A BNK's header: (big endian, version, sound count, table offset, header size).
pub fn bnk_info(r: &mut Reader, offset: u64) -> io::Result<Option<(bool, u8, u16, u64, u64)>> {
    let mut r = Rd(r);
    let magic = r.id(offset)?;
    let be = if magic == id(b"BNKb") {
        true
    } else if magic == id(b"BNKl") {
        false
    } else {
        return Ok(None);
    };
    let version = r.u8(offset + 4)?;
    let num = r.u16(offset + 6, be)?;
    let (table, header_size) = match version {
        0x02 => (0x0c, r.u32(offset + 8, be)? as u64),
        0x04 | 0x05 => (0x14, r.size()),
        _ => return Ok(None),
    };
    Ok(Some((be, version, num, table, header_size)))
}

/// parse_bnk_header: sound `target` of the BNK at `offset`. `embedded` targets the table
/// index directly (even empty entries); otherwise it counts only non-empty ones.
pub fn load_bnk(r: &mut Reader, entry: usize, offset: u64, target: usize, embedded: bool) -> io::Result<Option<BnkSound>> {
    let Some((be, version, num, table, header_size)) = bnk_info(r, offset)? else { return Ok(None) };
    let mut r = Rd(r);
    let size = r.size();
    let mut header_offset = 0u64;
    if embedded {
        if target >= num as usize {
            return Ok(None);
        }
        let e = offset + table + 4 * target as u64;
        header_offset = e + r.u32(e, be)? as u64;
    } else {
        let mut real = 0;
        for i in 0..num as u64 {
            let e = offset + table + 4 * i;
            let t = r.u32(e, be)? as u64;
            if t != 0 {
                if target == real {
                    header_offset = e + t;
                }
                real += 1;
            }
        }
    }
    if header_offset == 0 || header_offset >= size {
        return Ok(None);
    }
    // Header limit as vgmstream has it for the bank on its own (v2: the bank's stored size;
    // later: up to the end of the file).
    let max_len = if version == 2 { (offset + header_size) as i64 } else { header_size as i64 } - header_offset as i64;
    let Some(mut ea) = parse_variable_header(&mut r, header_offset, max_len, version != 0)? else { return Ok(None) };
    if offset != 0 {
        for i in 0..ea.channels as usize {
            ea.offsets[i] += offset;
        }
    }
    if !basic_checks(&ea) {
        return Ok(None);
    }
    let Some(kind) = kind_of(&ea) else { return Ok(None) };
    let ch = ea.channels as usize;
    let samples = ea.num_samples as u64;

    // Channel starts (init_vgmstream_ea_variable_header, bank part).
    let mut starts = vec![0u64; ch];
    if ea.block_config & FLAG_OFFSETS == 0 {
        use ea_xa::Kind::*;
        for (i, s) in starts.iter_mut().enumerate() {
            let i = i as u64;
            *s = match kind {
                Ok(EaXa) => ea.offsets[0],
                Ok(EaXaInt) => ea.offsets[0] * (samples / 28 * 0x0f) * i, // (sic)
                Ok(Pcm8Int) | Ok(Pcm16Int { .. }) => ea.offsets[0] + if ea.bps == 8 { 1 } else { 2 } * i,
                Ok(Pcm8) | Ok(Pcm16 { .. }) => ea.offsets[0] + samples * if ea.bps == 8 { 1 } else { 2 } * i,
                Ok(Psx) => ea.offsets[0] + samples / 28 * 0x10 * i,
                Err(_) => ea.offsets[0],
                Ok(EaXaV2) | Ok(Pcm8UInt) | Ok(Pcm16Group { .. }) => return Ok(None),
            };
        }
    } else if ea.platform == PLATFORM_PS2 && ea.flag_value & 0x100 != 0 {
        for (i, s) in starts.iter_mut().enumerate() {
            *s = ea.offsets[i] + 0x10;
        }
    } else {
        starts[..ch].copy_from_slice(&ea.offsets[..ch]);
    }
    if starts.iter().any(|&s| s >= size) {
        return Ok(None);
    }
    let lo = *starts.iter().min().unwrap();
    let need = match kind {
        Ok(k) => flat_need(k, samples, ch as u64),
        Err(_) => 0,
    };
    let hi = starts.iter().map(|&s| (s + need).min(size)).max().unwrap().max(lo + 1);
    let loops = if ea.loop_flag { vgm_loop(ea.loop_start as i64, ea.loop_end as i64, samples) } else { None };
    let (codec, note) = match kind {
        Ok(k) => {
            if k == ea_xa::Kind::Psx {
                let probe = r.b(starts[0], 0x40.min(need as usize))?;
                if !crate::codecs::psx::plausible(&probe) {
                    return Ok(None);
                }
            }
            let block = ea_xa::Block { samples: samples.min(u32::MAX as u64) as u32, starts: starts.iter().map(|s| s - lo).collect(), reset: false };
            (Codec::EaXa(ea_xa::Params { kind: k, blocks: Arc::new(vec![block]) }), None)
        }
        Err(name) => (Codec::None, Some(format!("{name} audio isn't supported"))),
    };
    Ok(Some(BnkSound {
        sound: Sound { channels: ch as u16, rate: ea.sample_rate as u32, samples, loops, data: Data::at(entry, lo, hi - lo), codec, note, end: hi },
        header: header_offset,
    }))
}

/// Table indexes of the non-empty sounds of the BNK at `offset` (what a standalone bank lists).
pub fn bnk_real_sounds(r: &mut Reader, offset: u64) -> io::Result<Vec<usize>> {
    let Some((be, _, num, table, _)) = bnk_info(r, offset)? else { return Ok(vec![]) };
    let t = r.bytes(offset + table, 4 * num as usize)?;
    Ok((0..num as usize)
        .filter(|&i| {
            let b: [u8; 4] = t[4 * i..4 * i + 4].try_into().unwrap();
            (if be { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }) != 0
        })
        .collect())
}
