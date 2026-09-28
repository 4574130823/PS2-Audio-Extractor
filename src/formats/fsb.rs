//! FSB1 to FSB4: FMOD sample banks (vgmstream meta/fsb.c). PS2 banks hold PS-ADPCM
//! ("VAG" mode) or PCM; the other codecs FSB carries (MPEG, IMA, XMA, DSP, CELT) are listed
//! with a note.

use std::io;

use super::vag::vgm_loop;
use super::{Ctx, Found, Parser, label, le16, le32, sane_rate};
use crate::codecs::{Codec, pcm, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "FSB",
    magics: &[b"FSB1", b"FSB2", b"FSB3", b"FSB4"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

// header flags
const SOURCE_BASICHEADERS: u32 = 0x02;
const SOURCE_BIGENDIANPCM: u32 = 0x08;
const SOURCE_NOTINTERLEAVED: u32 = 0x10;
const SOURCE_MPEG_PADDED4: u32 = 0x40;
// sample mode flags
const LOOP_OFF: u32 = 0x01;
const LOOP_NORMAL: u32 = 0x02;
const BITS8: u32 = 0x08;
const STEREO: u32 = 0x40;
const UNSIGNED: u32 = 0x80;
const MPEG: u32 = 0x200;
const DUPLICATE: u32 = 0x8000;
const IMAADPCM: u32 = 0x40_0000;
const VAG: u32 = 0x80_0000;
const XMA: u32 = 0x100_0000;
const GCADPCM: u32 = 0x200_0000;
const CELT: u32 = 0x800_0000;

const VERSION_3_0: u32 = 0x0003_0000;
const VERSION_3_1: u32 = 0x0003_0001;
const VERSION_4_0: u32 = 0x0004_0000;

struct Sample {
    name: Option<String>,
    num_samples: i32,
    stream_size: u32,
    loop_start: i32,
    loop_end: i32,
    mode: u32,
    rate: i32,
    channels: u16,
    stream_offset: u32,
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x30)?;
    let id = &h[0..4];
    let total = le32(&h, 4) as i32;
    let avail = ctx.size() - off;
    let mut samples = Vec::new();
    let (base, headers_size, data_size, flags);
    if id == b"FSB1" {
        // one sample only
        if !(0..=1).contains(&total) {
            return Ok(vec![]);
        }
        base = 0x10u32;
        headers_size = 0x40u32;
        data_size = le32(&h, 8);
        flags = 0;
        let s = ctx.bytes(off + 0x10, 0x40)?;
        let mode = le32(&s, 0x34);
        let mut num_samples = le32(&s, 0x20) as i32;
        let loop_end = le32(&s, 0x3c) as i32;
        if loop_end > num_samples {
            num_samples = loop_end;
        }
        samples.push(Sample {
            name: label(&s[0..0x20]),
            num_samples,
            stream_size: le32(&s, 0x24),
            loop_start: le32(&s, 0x38) as i32,
            loop_end,
            mode,
            rate: le32(&s, 0x28) as i32,
            channels: if mode & STEREO != 0 { 2 } else { 1 },
            stream_offset: base + headers_size,
        });
    } else {
        let (b, mut min) = match id {
            b"FSB2" => (0x10u32, 0x40u32),
            b"FSB3" => (0x18, 0x40),
            b"FSB4" => (0x30, 0x50),
            _ => return Ok(vec![]),
        };
        base = b;
        headers_size = le32(&h, 8);
        data_size = le32(&h, 0x0c);
        let version = if base > 0x10 { le32(&h, 0x10) } else { 0 };
        flags = if base > 0x10 { le32(&h, 0x14) } else { 0 };
        if version == VERSION_3_1 {
            min = 0x50;
        } else if version != 0 && version != VERSION_3_0 && version != VERSION_4_0 {
            return Ok(vec![]);
        }
        // (FSB2 has no version: check it's where FSB2 would be)
        if (id == b"FSB2") != (version == 0) {
            return Ok(vec![]);
        }
        if headers_size < min || !(1..=10000).contains(&total) || base as u64 + headers_size as u64 > avail {
            return Ok(vec![]);
        }
        let hdrs = ctx.bytes(off + base as u64, headers_size as usize)?;
        let mut hoff = 0usize;
        let mut data_off = base + headers_size;
        let mut prev_data_off = data_off;
        let (mut mode, mut rate, mut channels, mut name) = (0u32, 0i32, 0u16, None);
        for i in 0..total {
            let (header_size, num_samples, stream_size, loop_start, loop_end);
            if flags & SOURCE_BASICHEADERS != 0 && i > 0 {
                // mini header: everything else is the first sample's
                if hoff + 8 > hdrs.len() {
                    return Ok(vec![]);
                }
                let mut size = 8usize;
                num_samples = le32(&hdrs, hoff) as i32;
                stream_size = le32(&hdrs, hoff + 4);
                loop_start = 0;
                loop_end = 0;
                if mode & GCADPCM != 0 {
                    size += 0x2e * channels as usize;
                } else if mode & XMA != 0 {
                    let seek = hdrs.get(hoff + 0x14..hoff + 0x18).map(|b| le32(b, 0)).unwrap_or(0) as usize;
                    size += 0x10 + seek;
                }
                header_size = size;
            } else {
                if hoff + 0x40 > hdrs.len() {
                    return Ok(vec![]);
                }
                let s = &hdrs[hoff..];
                header_size = le16(s, 0) as usize;
                name = label(&s[2..0x20]);
                num_samples = le32(s, 0x20) as i32;
                stream_size = le32(s, 0x24);
                loop_start = le32(s, 0x28) as i32;
                loop_end = le32(s, 0x2c) as i32;
                mode = le32(s, 0x30);
                rate = le32(s, 0x34) as i32;
                channels = le16(s, 0x3e);
                if header_size < 0x40 || header_size > hdrs.len() {
                    return Ok(vec![]);
                }
            }
            let stream_offset = if mode & DUPLICATE != 0 { prev_data_off } else { data_off };
            samples.push(Sample { name: name.clone(), num_samples, stream_size, loop_start, loop_end, mode, rate, channels, stream_offset });
            hoff += header_size;
            if mode & DUPLICATE == 0 {
                prev_data_off = data_off;
                data_off = data_off.wrapping_add(stream_size);
                if flags & SOURCE_MPEG_PADDED4 != 0 && data_off % 0x20 != 0 {
                    data_off += 0x20 - data_off % 0x20;
                }
            }
        }
    }
    let end = (off + base as u64 + headers_size as u64 + data_size as u64).min(ctx.size());

    let mut found = Vec::new();
    let mut psx_checked = false;
    for s in samples {
        if s.channels == 0 || s.channels > 16 || !sane_rate(s.rate as u32) || s.num_samples <= 0 {
            continue; // vgmstream refuses these
        }
        let ch = s.channels as u64;
        let data_at = off + s.stream_offset as u64;
        let mut note = None;
        let codec = if s.mode & (MPEG | IMAADPCM) != 0 {
            None
        } else if s.mode & VAG != 0 {
            let il = if flags & SOURCE_NOTINTERLEAVED != 0 { s.stream_size as u64 / ch } else { 0x10 };
            if !psx_checked {
                // reject false signature hits: PS-ADPCM must look like it
                if !psx::plausible(&ctx.bytes(data_at, 0x100.min(s.stream_size as usize))?) {
                    return Ok(vec![]);
                }
                psx_checked = true;
            }
            Some(Codec::Psx(psx::Params::interleaved(il)))
        } else if s.mode & (XMA | GCADPCM | CELT) != 0 {
            None
        } else if s.mode & BITS8 != 0 {
            Some(Codec::Pcm(if s.mode & UNSIGNED != 0 { pcm::Params::u8(0) } else { pcm::Params::s8(0) }))
        } else if flags & SOURCE_BIGENDIANPCM != 0 {
            Some(Codec::Pcm(pcm::Params::be16(0)))
        } else {
            Some(Codec::Pcm(pcm::Params::le16(0)))
        };
        let codec = codec.unwrap_or_else(|| {
            note = Some(if s.mode & MPEG != 0 {
                "MPEG FSB isn't supported"
            } else if s.mode & IMAADPCM != 0 {
                "IMA ADPCM FSB isn't supported here"
            } else if s.mode & XMA != 0 {
                "XMA FSB isn't supported"
            } else if s.mode & GCADPCM != 0 {
                "GameCube DSP FSB isn't supported"
            } else {
                "CELT FSB isn't supported"
            });
            Codec::None
        });
        let mut t = Track::new(ctx.entry, off, "FSB", s.channels, s.rate as u32, s.num_samples as u64, Data::at(ctx.entry, data_at, s.stream_size as u64), codec);
        t.note = note.map(String::from);
        // loops, as vgmstream's fix_loops
        let loop_end = if s.loop_end != 0 { s.loop_end + 1 } else { 0 };
        let full = s.loop_start == 0 && loop_end == s.num_samples;
        let small = (s.num_samples as i64) < 20 * s.rate as i64;
        let mut looped = s.mode & LOOP_OFF == 0;
        if looped && s.mode & LOOP_NORMAL == 0 && full && small {
            looped = false;
        }
        if looped {
            t = vgm_loop(t, s.loop_start as i64, loop_end as i64);
        }
        found.push(Found::new(t, end).label(s.name));
    }
    Ok(found)
}
