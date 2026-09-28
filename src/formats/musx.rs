//! Eurocom MUSX (.sfx/.musx; vgmstream meta/musx.c): music streams (MFX), music banks
//! (MFX_BANK) and sound effect banks (SFX_BANK) from Eurocom games [Sphinx and the Cursed
//! Mummy, Spyro: A Hero's Tail, Predator: Concrete Jungle, 007: Quantum of Solace (PS2)...].
//! PS2/PSP ones are PS-ADPCM (0x80 interleave); other platforms' codecs (DAT4 IMA, Xbox IMA,
//! DSP, PCM) are listed but not decoded.

use std::io;

use super::ea_schl::{Rd, vgm_loop};
use super::{Ctx, Found, Parser, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser { name: "MUSX", magics: &[b"MUSX"], magic_at: 0, exts: &[], locate: None, parse };

#[derive(Clone, Copy, PartialEq, Debug)]
enum Form {
    Mfx,
    MfxBank,
    SfxBank,
    Sbnk,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum MCodec {
    Psx,
    Dsp,
    Xbox,
    Ima,
    Dat,
    Ngca,
    Pcm,
}

#[derive(Clone, Default)]
struct Musx {
    big_endian: bool,
    version: u32,
    file_size: u64,
    is_old: bool,
    tables_offset: u64,
    loops_offset: u64,
    stream_offset: u64,
    stream_size: u64,
    coefs_offset: u64,
    codec: Option<MCodec>,
    platform: u32,
    channels: u32,
    sample_rate: u32,
    loop_flag: bool,
    flags: u32,
    loop_start: i64,
    loop_end: i64,
    num_samples: i64,
    loop_start_sample: i64,
    loop_end_sample: i64,
}

fn id(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

/// Reads relative to the MUSX start.
struct M<'a, 'b> {
    r: &'a mut Rd<'b>,
    base: u64,
}

impl M<'_, '_> {
    fn u32(&mut self, o: u64, be: bool) -> io::Result<u32> {
        self.r.u32(self.base + o, be)
    }
    fn s32(&mut self, o: u64, be: bool) -> io::Result<i64> {
        Ok(self.r.u32(self.base + o, be)? as i32 as i64)
    }
    fn u16(&mut self, o: u64, be: bool) -> io::Result<u16> {
        self.r.u16(self.base + o, be)
    }
    fn u8(&mut self, o: u64) -> io::Result<u8> {
        self.r.u8(self.base + o)
    }
    fn id(&mut self, o: u64) -> io::Result<u32> {
        self.r.id(self.base + o)
    }
    fn size(&self) -> u64 {
        self.r.size() - self.base
    }
}

fn ps_check_format(m: &mut M, offset: u64, max: u64) -> io::Result<bool> {
    let end = (offset + max).min(m.size());
    let mut o = offset;
    while o < end {
        let b = m.r.b(m.base + o, 2)?;
        if (b[0] >> 4) & 0x0f > 5 || b[1] > 7 {
            return Ok(false);
        }
        o += 0x10;
    }
    Ok(true)
}

fn xbox_check_format(m: &mut M, offset: u64, max: u64, channels: u32) -> io::Result<bool> {
    let end = (offset + max).min(m.size());
    let mut o = offset;
    while o < end {
        for ch in 0..channels as u64 {
            if m.u16(o + 4 * ch + 2, false)? > 88 {
                return Ok(false);
            }
        }
        o += 0x24 * channels as u64;
    }
    Ok(true)
}

fn parse_stream(m: &mut M, x: &mut Musx) -> io::Result<bool> {
    let be = x.big_endian;
    if x.platform == 0 {
        if be {
            x.platform = id(b"GC02");
        } else {
            let channels = if x.channels == 0 { 2 } else { x.channels };
            let max = 0x5000.min(x.stream_size);
            x.platform = if ps_check_format(m, x.stream_offset, max)? {
                id(b"PS2_")
            } else if xbox_check_format(m, x.stream_offset, max, channels)? {
                id(b"XB02")
            } else {
                id(b"PC02")
            };
        }
    }
    if x.tables_offset != 0 && x.loops_offset != 0 {
        let count = m.u32(x.loops_offset + 4, be)? as u64;
        if count > 0x10000 {
            return Ok(false);
        }
        let cues2 = x.loops_offset + m.u32(x.loops_offset + 0x0c, be)? as u64;
        for i in 0..count {
            let (o1, t, o2) = if x.is_old {
                (m.u32(cues2 + i * 0x20 + 4, be)?, m.u32(cues2 + i * 0x20 + 8, be)?, m.u32(cues2 + i * 0x20 + 0x14, be)?)
            } else {
                (m.u32(cues2 + i * 0x14 + 4, be)?, m.u32(cues2 + i * 0x14 + 8, be)?, m.u32(cues2 + i * 0x14 + 0x0c, be)?)
            };
            if t == 6 || t == 7 {
                x.loop_start = o2 as i64;
                x.loop_end = o1 as i64;
                x.loop_flag = true;
                break;
            }
        }
    } else if x.loops_offset != 0 && m.u32(x.loops_offset, true)? != 0xABABABAB {
        x.flags = m.u32(x.loops_offset + 4, false)?;
        x.loop_end_sample = m.s32(x.loops_offset + 0x10, false)?;
        x.loop_start_sample = m.s32(x.loops_offset + 0x14, false)?;
        x.loop_end = m.s32(x.loops_offset + 0x18, false)?;
        x.loop_start = m.s32(x.loops_offset + 0x1c, false)?;
        x.num_samples = x.loop_end_sample;
        x.loop_flag = x.loop_start_sample >= 0;
    }
    if x.stream_size == 0 {
        x.stream_size = x.file_size.saturating_sub(x.stream_offset);
        if x.stream_size > 0x800 {
            let at = x.stream_offset + x.stream_size - 0x800;
            let buf = m.r.b(m.base + at, 0x800)?;
            let mut pos = 0x800 - 4;
            while pos > 0 {
                if buf[pos..pos + 4] != [0xAB; 4] {
                    break;
                }
                x.stream_size -= 4;
                pos -= 4;
            }
        }
    }
    let p = x.platform.to_be_bytes();
    let (ch, rate, codec) = match &p {
        b"PS2_" => (2, 32000, MCodec::Psx),
        b"GC__" => (2, 32000, MCodec::Dat),
        b"GC02" => (2, 32000, if x.coefs_offset != 0 { MCodec::Dsp } else { MCodec::Ima }),
        b"XB__" | b"XB1_" => (2, 44100, MCodec::Dat),
        b"XB02" => (2, 44100, MCodec::Xbox),
        b"PSP_" => (2, 32768, MCodec::Psx),
        b"WII_" | b"XE__" => (2, 32000, MCodec::Dat),
        b"PS3_" | b"PC__" => (2, if x.version == 10 && x.flags & 2 != 0 { 32000 } else { 44100 }, MCodec::Dat),
        b"PC02" => (2, 32000, MCodec::Ima),
        _ => return Ok(false),
    };
    x.codec = Some(codec);
    if x.channels == 0 {
        x.channels = ch;
    }
    if x.sample_rate == 0 {
        x.sample_rate = rate;
    }
    Ok(true)
}

/// parse_musx for subsong `target` (1-based): the header, and the subsong count.
fn parse_musx(m: &mut M, target: u32) -> io::Result<Option<(Musx, u32)>> {
    let mut x = Musx { version: m.u32(8, false)?, file_size: m.u32(0x0c, false)? as u64, ..Default::default() };
    match x.version {
        201 | 1 => {
            x.tables_offset = 0x10;
            x.big_endian = m.r.guess_be(m.base + 0x10)?;
            x.is_old = true;
        }
        4 | 5 | 6 => {
            x.platform = m.id(0x10)?;
            x.tables_offset = 0x20;
            x.big_endian = x.platform == id(b"GC__");
        }
        10 => {
            x.platform = m.id(0x10)?;
            x.big_endian = [id(b"GC__"), id(b"XE__"), id(b"PS3_"), id(b"WII_")].contains(&x.platform);
        }
        _ => return Ok(None),
    }
    let be = x.big_endian;
    let form;
    if x.tables_offset != 0 {
        let t = x.tables_offset;
        let t1 = m.u32(t, be)? as u64;
        let t2 = m.u32(t + 8, be)?;
        let t4_size = m.u32(t + 0x1c, be)?;
        if t2 == 0 || t2 == 0xABABABAB {
            return Ok(None);
        } else if t4_size != 0 && t4_size != 0xABABABAB {
            form = Form::SfxBank;
        } else if m.u32(t1, be)? < 9 && m.u32(t1 + 8, be)? == 0x14 && m.u32(t1 + 0x10, be)? <= 100 {
            form = Form::Mfx;
        } else if m.u32(t1, be)? == 0 && m.u32(t1 + 4, be)? > m.u32(t1, be)? && m.u32(t1 + 8, be)? > m.u32(t1 + 4, be)? {
            form = Form::MfxBank;
        } else {
            return Ok(None);
        }
    } else if m.id(0x800)? == id(b"SBNK") {
        form = Form::Sbnk;
    } else if m.id(0x800)? == id(b"FORM") || m.id(0x800)? == id(b"ESPD") {
        return Ok(None);
    } else {
        form = Form::Mfx;
    }

    let mut total = 1u32;
    match form {
        Form::Mfx => {
            if x.tables_offset != 0 {
                x.loops_offset = m.u32(x.tables_offset, be)? as u64;
                x.stream_offset = m.u32(x.tables_offset + 8, be)? as u64;
                x.stream_size = m.u32(x.tables_offset + 0x0c, be)? as u64;
            } else {
                if m.u32(0x30, true)? != 0xABABABAB {
                    match &m.id(0x40)?.to_be_bytes() {
                        b"DAT4" | b"DAT5" | b"DAT8" | b"DAT9" => {
                            x.stream_size = m.u32(0x44, false)? as u64;
                            x.channels = m.u32(0x48, false)?;
                            x.sample_rate = m.u32(0x4c, false)?;
                            x.loops_offset = 0x50;
                        }
                        _ => {
                            x.loops_offset = if m.u32(0x30, true)? == 0 && m.u32(0x34, true)? == 0 { 0 } else { 0x30 };
                        }
                    }
                }
                x.stream_offset = 0x800;
                x.stream_size = 0;
            }
            if !parse_stream(m, &mut x)? {
                return Ok(None);
            }
        }
        Form::MfxBank => {
            total = m.u32(x.tables_offset + 4, be)? / 4;
            if total == 0 || target > total {
                return Ok(None);
            }
            let base = m.u32(x.tables_offset, be)? as u64;
            let data = m.u32(x.tables_offset + 8, be)? as u64;
            let t = m.u32(base + (target as u64 - 1) * 4, be)? as u64 + data;
            x.stream_offset = m.u32(t + 4, be)? as u64 + data;
            x.stream_size = m.u32(t + 8, be)? as u64;
            x.loops_offset = t + 0x0c;
            x.channels = 1;
            x.sample_rate = 22050;
            if !parse_stream(m, &mut x)? {
                return Ok(None);
            }
        }
        Form::SfxBank => {
            let head = m.u32(x.tables_offset + 8, be)? as u64;
            let coef = m.u32(x.tables_offset + 0x10, be)? as u64;
            let coef_size = m.u32(x.tables_offset + 0x14, be)?;
            let data = m.u32(x.tables_offset + 0x18, be)? as u64;
            total = m.u32(head, be)?;
            if total == 0 || target > total {
                return Ok(None);
            }
            if x.is_old {
                let t = head + 4 + (target as u64 - 1) * 0x28;
                x.stream_offset = m.u32(t + 4, be)? as u64 + data;
                x.stream_size = m.u32(t + 8, be)? as u64;
                x.sample_rate = m.u32(t + 0x0c, be)?;
                x.coefs_offset = m.u32(t + 0x1c, be)? as u64 + coef;
            } else {
                let t = head + 4 + (target as u64 - 1) * 0x20;
                x.stream_offset = m.u32(t + 4, be)? as u64 + data;
                x.stream_size = m.u32(t + 8, be)? as u64;
                x.sample_rate = m.u32(t + 0x0c, be)?;
                x.coefs_offset = m.u32(t + 0x14, be)? as u64 + coef;
            }
            x.channels = 1;
            if coef_size == 0 {
                x.coefs_offset = 0;
            }
            if !parse_stream(m, &mut x)? {
                return Ok(None);
            }
        }
        Form::Sbnk => {
            let version = m.u32(0x804, be)?;
            if version == 0x2A {
                return Ok(None);
            }
            x.tables_offset = 0x810;
            total = m.u32(x.tables_offset + 0x20, be)?;
            if total == 0 || target > total {
                return Ok(None);
            }
            let head = m.u32(x.tables_offset + 0x24, be)? as u64 + x.tables_offset + 0x24;
            let data = m.u32(x.tables_offset + 0x3c, be)? as u64;
            let t = head + (target as u64 - 1) * 0x1c;
            x.num_samples = m.s32(t + 4, be)?;
            x.loop_start_sample = m.s32(t + 8, be)?;
            x.sample_rate = m.u16(t + 0x0c, be)? as u32;
            let codec = m.u8(t + 0x0e)?;
            x.channels = m.u8(t + 0x0f)? as u32;
            x.stream_offset = m.u32(t + 0x10, be)? as u64 + data;
            x.stream_size = m.u32(t + 0x14, be)? as u64;
            x.loop_start = m.s32(t + 0x18, be)?;
            x.loop_end_sample = x.num_samples;
            x.loop_flag = x.loop_start_sample >= 0;
            x.codec = Some(match codec {
                0x11 => MCodec::Dat,
                0x13 => MCodec::Ngca,
                0x14 => MCodec::Pcm,
                _ => return Ok(None),
            });
        }
    }
    Ok(Some((x, total)))
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let entry = ctx.entry;
    let size = ctx.size();
    let mut rd = Rd(&mut ctx.r);
    if off + 0x20 > size {
        return Ok(vec![]);
    }
    let mut m = M { r: &mut rd, base: off };
    let declared = m.u32(0x0c, false)? as u64;
    if declared < 0x20 || off + declared > size + 0x800 {
        return Ok(vec![]);
    }
    let end = (off + declared).min(size);
    let Some((_, total)) = parse_musx(&mut m, 1)? else { return Ok(vec![]) };
    if total == 0 || total > 10000 {
        return Ok(vec![]);
    }
    let mut found = Vec::new();
    for target in 1..=total {
        let Some((x, _)) = parse_musx(&mut m, target)? else { continue };
        if x.channels == 0 || x.channels > 8 || !sane_rate(x.sample_rate) {
            continue;
        }
        let ch = x.channels as u16;
        let codec = x.codec.unwrap();
        let data_off = off + x.stream_offset;
        if data_off >= end {
            continue;
        }
        let (mut samples, mut ls, mut le) = match codec {
            MCodec::Psx => (
                psx::bytes_to_samples(x.stream_size, ch) as i64,
                (x.loop_start.max(0) as u64 / ch as u64 / 16 * 28) as i64,
                (x.loop_end.max(0) as u64 / ch as u64 / 16 * 28) as i64,
            ),
            _ => (0, 0, 0),
        };
        if codec == MCodec::Psx && x.loop_start < 0 {
            ls = -1;
        }
        if x.num_samples != 0 {
            samples = x.num_samples;
        }
        if x.loop_flag && x.loop_start_sample != 0 {
            ls = x.loop_start_sample;
        }
        if x.loop_flag && x.loop_end_sample != 0 {
            le = x.loop_end_sample;
        }
        if samples <= 0 {
            if codec == MCodec::Psx {
                continue;
            }
            // (not decoded: a rough length for the listing)
            samples = match codec {
                MCodec::Pcm => x.stream_size / 2 / ch as u64,
                _ => x.stream_size * 2 / ch as u64,
            } as i64;
        }
        let samples = samples as u64;
        // what the decoder reads: the stream, or more when the sample count asks for it
        let rows = (samples.div_ceil(28) * 16 * ch as u64).next_multiple_of(0x80 * ch as u64);
        let len = x.stream_size.max(rows).min(end - data_off);
        let mut t = match codec {
            MCodec::Psx => {
                let probe = m.r.b(data_off, 0x100.min(len as usize))?;
                if !psx::plausible(&probe) {
                    continue;
                }
                Track::new(entry, off, "MUSX", ch, x.sample_rate, samples, Data::at(entry, data_off, len), Codec::Psx(psx::Params::interleaved(0x80)))
            }
            other => {
                let mut t = Track::new(entry, off, "MUSX", ch, x.sample_rate, samples, Data::at(entry, data_off, len), Codec::None);
                t.note = Some(format!("{other:?} audio (non-PS2 MUSX) isn't supported"));
                t
            }
        };
        if x.loop_flag {
            if let Some((a, b)) = vgm_loop(ls, le, samples) {
                t = t.looped(a, b);
            }
        }
        found.push(Found::new(t, end));
    }
    Ok(found)
}
