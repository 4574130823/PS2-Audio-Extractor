//! .bnk: Sony's 989SND/SCREAM sound banks (vgmstream meta/bnk_sony.c), the PS2-era
//! versions: SBv2 (bank v1) and SBlk v1-v5, plus the PS3/PS4/Vita SBlk v8-v0x10 banks'
//! PS-ADPCM and PCM streams. ATRAC9/MPEG/HEVAG streams are listed with a note; SBlk
//! v0x1a+ (PS5/PC) banks aren't ported.

use std::io;

use super::vag::{ps_find_loop, vgm_loop};
use super::{Ctx, Found, Parser, label, sane_rate};
use crate::codecs::{Codec, pcm, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "BNK",
    magics: &[b"SBv2", b"SBlk", b"klBS"],
    magic_at: 0,
    exts: &[],
    locate: Some(locate),
    parse,
};

/// The bank header is up to 0x20 bytes before its SBv2/SBlk block and points at it.
fn locate(ctx: &mut Ctx, hit: u64) -> io::Result<Option<u64>> {
    for d in (4..=0x20u64).step_by(4) {
        let Some(c) = hit.checked_sub(d) else { break };
        let b = ctx.bytes(c, 0x10)?;
        let le = u32::from_le_bytes(b[0..4].try_into().unwrap());
        let be = u32::from_be_bytes(b[0..4].try_into().unwrap());
        let big = le > be;
        let rd = |at: usize| if big { u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) } else { u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) };
        if matches!(rd(0), 1 | 3) && matches!(rd(4), 2 | 3) && rd(8) as u64 == d {
            return Ok(Some(c));
        }
    }
    Ok(None)
}

#[derive(Clone, Copy, PartialEq)]
enum Codec_ {
    Psx,
    Pcm16,
    Unsupported(&'static str),
}

struct Bank {
    big: bool,
    off: u64,
    sblk: u64,
    version: u32,
    table1: u64,
    table2: u64,
    table3: u64,
    table4: u64,
    sounds: u32,
    grains: u32,
    table1_entry_size: u64,
    table1_suboffset: u64,
}

impl Bank {
    fn u8(&self, ctx: &mut Ctx, at: u64) -> io::Result<u8> {
        ctx.u8(self.off + at)
    }
    fn u16(&self, ctx: &mut Ctx, at: u64) -> io::Result<u16> {
        let b = ctx.bytes(self.off + at, 2)?;
        Ok(if self.big { u16::from_be_bytes([b[0], b[1]]) } else { u16::from_le_bytes([b[0], b[1]]) })
    }
    fn u32(&self, ctx: &mut Ctx, at: u64) -> io::Result<u32> {
        let b: [u8; 4] = ctx.bytes(self.off + at, 4)?.try_into().unwrap();
        Ok(if self.big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
    }
    fn string(&self, ctx: &mut Ctx, at: u64) -> io::Result<Option<String>> {
        Ok(label(&ctx.bytes(self.off + at, 0xff)?))
    }
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    let le = u32::from_le_bytes(h[0..4].try_into().unwrap());
    let be = u32::from_be_bytes(h[0..4].try_into().unwrap());
    let big = le > be;
    let rd = |at: usize| if big { u32::from_be_bytes(h[at..at + 4].try_into().unwrap()) } else { u32::from_le_bytes(h[at..at + 4].try_into().unwrap()) };
    let version = rd(0);
    let sections = rd(4);
    let sblk = rd(8) as u64;
    if !matches!(version, 1 | 3) || !(2..=3).contains(&sections) || sblk > 0x20 || sblk < 0x18 {
        return Ok(vec![]);
    }
    let data_offset = rd(0x10) as u64;
    let data_size = rd(0x14) as u64;
    let avail = ctx.size() - off;
    if data_offset < sblk || data_offset > avail || data_size == 0 {
        return Ok(vec![]);
    }
    let mut b = Bank {
        big,
        off,
        sblk,
        version: 0,
        table1: 0,
        table2: 0,
        table3: 0,
        table4: 0,
        sounds: 0,
        grains: 0,
        table1_entry_size: 0,
        table1_suboffset: 0,
    };
    let id = ctx.bytes(off + sblk, 4)?;
    if version == 1 {
        if big || &id[..] != b"SBv2" {
            return Ok(vec![]);
        }
        b.version = match ctx.u32le(off + sblk + 4)? {
            2 | 0x0200_0000 => 2,
            _ => return Ok(vec![]),
        };
    } else {
        if &id[..] != if big { b"klBS" } else { b"SBlk" } {
            return Ok(vec![]);
        }
        b.version = b.u32(ctx, sblk + 4)?;
    }
    if !tables(ctx, &mut b)? {
        return Ok(vec![]);
    }
    // bounds of the tables
    for t in [b.table1, b.table2, b.table3] {
        if t < sblk || t >= avail {
            return Ok(vec![]);
        }
    }

    // subsongs: (table2 entry offset, table3 entry offset)
    let mut subs: Vec<(u64, u64)> = Vec::new();
    if b.grains > 20000 {
        return Ok(vec![]);
    }
    match b.version {
        1 => {
            for i in 0..b.grains as u64 {
                if b.u32(ctx, b.table2 + i * 0x28)? == 1 {
                    subs.push((0, i * 0x28 + 8));
                }
            }
        }
        2 => {
            for i in 0..b.grains as u64 {
                let n = b.u8(ctx, b.table2 + i * 8)?;
                let sub = b.u32(ctx, b.table2 + i * 8 + 4)? as u64;
                for j in 0..n as u64 {
                    subs.push((0, sub.wrapping_sub(b.table3 - b.sblk).wrapping_add(j * 0x18)));
                }
            }
        }
        _ => {
            for i in 0..b.grains as u64 {
                let v = b.u32(ctx, b.table2 + i * 8)?;
                if v >> 16 == 0x0100 {
                    subs.push((i * 8, (v & 0xFFFF) as u64));
                }
            }
        }
    }
    let mut total = subs.len();
    if total == 0 {
        return Ok(vec![]);
    }

    let mut found = Vec::new();
    let end = (off + data_offset + data_size).min(ctx.size());
    for &(t2, t3) in subs.clone().iter() {
        let sndh = b.table3.wrapping_add(t3);
        if sndh + if b.version <= 9 { 0x18 } else { 0x50 } > avail {
            continue;
        }
        let (flags, stream_offset, mut stream_size, rate);
        if b.version <= 9 {
            let center_note = b.u8(ctx, sndh + 2)?;
            let center_fine = b.u8(ctx, sndh + 3)?;
            flags = b.u16(ctx, sndh + 0x0e)?;
            stream_offset = b.u32(ctx, sndh + 0x10)? as u64;
            stream_size = if b.version >= 3 { b.u32(ctx, sndh + 0x14)? as u64 } else { 0 };
            let pitch = spu2_note_to_pitch(60, 0, center_note, center_fine);
            rate = (48000 * pitch / 4096) as u32;
        } else {
            flags = b.u16(ctx, sndh + 0x12)?;
            stream_offset = b.u32(ctx, sndh + 0x44)? as u64;
            stream_size = b.u32(ctx, sndh + 0x48)? as u64;
            let bits = b.u32(ctx, sndh + 0x4c)?;
            rate = f32::from_bits(bits) as i32 as u32;
        }
        let Ok(name) = names(ctx, &b, t2)? else { continue };

        let mut start = data_offset + stream_offset;
        let mut channels = 1u64;
        let (mut loop_start, mut loop_length, mut loop_end) = (0i32, 0i32, 0i32);
        let mut num_samples = 0i32;
        let mut extradata = 0u64;
        let mut interleave = 0u64;
        let codec;
        if b.version <= 5 {
            if b.version <= 3 && stream_size == 0 && flags & 0x80 == 0 {
                // no size: up to an empty frame or an end frame
                stream_size = 0x10;
                let max = ctx.size();
                let mut o = off + start + 0x10;
                while o < max {
                    let f = ctx.bytes(o, 0x10)?;
                    if f.iter().all(|&x| x == 0) {
                        break;
                    }
                    stream_size += 0x10;
                    let (a, c) = (u32::from_be_bytes(f[0..4].try_into().unwrap()), u32::from_be_bytes(f[4..8].try_into().unwrap()));
                    if (a == 0x0007_7777 && c == 0x7777_7777) || (a == 0x0007_0000 && c == 0) {
                        break;
                    }
                    o += 0x10;
                }
            }
            // two subsongs used as one stereo stream [ATV Offroad Fury, Fat Princess]
            if total == 2 && stream_size * 2 == data_size {
                channels = 2;
                stream_size *= 2;
                total = 1;
                start -= stream_offset;
            }
            interleave = stream_size / channels;
            if flags & 0x80 != 0 {
                codec = Codec_::Pcm16;
            } else if flags & 0x1000 != 0 {
                codec = Codec_::Unsupported("MPEG in Sony BNK isn't supported");
            } else {
                if let Some((a, e)) = ps_find_loop(ctx, off + start, stream_size, channels, interleave, false)? {
                    loop_start = a as i32;
                    loop_end = e as i32;
                }
                codec = Codec_::Psx;
            }
        } else if b.version <= 9 {
            let subtype = b.u32(ctx, start)?;
            extradata = b.u32(ctx, start + 4)? as u64 + 8;
            codec = match subtype {
                0 => Codec_::Psx,
                1 | 4 => {
                    channels = if subtype == 1 { 1 } else { 2 };
                    Codec_::Pcm16
                }
                2 | 3 | 5 => Codec_::Unsupported(if big { "MPEG in Sony BNK isn't supported" } else { "ATRAC9 in Sony BNK isn't supported" }),
                _ => continue,
            };
        } else {
            let subtype = b.u32(ctx, start)?;
            extradata = b.u32(ctx, start + 8)? as u64 + 0x10;
            let mut pcm_psx = |ctx: &mut Ctx, ch: &mut u64| -> io::Result<()> {
                num_samples = b.u32(ctx, start + 0x10)? as i32;
                *ch = b.u32(ctx, start + 0x14)? as u64;
                loop_start = b.u32(ctx, start + 0x18)? as i32;
                loop_length = b.u32(ctx, start + 0x1c)? as i32;
                Ok(())
            };
            codec = if b.version == 0x0c {
                match (big, subtype) {
                    (true, 0) | (false, 0x10000) => {
                        pcm_psx(ctx, &mut channels)?;
                        Codec_::Psx
                    }
                    (true, 1) | (false, 0) | (false, 1) => {
                        pcm_psx(ctx, &mut channels)?;
                        Codec_::Pcm16
                    }
                    (true, 3) => Codec_::Unsupported("MPEG in Sony BNK isn't supported"),
                    _ => continue,
                }
            } else {
                match subtype {
                    1 | 4 => {
                        pcm_psx(ctx, &mut channels)?;
                        Codec_::Pcm16
                    }
                    0 | 3 => Codec_::Unsupported("HEVAG in Sony BNK isn't supported"),
                    2 | 5 | 0x30000..=0x30002 => Codec_::Unsupported("ATRAC9 in Sony BNK isn't supported"),
                    _ => continue,
                }
            };
        }
        let _ = num_samples;
        start += extradata;
        stream_size = stream_size.wrapping_sub(extradata);
        if loop_start < 0 {
            loop_start = 0;
            loop_length = 0;
        }
        if loop_length != 0 {
            loop_end = loop_start.wrapping_add(loop_length);
        }
        let looped = loop_start >= 0 && loop_end > 0;
        if channels == 0 || channels > 8 || !sane_rate(rate) || stream_size == 0 || stream_size > avail {
            continue;
        }
        let data_at = off + start;
        let (samples, c) = match codec {
            Codec_::Psx => {
                if !psx::plausible(&ctx.bytes(data_at, 0x100.min(stream_size as usize))?) {
                    continue;
                }
                let il = if interleave == 0 { 0x10 } else { interleave };
                (stream_size / channels / 16 * 28, Codec::Psx(psx::Params::interleaved(il)))
            }
            Codec_::Pcm16 => {
                let il = if interleave == 0 { 2 } else { interleave };
                let p = if big { pcm::Params::be16(if il == 2 { 0 } else { il }) } else { pcm::Params::le16(if il == 2 { 0 } else { il }) };
                (stream_size / channels / 2, Codec::Pcm(p))
            }
            Codec_::Unsupported(_) => (0, Codec::None),
        };
        let mut t = Track::new(ctx.entry, off, "BNK", channels as u16, rate, samples, Data::at(ctx.entry, data_at, stream_size), c);
        if let Codec_::Unsupported(n) = codec {
            t.note = Some(n.into());
        }
        if looped {
            t = vgm_loop(t, loop_start as i64, loop_end as i64);
        }
        found.push(Found::new(t, end).label(name));
        if total == 1 && subs.len() == 2 {
            break; // the stereo pair is one stream
        }
    }
    Ok(found)
}

/// vgmstream's `process_tables`.
fn tables(ctx: &mut Ctx, b: &mut Bank) -> io::Result<bool> {
    let s = b.sblk;
    match b.version {
        1 => {
            b.sounds = b.u16(ctx, s + 0x16)? as u32;
            b.grains = b.u16(ctx, s + 0x18)? as u32;
            b.table1 = s + b.u32(ctx, s + 0x1c)? as u64;
            b.table2 = s + b.u32(ctx, s + 0x20)? as u64;
            b.table3 = b.table2;
        }
        2 => {
            b.sounds = b.u16(ctx, s + 0x14)? as u32;
            b.grains = b.u16(ctx, s + 0x16)? as u32;
            b.table1 = s + b.u32(ctx, s + 0x1c)? as u64;
            b.table2 = s + b.u32(ctx, s + 0x20)? as u64;
            b.table3 = s + b.u32(ctx, s + 0x24)? as u64;
        }
        3 | 4 | 5 | 8 | 9 => {
            b.sounds = b.u16(ctx, s + 0x16)? as u32;
            b.grains = b.u16(ctx, s + 0x18)? as u32;
            b.table1 = s + b.u32(ctx, s + 0x1c)? as u64;
            b.table2 = s + b.u32(ctx, s + 0x20)? as u64;
            b.table3 = s + b.u32(ctx, s + 0x34)? as u64;
            b.table4 = s + b.u32(ctx, s + 0x38)? as u64;
            b.table1_entry_size = 0x0c;
            b.table1_suboffset = 0x08;
        }
        0x0c..=0x10 => {
            b.table1 = s + b.u32(ctx, s + 0x18)? as u64;
            b.table2 = s + b.u32(ctx, s + 0x1c)? as u64;
            b.table3 = s + b.u32(ctx, s + 0x2c)? as u64;
            b.table4 = s + b.u32(ctx, s + 0x30)? as u64;
            b.sounds = b.u16(ctx, s + 0x38)? as u32;
            b.grains = b.u16(ctx, s + 0x3a)? as u32;
            b.table1_entry_size = 0x24;
            b.table1_suboffset = 0x0c;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// vgmstream's `process_names`: the name of the sound that plays this grain.
/// `Err(None)`-like failure is `Ok(Err(()))`: vgmstream fails the subsong (v3 names not found).
fn names(ctx: &mut Ctx, b: &Bank, t2: u64) -> io::Result<Result<Option<String>, ()>> {
    if b.table4 <= b.sblk || b.sounds > 20000 {
        return Ok(Ok(None));
    }
    let t4 = b.table4;
    let mut entry: i64 = -1;
    match b.version {
        3 | 4 | 5 => {
            for i in 0..b.sounds as u64 {
                let eo = b.u32(ctx, b.table1 + i * b.table1_entry_size + 8)? as u64;
                let ec = b.u8(ctx, b.table1 + i * b.table1_entry_size + 4)? as u64;
                if t2 >= eo && t2 < eo + ec * 8 {
                    entry = i as i64;
                    break;
                }
            }
            if b.version == 3 {
                let entries = t4 + 0x18;
                let names_at = t4 + b.u32(ctx, t4 + 8)? as u64;
                for i in 0..32u64 {
                    let idx = b.u16(ctx, entries + i * 2)? as u64;
                    let mut so = names_at + idx * 0x14;
                    let mut guard = 0;
                    while b.u8(ctx, so)? != 0 && guard < 10000 {
                        let n = ctx.bytes(b.off + so, 0x14)?;
                        if (n[0] as u64 + n[4] as u64 + n[8] as u64 + n[0x0c] as u64) & 0x1f != i {
                            return Ok(Err(()));
                        }
                        let id = if b.big { u16::from_be_bytes([n[0x10], n[0x11]]) } else { u16::from_le_bytes([n[0x10], n[0x11]]) };
                        if id as i64 == entry {
                            return Ok(Ok(label(&n[..0x10])));
                        }
                        so += 0x14;
                        guard += 1;
                    }
                }
                return Ok(Err(()));
            }
            let entries = t4 + b.u32(ctx, t4 + 8)? as u64;
            let names_at = t4 + b.u32(ctx, t4 + 0x0c)? as u64;
            for i in 0..b.sounds as u64 {
                if b.u16(ctx, entries + i * 0x10 + 0x0c)? as i64 == entry {
                    let at = names_at + b.u32(ctx, entries + i * 0x10)? as u64;
                    return Ok(Ok(b.string(ctx, at)?));
                }
            }
        }
        8..=0x10 => {
            for i in 0..b.sounds as u64 {
                let eo = b.u32(ctx, b.table1 + i * b.table1_entry_size + b.table1_suboffset)? as u64;
                if eo <= t2 {
                    entry = i as i64;
                }
            }
            let entries = t4 + b.u32(ctx, t4 + 8)? as u64;
            let names_at = entries + 0x10 * b.sounds as u64;
            for i in 0..b.sounds as u64 {
                if b.u16(ctx, entries + i * 0x10 + 0x0c)? as i64 == entry {
                    let at = names_at + b.u32(ctx, entries + i * 0x10)? as u64;
                    return Ok(Ok(b.string(ctx, at)?));
                }
            }
        }
        _ => {}
    }
    Ok(Ok(None))
}

// ------------------------------------------------------------------------ SPU2 pitch

const NOTE_PITCH: [u16; 12] = [0x8000, 0x879C, 0x8FAC, 0x9837, 0xA145, 0xAADC, 0xB504, 0xBFC8, 0xCB2F, 0xD744, 0xE411, 0xF1A1];

const FINE_PITCH: [u16; 128] = [
    0x8000, 0x800E, 0x801D, 0x802C, 0x803B, 0x804A, 0x8058, 0x8067, 0x8076, 0x8085, 0x8094, 0x80A3, 0x80B1, 0x80C0, 0x80CF, 0x80DE,
    0x80ED, 0x80FC, 0x810B, 0x811A, 0x8129, 0x8138, 0x8146, 0x8155, 0x8164, 0x8173, 0x8182, 0x8191, 0x81A0, 0x81AF, 0x81BE, 0x81CD,
    0x81DC, 0x81EB, 0x81FA, 0x8209, 0x8218, 0x8227, 0x8236, 0x8245, 0x8254, 0x8263, 0x8272, 0x8282, 0x8291, 0x82A0, 0x82AF, 0x82BE,
    0x82CD, 0x82DC, 0x82EB, 0x82FA, 0x830A, 0x8319, 0x8328, 0x8337, 0x8346, 0x8355, 0x8364, 0x8374, 0x8383, 0x8392, 0x83A1, 0x83B0,
    0x83C0, 0x83CF, 0x83DE, 0x83ED, 0x83FD, 0x840C, 0x841B, 0x842A, 0x843A, 0x8449, 0x8458, 0x8468, 0x8477, 0x8486, 0x8495, 0x84A5,
    0x84B4, 0x84C3, 0x84D3, 0x84E2, 0x84F1, 0x8501, 0x8510, 0x8520, 0x852F, 0x853E, 0x854E, 0x855D, 0x856D, 0x857C, 0x858B, 0x859B,
    0x85AA, 0x85BA, 0x85C9, 0x85D9, 0x85E8, 0x85F8, 0x8607, 0x8617, 0x8626, 0x8636, 0x8645, 0x8655, 0x8664, 0x8674, 0x8683, 0x8693,
    0x86A2, 0x86B2, 0x86C1, 0x86D1, 0x86E0, 0x86F0, 0x8700, 0x870F, 0x871F, 0x872E, 0x873E, 0x874E, 0x875D, 0x876D, 0x877D, 0x878C,
];

/// vgmstream's `ps_note_to_pitch` (util/spu_utils.c, from OpenGOAL).
fn ps_note_to_pitch(center_note: i32, center_fine: i32, note: i32, fine: i32) -> u16 {
    let mut fine_idx = fine + center_fine;
    let mut fine_adjust = if fine_idx < 0 { fine_idx + 0x7f } else { fine_idx };
    fine_adjust /= 128;
    let note_adjust = note + fine_adjust - center_note;
    let mut unk3 = note_adjust / 6;
    if note_adjust < 0 {
        unk3 -= 1;
    }
    fine_idx -= fine_adjust * 128;
    let mut unk2 = if note_adjust < 0 { -1 } else { 0 };
    if unk3 < 0 {
        unk3 -= 1;
    }
    unk2 = unk3 / 2 - unk2;
    let mut unk1 = unk2 - 2;
    let mut note_idx = note_adjust - unk2 * 12;
    if note_idx < 0 || (note_idx == 0 && fine_idx < 0) {
        note_idx += 12;
        unk1 = unk2 - 3;
    }
    if fine_idx < 0 {
        note_idx = (note_idx - 1) + fine_adjust;
        fine_idx += (fine_adjust + 1) * 128;
    }
    let note_idx = note_idx.clamp(0, 11) as usize;
    let fine_idx = fine_idx.clamp(0, 127) as usize;
    let mut pitch = ((NOTE_PITCH[note_idx] as i32 * FINE_PITCH[fine_idx] as i32) >> 16) as u16;
    if unk1 < 0 {
        pitch = ((pitch as i32 + (1 << (-unk1 - 1))) >> -unk1) as u16;
    }
    pitch
}

fn spu2_note_to_pitch(note: i32, fine: i32, center_note: u8, center_fine: u8) -> i32 {
    let negative = center_note >> 7 != 0;
    let center = if negative { 0x100 - center_note as i32 } else { center_note as i32 };
    let mut pitch = ps_note_to_pitch(center, center_fine as i32, note, fine) as i32;
    if pitch > 0x4000 {
        pitch = 0x4000;
    }
    if !negative {
        pitch = pitch * 44100 / 48000;
    }
    pitch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spu_rates() {
        // 48000 Hz = center note 0xc4 (see vgmstream's notes)
        assert_eq!(48000 * spu2_note_to_pitch(60, 0, 0xc4, 0x00) / 4096, 48000);
    }
}
