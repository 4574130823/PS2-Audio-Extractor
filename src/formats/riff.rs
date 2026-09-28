//! RIFF/WAVE (vgmstream meta/riff.c `init_vgmstream_riff`): PCM16 and PCM8 audio are
//! decoded; loop points come from "smpl", "cue "+"LIST/adtl" (labl/ltxt), "wsmp", "ctrl"
//! and "NXBF" chunks, in vgmstream's order of preference. Other codecs vgmstream reads in
//! RIFF (MS-ADPCM, IMA flavors, AICA, Level-5, ATRAC3/9, Vorbis, MPEG...) are listed with a
//! note. Big endian RIFX (PS3/X360 only) isn't ported.

use std::io;

use super::vag::vgm_loop;
use super::{Ctx, Found, Parser, be32, le16, le32, sane_rate};
use crate::codecs::{Codec, pcm};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "WAV",
    magics: &[b"RIFF"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Coding {
    Pcm16,
    Pcm8U,
    Pcm24,
    Pcm32,
    Float,
    MsAdpcm,
    MsIma,
    XboxIma,
    Ima,
    DviIma,
    Aica,
    AicaInt,
    Level5,
    OkiUm,
    Vorbis,
    Mpeg,
    Atrac,
}

#[derive(Default)]
struct Loops {
    flag: bool,
    smpl: Option<(i32, i32)>,
    cue: bool,
    cue_start: i32,
    cue_end: i32,
    labl: bool,
    start_ms: i64,
    end_ms: i64,
    rgn: bool,
    ctrl: Option<i32>,
    wsmp: Option<(i32, i32)>,
    nxbf: Option<i32>,
}

struct Fmt {
    format: u16,
    channels: u16,
    rate: u32,
    coding: Coding,
    interleave: u64,
}

/// vgmstream's `parse_fmt`: None when vgmstream rejects the file.
fn parse_fmt(ctx: &mut Ctx, at: u64, size: u32) -> io::Result<Option<Fmt>> {
    let f = ctx.bytes(at, 0x28)?;
    let format = le16(&f, 0);
    let channels = le16(&f, 2);
    let rate = le32(&f, 4);
    let mut block_size = le16(&f, 0x0c);
    let bps = le16(&f, 0x0e);
    let extra_size = if size >= 0x10 { le16(&f, 0x10) } else { 0 };
    if channels == 0 {
        return Ok(None);
    }
    let mut interleave = 0;
    let coding = match format {
        0x0000 => {
            if bps != 4 || (block_size != 2 * channels && block_size != channels) {
                return Ok(None);
            }
            Coding::AicaInt
        }
        0x0001 => {
            let c = match bps {
                32 => Coding::Pcm32,
                24 => Coding::Pcm24,
                16 => {
                    if block_size == 2 && channels > 1 {
                        block_size = 2 * channels; // Rayman 2 (DC)
                    }
                    Coding::Pcm16
                }
                8 => Coding::Pcm8U,
                _ => return Ok(None),
            };
            interleave = (block_size / channels) as u64;
            c
        }
        0x0002 => {
            if bps == 4 {
                Coding::MsAdpcm
            } else if bps == 16 && size == 0x14 && block_size == 2 * channels {
                Coding::Ima
            } else {
                return Ok(None);
            }
        }
        0x0003 => {
            if bps != 32 {
                return Ok(None);
            }
            Coding::Float
        }
        0x0011 | 0x0069 => {
            if bps != 4 {
                return Ok(None);
            }
            if format == 0x11 { Coding::MsIma } else { Coding::XboxIma }
        }
        0x0020 => {
            if bps != 4 {
                return Ok(None);
            }
            Coding::Aica
        }
        0x0055 => Coding::Mpeg,
        0x007A => {
            if ctx.ext() != "med" || bps != 4 {
                return Ok(None);
            }
            Coding::MsIma
        }
        0x0300 => {
            if bps != 4 || block_size != 0x400 * channels || size != 0x14 || channels != 1 {
                return Ok(None);
            }
            Coding::DviIma
        }
        0x0555 => {
            interleave = 0x12;
            Coding::Level5
        }
        0x0917 => {
            if bps != 4 || block_size != 0x200 * channels || size != 0x14 || channels != 1 {
                return Ok(None);
            }
            Coding::MsIma
        }
        0x676f..=0x6771 => Coding::Vorbis,
        0x0270 => Coding::Atrac,
        0xFFFE => {
            if extra_size < 0x16 {
                return Ok(None);
            }
            let g1 = le32(&f, 0x18);
            let g2 = ((le16(&f, 0x1c) as u32) << 16) | le16(&f, 0x1e) as u32;
            let (g3, g4) = (be32(&f, 0x20), be32(&f, 0x24));
            if (g1, g2, g3, g4) == (1, 0x10, 0x800000AA, 0x00389B71) {
                if bps != 16 {
                    return Ok(None);
                }
                interleave = 2;
                Coding::Pcm16
            } else if (g1, g2, g3, g4) == (0xE923AABF, 0xCB584471, 0xA119FFFA, 0x01E4CE62)
                || (g1, g2, g3, g4) == (0x47E142D2, 0x36BA4D8D, 0x88FC6165, 0x4F8C836C)
            {
                Coding::Atrac
            } else {
                return Ok(None);
            }
        }
        0xFFFF => {
            if bps != 4 || block_size != channels || size != 0x10 || channels > 2 {
                return Ok(None);
            }
            Coding::OkiUm
        }
        _ => return Ok(None),
    };
    Ok(Some(Fmt { format, channels, rate, coding, interleave }))
}

/// "Marker hh:mm:ss.cc" labels (as vgmstream's sscanf reads them): milliseconds.
fn marker_ms(s: &[u8]) -> i64 {
    let Some(rest) = s.strip_prefix(b"Marker ") else { return -1 };
    let mut vals = [0i64; 4];
    let mut p = 0usize;
    for (i, sep) in [b':', b':', b'.', 0u8].iter().enumerate() {
        let digits = rest[p..].iter().take(2).take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            return -1;
        }
        vals[i] = std::str::from_utf8(&rest[p..p + digits]).unwrap().parse().unwrap();
        p += digits;
        if *sep != 0 {
            if rest.get(p) != Some(sep) {
                return -1;
            }
            p += 1;
        }
    }
    ((vals[0] * 60 + vals[1]) * 60 + vals[2]) * 1000 + vals[3] * 10
}

fn parse_adtl(ctx: &mut Ctx, at: u64, len: u64, l: &mut Loops) -> io::Result<()> {
    let (mut start_found, mut end_found) = (false, false);
    let end = at + len;
    let mut o = at + 4;
    while o < end {
        let c = ctx.bytes(o, 8)?;
        let mut size = le32(&c, 4) as u64;
        if o + 8 + size > end {
            return Ok(());
        }
        o += 8;
        match &c[0..4] {
            b"labl" => {
                let label_size = size as i64 - 4;
                if (0..128).contains(&label_size) {
                    let b = ctx.bytes(o, 4 + label_size as usize)?;
                    let id = le32(&b, 0) as i32;
                    let text: Vec<u8> = b[4..].iter().copied().take_while(|&x| x != 0).collect();
                    let v = marker_ms(&text);
                    if v >= 0 {
                        if id == 1 && !start_found {
                            l.start_ms = v;
                            start_found = true;
                        } else if id == 2 && !end_found {
                            l.end_ms = v;
                            end_found = true;
                        }
                    }
                }
            }
            b"ltxt" if !l.rgn => {
                let b = ctx.bytes(o, 12)?;
                if &b[8..12] == b"rgn " && le32(&b, 0) == 1 {
                    l.rgn = true;
                    let region = le32(&b, 4) as i32;
                    if l.cue && l.cue_end == 0 {
                        l.cue_end = l.cue_start.wrapping_add(region);
                    }
                }
            }
            _ => {}
        }
        if size % 2 == 1 && o + size + 1 <= end {
            size += 1;
        }
        o += size;
    }
    if start_found && end_found {
        l.labl = true;
        l.flag = true;
    }
    if start_found && !end_found {
        l.labl = true;
        l.flag = false;
    }
    if l.start_ms > l.end_ms {
        std::mem::swap(&mut l.start_ms, &mut l.end_ms);
    }
    Ok(())
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x38)?;
    if &h[0..4] != b"RIFF" || &h[8..12] != b"WAVE" {
        return Ok(vec![]);
    }
    let mut riff_size = le32(&h, 4) as u64;
    let avail = ctx.size() - off;
    // vgmstream's size checks need the real file size (a RIFF file on its own); inside other
    // files the RIFF size is all there is.
    let mut file_size = (riff_size + 8).min(avail);
    let mut ignore_riff_size = false;
    if off == 0 && avail != riff_size + 8 {
        let fs = avail;
        let codec = le16(&h, 0x14);
        let mut fixed_fs = fs;
        let mut ok = true;
        if codec & 0xFF00 == 0x6700 && riff_size + 9 == fs {
            riff_size += 1;
        } else if codec == 0x69 && riff_size == fs {
            riff_size -= 8;
        } else if codec == 0x69 && riff_size + 4 == fs {
            riff_size -= 4;
        } else if codec == 0x69 && riff_size + 0x10 == fs {
            riff_size += 8;
        } else if codec == 0 && riff_size + 4 == fs {
            riff_size -= 4;
        } else if codec == 0 && riff_size == fs {
            riff_size -= 8;
        } else if codec == 0 && riff_size + 10 == fs {
            riff_size -= 2;
        } else if codec == 0x300 && riff_size == fs {
            riff_size -= 8;
        } else if codec == 0xFFFE && riff_size + 8 + 0x18 == fs {
            riff_size += 0x18;
        } else if codec == 0x555 {
            let ch = le16(&h, 0x16) as u64;
            let f = riff_size + 8 + 4 * ch.saturating_sub(1);
            if f <= fs && fs - f < 0x10 {
                fixed_fs = f;
                riff_size = f - 8;
            }
        } else if riff_size >= fs && &h[0x24..0x28] == b"NXBF" {
            riff_size = fs - 8;
        } else if codec == 0x11 && riff_size / 4 == le32(&h, 0x30) as u64 {
            riff_size = fs - 8;
        } else if codec == 0xFFFE && riff_size + 8 + 0x30 == fs {
            riff_size += 0x30;
        } else if codec == 0xFFFE && riff_size + 8 + 0x38 == fs {
            riff_size += 0x38;
        } else if codec == 2 && riff_size + 8 + 0x1c == fs {
            riff_size += 0x1c;
        } else if codec == 1 && (riff_size + 0x10 == fs || riff_size + 0x11 == fs || riff_size + 8 == fs + 0x3e || riff_size + 8 == fs + 2) {
            ignore_riff_size = true;
        } else if codec == 0xFFFE && riff_size + 8 + 0x40 == fs {
            fixed_fs -= 0x40;
        } else if codec == 0x11 && fs.wrapping_sub(riff_size).wrapping_sub(8) <= 0x900 && ctx.is(riff_size + 8, b"cont")? {
            riff_size = fs - 8;
        } else if codec == 1 && riff_size % 2 == 1 && riff_size + 9 == fs {
            riff_size += 1;
        } else if codec == 0xFFFF && riff_size + 8 + 0x26 == fs {
            riff_size += 0x26;
        } else {
            ok = false;
        }
        if (ok && fixed_fs == riff_size + 8) || ignore_riff_size {
            file_size = fixed_fs;
        } else {
            // not a standalone RIFF: take it as a RIFF at the start of a bigger file
            riff_size = le32(&h, 4) as u64;
            file_size = (riff_size + 8).min(avail);
        }
    }
    if file_size < 0x2c {
        return Ok(vec![]);
    }

    // chunks
    let mut pos = 0x0cu64;
    let mut fmt: Option<Fmt> = None;
    let mut data: Option<(u64, u64)> = None;
    let mut junk = false;
    let mut fact = 0i32;
    let mut pflt = false;
    let mut l = Loops::default();
    while pos < file_size {
        let c = ctx.bytes(off + pos, 8)?;
        let mut size = le32(&c, 4) as u64;
        if pos + 8 + size > file_size {
            break;
        }
        pos += 8;
        let at = off + pos;
        match &c[0..4] {
            b"fmt " => {
                if fmt.is_some() {
                    return Ok(vec![]);
                }
                match parse_fmt(ctx, at, size as u32)? {
                    Some(f) => {
                        if f.format == 0 && size == 0x12 {
                            size += 2;
                        }
                        fmt = Some(f);
                    }
                    None => return Ok(vec![]),
                }
            }
            b"data" => {
                if data.is_some() {
                    return Ok(vec![]);
                }
                data = Some((pos, size));
            }
            b"JUNK" => junk = true,
            b"fact" => {
                let b = ctx.bytes(at, 8)?;
                if size == 4 {
                    fact = le32(&b, 0) as i32;
                } else if size == 0x10 && &b[4..8] == b"LyN " {
                    return Ok(vec![]);
                } else if matches!(fmt.as_ref().map(|f| f.coding), Some(Coding::Atrac)) && (size == 8 || size == 0x0c) {
                    fact = le32(&b, 0) as i32;
                }
            }
            b"LIST" => {
                if ctx.is(at, b"adtl")? {
                    parse_adtl(ctx, at, size, &mut l)?;
                }
            }
            b"smpl" => {
                let b = ctx.bytes(at, 0x3c)?;
                if le32(&b, 0x1c) == 1 && le32(&b, 0x28) == 0 {
                    l.smpl = Some((le32(&b, 0x2c) as i32, le32(&b, 0x30) as i32));
                    l.flag = true;
                }
            }
            b"wsmp" => {
                let b = ctx.bytes(at, 0x24)?;
                if size >= 0x24 && le32(&b, 0) == 0x14 && (le32(&b, 0x10) as i32) > 0 && le32(&b, 0x14) == 0x10 && le32(&b, 0x18) == 0 {
                    let s = le32(&b, 0x1c) as i32;
                    l.wsmp = Some((s, s.wrapping_add(le32(&b, 0x20) as i32)));
                    l.flag = true;
                }
            }
            b"cue " => {
                if matches!(fmt.as_ref().map(|f| f.coding), Some(Coding::Pcm8U | Coding::Pcm16 | Coding::MsAdpcm)) {
                    let n = ctx.u32le(at)? as i32;
                    if (1..=2).contains(&n) {
                        let b = ctx.bytes(at + 4, 0x18 * n as usize)?;
                        for i in 0..n as usize {
                            let id = le32(&b, i * 0x18);
                            let point = le32(&b, i * 0x18 + 0x14) as i32;
                            match id {
                                1 => l.cue_start = point,
                                2 => l.cue_end = point,
                                _ => {}
                            }
                        }
                        if l.cue_end > 0 && l.cue_start > l.cue_end {
                            std::mem::swap(&mut l.cue_start, &mut l.cue_end);
                        }
                        l.cue = true;
                        l.flag = true;
                    }
                }
            }
            b"NXBF" => {
                let s = ctx.u32le(at + 0x14)? as i32;
                l.nxbf = Some(s);
                l.flag = s >= 0;
            }
            b"pflt" => pflt = true,
            b"ctrl" => {
                let b = ctx.bytes(at, 8)?;
                l.flag = le32(&b, 0) != 0;
                l.ctrl = Some(le32(&b, 4) as i32);
            }
            b"shft" => {
                if size < 4 {
                    return Ok(vec![]);
                }
            }
            b"LySE" | b"dsph" | b"cwav" => return Ok(vec![]),
            _ => {}
        }
        if size % 2 == 1 && pos + size + 1 <= file_size {
            size += 1;
        }
        pos += size;
    }
    let (Some(f), Some((data_pos, data_size))) = (fmt, data) else { return Ok(vec![]) };
    // mutant RIFFs parsed elsewhere by vgmstream
    let ext = ctx.ext();
    if junk && matches!(f.coding, Coding::MsAdpcm | Coding::XboxIma) && (ext == "wav" || ext == "lwav") {
        return Ok(vec![]);
    }
    let data_off = off + data_pos;
    if f.format == 1 && f.rate <= 32000 && ctx.is(data_off, b"MSFC")? {
        let b = ctx.bytes(data_off + 0x34, 12)?;
        if b.iter().all(|&x| x == 0xff) {
            return Ok(vec![]);
        }
    }
    if f.format == 2 && ext == "ckd" {
        return Ok(vec![]);
    }
    if !(1..=16).contains(&f.channels) || !sane_rate(f.rate) {
        return Ok(vec![]);
    }
    let ch = f.channels as u64;
    let (samples, codec, note): (i64, Codec, Option<&str>) = match f.coding {
        Coding::Pcm16 => (
            pcm::bytes_to_samples(data_size, f.channels, 16) as i64,
            Codec::Pcm(pcm::Params::le16(if f.interleave == 2 { 0 } else { f.interleave })),
            None,
        ),
        Coding::Pcm8U => (pcm::bytes_to_samples(data_size, f.channels, 8) as i64, Codec::Pcm(pcm::Params::u8(if f.interleave == 1 { 0 } else { f.interleave })), None),
        Coding::Pcm24 => ((data_size * 8 / ch / 24) as i64, Codec::None, Some("24-bit PCM WAV isn't supported")),
        Coding::Pcm32 => ((data_size * 8 / ch / 32) as i64, Codec::None, Some("32-bit PCM WAV isn't supported")),
        Coding::Float => ((data_size * 8 / ch / 32) as i64, Codec::None, Some("float PCM WAV isn't supported")),
        Coding::Level5 => {
            if !pflt {
                return Ok(vec![]);
            }
            ((data_size / 0x12 / ch * 32) as i64, Codec::None, Some("Level-5 ADPCM WAV isn't supported"))
        }
        Coding::Aica | Coding::AicaInt => ((data_size * 2 / ch) as i64, Codec::None, Some("Yamaha AICA ADPCM WAV isn't supported here")),
        Coding::Ima | Coding::DviIma => ((data_size * 2 / ch) as i64, Codec::None, Some("IMA ADPCM WAV isn't supported here")),
        Coding::MsAdpcm => (fact as i64, Codec::None, Some("MS-ADPCM WAV isn't supported")),
        Coding::MsIma | Coding::XboxIma => (fact as i64, Codec::None, Some("IMA ADPCM WAV isn't supported here")),
        Coding::OkiUm => (fact as i64, Codec::None, Some("OKI ADPCM WAV isn't supported")),
        Coding::Vorbis => (fact as i64, Codec::None, Some("Vorbis in WAV isn't supported")),
        Coding::Mpeg => (fact as i64, Codec::None, Some("MPEG WAV isn't supported")),
        Coding::Atrac => (fact as i64, Codec::None, Some("ATRAC WAV isn't supported")),
    };
    if samples <= 0 && note.is_none() {
        return Ok(vec![]);
    }
    let mut t = Track::new(ctx.entry, off, "WAV", f.channels, f.rate, samples.max(0) as u64, Data::at(ctx.entry, data_off, data_size), codec);
    if let Some(n) = note {
        t.note = Some(n.into());
    }
    if l.flag {
        let n = samples;
        let lp = if let Some((s, e)) = l.smpl {
            let mut e = e as i64 + 1;
            if e - 1 == n {
                e -= 1;
            }
            Some((s as i64, e))
        } else if l.cue && l.labl {
            let mut e = l.cue_end as i64 + 1;
            if e - 1 == n {
                e -= 1;
            }
            Some((l.cue_start as i64, e))
        } else if l.cue && l.rgn {
            Some((l.cue_start as i64, l.cue_end as i64))
        } else if l.labl && l.start_ms >= 0 {
            Some((l.start_ms * f.rate as i64 / 1000, l.end_ms * f.rate as i64 / 1000))
        } else if l.cue {
            Some((l.cue_start as i64, n))
        } else if let (Some(s), Coding::Level5) = (l.ctrl, f.coding) {
            Some((s as i64, n))
        } else if let Some((s, e)) = l.wsmp {
            Some((s as i64, e as i64))
        } else if let (Some(s), Coding::Pcm16) = (l.nxbf, f.coding) {
            Some(((s as u32 / 2 / f.channels as u32) as i64, n))
        } else {
            None
        };
        if let Some((a, b)) = lp {
            t = vgm_loop(t, a, b);
        }
    }
    Ok(vec![Found::new(t, off + file_size)])
}
