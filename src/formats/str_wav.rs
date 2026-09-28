//! STR+WAV - Blitz Games streams + header [Zapper, The Fairly OddParents, Bad Boys II,
//! SpongeBob, Pac-Man World 3, Taz: Wanted...] (vgmstream meta/str_wav.c).
//!
//! The .str holds only audio; the header is a separate file: "file.wav" for "file.wav.str",
//! else "file.wav" or "file.sth" next to "file.str". Nearly every game has its own header
//! layout, recognized the way vgmstream does.

use std::io;

use super::{Ctx, Found, Parser, split_ext};
use crate::codecs::{Codec, ima, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "STR+WAV",
    magics: &[],
    magic_at: 0,
    exts: &["str", "data"],
    locate: None,
    parse,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Psx,
    PsxChunked,
    Dsp,
    Xbox,
    Wma,
    Ima,
    Xma2,
    Mpeg,
}

#[derive(Default)]
struct Header {
    tracks: i32,
    channels: i32,
    rate: i32,
    samples: i32,
    loop_start: i32,
    loop_end: i32,
    interleave: u64,
    flags: u32,
    kind: Option<Kind>,
}

/// Header reads like vgmstream's: past the end they give 0xFFFFFFFF.
struct H<'a>(&'a [u8]);

impl H<'_> {
    fn get(&self, at: u64, n: usize) -> Option<&[u8]> {
        self.0.get(at as usize..at as usize + n)
    }
    fn l(&self, at: u64) -> u32 {
        self.get(at, 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).unwrap_or(u32::MAX)
    }
    fn b(&self, at: u64) -> u32 {
        self.get(at, 4).map(|b| u32::from_be_bytes(b.try_into().unwrap())).unwrap_or(u32::MAX)
    }
    fn sl(&self, at: u64) -> i32 {
        self.l(at) as i32
    }
    fn sb(&self, at: u64) -> i32 {
        self.b(at) as i32
    }
    fn l16(&self, at: u64) -> u32 {
        self.get(at, 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as u32).unwrap_or(0xffff)
    }
    fn u8(&self, at: u64) -> u32 {
        self.get(at, 1).map(|b| b[0] as u32).unwrap_or(0xff)
    }
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 || ctx.size() == 0 {
        return Ok(vec![]);
    }
    // The header file.
    let (stem, _) = split_ext(ctx.path());
    let base = stem.rsplit('/').next().unwrap_or(stem).to_string();
    let header = match ctx.sibling_named(&base) {
        Some(h) => {
            let (_, ext) = split_ext(&base);
            if !matches!(ext.to_ascii_lowercase().as_str(), "wav" | "wma" | "") {
                return Ok(vec![]);
            }
            Some(h)
        }
        None => ctx.sibling("wav").or_else(|| ctx.sibling("sth")),
    };
    let Some((_, mut hr)) = header else { return Ok(vec![]) };
    if hr.size == 0 || hr.size > 0x10000 {
        return Ok(vec![]);
    }
    let hb = hr.bytes(0, hr.size as usize)?;
    let body_start = ctx.bytes(0, 0x100.min(ctx.size()) as usize)?;
    let Some(mut s) = parse_header(&H(&hb), hb.len() as u64, &body_start) else { return Ok(vec![]) };
    let Some(kind) = s.kind else { return Ok(vec![]) };

    if s.flags == 0 || s.flags & 0xFFFF_FDF8 != 0 {
        return Ok(vec![]);
    }
    let loop_flag = s.flags & 1 != 0;
    if s.channels == 0 {
        s.channels = s.tracks * if s.flags & 0x02 != 0 { 2 } else { 1 };
    }
    if !(1..=8).contains(&s.channels) || !(1..=96000).contains(&s.rate) || s.samples <= 0 && !matches!(kind, Kind::Wma) {
        return Ok(vec![]);
    }
    let channels = s.channels as u16;
    let rate = s.rate as u32;
    let samples = s.samples.max(0) as u64;
    let size = ctx.size();
    // Whole interleave rows, as vgmstream reads them (no shorter last block).
    let row = s.interleave * channels as u64;
    let data = Data::at(ctx.entry, 0, if channels > 1 && row > 0 { size.div_ceil(row) * row } else { size });
    let (data, codec, note) = match kind {
        Kind::Psx => (data, Codec::Psx(psx::Params::interleaved(s.interleave)), None),
        Kind::PsxChunked => {
            // Stereo tracks in 0x20000 chunks taking turns, each two rows of 0x8000 per
            // channel: laid out as one stream of all channels, 0x8000 each.
            if s.flags & 0x02 == 0 || s.tracks < 1 {
                return Ok(vec![]);
            }
            let t = s.tracks as u64;
            let mut pieces = Vec::new();
            let mut g = 0u64;
            'groups: loop {
                for r in 0..2u64 {
                    for i in 0..t {
                        let at = (g * t + i) * 0x20000 + r * 0x10000;
                        if at >= size {
                            break 'groups;
                        }
                        pieces.push((at, 0x10000));
                    }
                }
                g += 1;
            }
            (Data::blocks(ctx.entry, pieces), Codec::Psx(psx::Params::interleaved(0x8000)), None)
        }
        Kind::Xbox => {
            if channels > 2 && !channels.is_multiple_of(2) {
                return Ok(vec![]);
            }
            (data, Codec::Ima(ima::Params::new(ima::Kind::Xbox, s.interleave)), None)
        }
        Kind::Ima => (data, Codec::Ima(ima::Params::new(ima::Kind::Blitz, s.interleave)), None),
        Kind::Dsp => (data, Codec::None, Some("GameCube/Wii DSP ADPCM audio isn't supported")),
        Kind::Wma => (data, Codec::None, Some("WMA audio isn't supported")),
        Kind::Xma2 => (data, Codec::None, Some("XMA audio isn't supported")),
        Kind::Mpeg => (data, Codec::None, Some("MP3 audio isn't supported")),
    };
    if kind == Kind::Psx && !psx::plausible(&body_start) {
        return Ok(vec![]);
    }
    let mut t = Track::new(ctx.entry, 0, "STR+WAV", channels, rate, samples, data, codec);
    t.note = note.map(str::to_string);
    if loop_flag {
        t = vgm_loop(t, s.loop_start as i64, s.loop_end as i64);
    }
    Ok(vec![Found::new(t, size)])
}

/// vgmstream's `parse_header`: the first layout that fits.
fn parse_header(h: &H, header_size: u64, body: &[u8]) -> Option<Header> {
    let mut s = Header::default();
    if h.b(0x00) != 0 {
        return None;
    }
    let hs = header_size as u32;
    let v = h.b(0x04);

    // Fuzion Frenzy (Xbox): WMA
    if v == 0x900 && h.l(0x0c) != hs && h.l(0x24) != 0 && h.l(0x24) == h.l(0x80) && header_size == 0x110 {
        s.samples = h.sl(0x20);
        s.rate = h.sl(0x24);
        s.flags = h.l(0x2c);
        s.tracks = h.sl(0x60);
        s.kind = Some(Kind::Wma);
        return Some(s);
    }
    // Taz: Wanted (GC), Cubix Robots for Everyone: Showdown (GC)
    if v == 0x900 && h.b(0x0c) != hs && h.b(0x24) != 0 && h.b(0x24) == h.b(0x90) && h.b(0xa0) == hs {
        s.samples = h.sb(0x20);
        s.rate = h.sb(0x24);
        s.flags = h.b(0x2c);
        s.tracks = h.sb(0x50);
        s.loop_start = h.sb(0xb8);
        s.loop_end = h.sb(0xbc);
        s.kind = Some(Kind::Dsp);
        s.interleave = if s.tracks > 1 { 0x8000 } else { 0x10000 };
        return Some(s);
    }
    // Taz Wanted demo (PC)
    if v == 0x900 && h.l(0x24) == h.l(0xfc) && h.l(0x10c) == hs {
        s.samples = h.sl(0x20);
        s.rate = h.sl(0x24);
        s.flags = h.l(0x2c);
        s.loop_end = h.sl(0x30);
        s.loop_start = h.sl(0x38);
        s.tracks = h.sl(0xd8);
        s.kind = Some(Kind::Ima);
        s.interleave = 0x10000;
        return Some(s);
    }
    // The Fairly OddParents - Breakin' da Rules (Xbox)
    if v == 0x900 && h.l(0x24) == h.l(0xb0) && h.l(0xc0).wrapping_mul(4).wrapping_add(h.l(0xc4)) == hs {
        s.samples = h.sl(0x20);
        s.rate = h.sl(0x24);
        s.flags = h.l(0x2c);
        s.loop_start = h.sl(0x38);
        s.tracks = h.sl(0x70);
        s.loop_end = s.samples;
        s.kind = Some(Kind::Xbox);
        s.interleave = if s.tracks > 1 { 0xD800 / 2 } else { 0xD800 };
        return Some(s);
    }
    // Pac-Man World 3 (Xbox)
    if (v == 0x800 || v == 0x0100_0800) && h.l(0x24) == h.l(0xb0) && h.l(0x28) == 0x10 && h.l(0xe0).wrapping_add(h.l(0xe4).wrapping_mul(0x40)) == hs {
        s.samples = h.sl(0x20);
        s.rate = h.sl(0x24);
        s.flags = h.l(0x2c);
        s.loop_start = h.sl(0x38);
        s.tracks = h.sl(0x70);
        s.loop_end = s.samples;
        s.kind = Some(Kind::Xbox);
        s.interleave = if s.tracks > 1 { 0xD800 / 2 } else { 0xD800 };
        return Some(s);
    }
    // The Fairly OddParents! - Shadow Showdown (GC), Bad Boys II (GC)
    if v == 0x800 && h.b(0x24) == h.b(0xb0) && h.b(0x24) == h.b(h.b(0xe0) as u64 + 0x08) && h.b(0xc0).wrapping_mul(4).wrapping_add(h.b(0xc4)) == hs {
        s.samples = h.sb(0x20);
        s.rate = h.sb(0x24);
        s.flags = h.b(0x2c);
        s.tracks = h.sb(0x70);
        s.loop_start = h.sb(0xd8);
        s.loop_end = h.sb(0xdc);
        s.kind = Some(Kind::Dsp);
        s.interleave = if s.tracks > 1 { 0x8000 } else { 0x10000 };
        return Some(s);
    }
    // Zapper: One Wicked Cricket! (Beta) (GC)
    if v == 0x900 && h.b(0x24) == h.b(0xb0) && h.b(0x88) != 0 && h.l(0xc0) == hs {
        s.samples = h.sb(0x20);
        s.rate = h.sb(0x24);
        s.flags = h.b(0x2c);
        s.tracks = h.sb(0x70);
        s.loop_start = h.sb(0xd8);
        s.loop_end = h.sb(0xdc);
        s.kind = Some(Kind::Dsp);
        s.interleave = if s.tracks > 1 { 0x8000 } else { 0x10000 };
        return Some(s);
    }
    // The Mummy Returns (PS2)
    if v == 0x900 && h.l(0x00) == 0 && h.l(0x0c) != 0 && h.l(0x2c) == 44100 && header_size == 0x50 {
        s.rate = h.sl(0x2c);
        s.flags = h.l(0x34);
        s.samples = h.sl(0x44);
        s.tracks = h.sl(0x48);
        s.kind = Some(Kind::Psx);
        s.interleave = 0x8000;
        return Some(s);
    }
    // Zapper: One Wicked Cricket! Beta (PS2)
    if v == 0x900 && h.l(0x2c) == 44100 && h.l(0x70) == 0 && header_size == 0x78 {
        s.rate = h.sl(0x2c);
        s.flags = h.l(0x34);
        s.samples = h.sl(0x5c);
        s.tracks = h.sl(0x60);
        s.kind = Some(if s.tracks > 1 { Kind::PsxChunked } else { Kind::Psx });
        s.interleave = 0x8000;
        return Some(s);
    }
    // Zapper (PS2), The Fairly OddParents (PS2), Bad Boys II (PS2); Pac-Man World 3 (PS2)
    if (v == 0x800 || v == 0x900 || v == 0x0100_0800) && h.l(0x24) == h.l(0x70) && h.l(0x78).wrapping_mul(4).wrapping_add(h.l(0x7c)) == hs {
        s.samples = h.sl(0x20);
        s.rate = h.sl(0x24);
        s.flags = h.l(0x2c);
        s.loop_start = h.sl(0x38);
        s.tracks = h.sl(0x40);
        s.loop_end = h.sl(0x54);
        s.kind = Some(Kind::Psx);
        s.interleave = if s.tracks > 1 { 0x4000 } else { 0x8000 };
        return Some(s);
    }
    // Taz Wanted (beta) (PC)
    if v == 0x900 && h.l(0x0c) != hs && h.l(0x24) != 0 && h.l(0xd4) != 0 && h.l(0xdc) == hs {
        s.samples = h.sl(0x20);
        s.rate = h.sl(0x24);
        s.flags = h.l(0x2c);
        s.loop_start = h.sl(0x38);
        s.tracks = h.sl(0xd4);
        s.loop_end = s.samples;
        s.kind = Some(Kind::Ima);
        s.interleave = if s.tracks > 1 { 0x8000 } else { 0x10000 };
        return Some(s);
    }
    // Taz Wanted (PC), Zapper: One Wicked Cricket! Beta (Xbox)
    if v == 0x900 && h.l(0x0c) != hs && h.l(0x24) != 0 && h.l(0x24) == h.l(0x90) && (h.l(0xa0) == hs || h.l(0xa0).wrapping_add(0x50) == hs) {
        s.samples = h.sl(0x20);
        s.rate = h.sl(0x24);
        s.flags = h.l(0x2c);
        s.loop_start = h.sl(0x38);
        s.tracks = h.sl(0x50);
        s.loop_end = s.samples;
        s.kind = Some(Kind::Xbox);
        s.interleave = if s.tracks > 1 { 0xD800 / 2 } else { 0xD800 };
        return Some(s);
    }
    // Zapper: One Wicked Cricket! (Xbox)
    if v == 0x900 && h.l(0x0c) != hs && h.l(0x24) != 0 && h.l(0x24) == h.l(0xb0) && h.l(0xc0) == hs {
        s.samples = h.sl(0x20);
        s.rate = h.sl(0x24);
        s.flags = h.l(0x2c);
        s.loop_start = h.sl(0x38);
        s.tracks = h.sl(0x70);
        s.loop_end = s.samples;
        s.kind = Some(Kind::Xbox);
        s.interleave = if s.tracks > 1 { 0xD800 / 2 } else { 0xD800 };
        return Some(s);
    }
    // Zapper: One Wicked Cricket! (PC)
    if v == 0x900 && h.l(0x24) == h.l(0x114) && h.l(0x12c) == hs {
        s.samples = h.sl(0x20);
        s.rate = h.sl(0x24);
        s.flags = h.l(0x2c);
        s.loop_end = h.sl(0x30);
        s.tracks = h.sl(0xf8);
        s.loop_start = 0;
        s.kind = Some(Kind::Ima);
        s.interleave = if s.tracks > 1 { 0x8000 } else { 0x10000 };
        return Some(s);
    }
    // Bad Boys II (PC), Pac-Man World 3 (PC)
    if (v == 0x800 || v == 0x0100_0800)
        && h.l(0x24) == h.l(0x114)
        && (h.l(0x128).wrapping_mul(4).wrapping_add(h.l(0x12c)) == hs || h.l(0x130).wrapping_add(h.l(0x134).wrapping_mul(0x40)) == hs)
    {
        s.samples = h.sl(0x20);
        s.rate = h.sl(0x24);
        s.flags = h.l(0x2c);
        s.loop_end = h.sl(0x30);
        s.loop_start = h.sl(0x38);
        s.tracks = h.sl(0xf8);
        s.kind = Some(Kind::Ima);
        s.interleave = if s.tracks > 1 { 0x8000 } else { 0x10000 };
        return Some(s);
    }
    // Pac-Man World 3 (GC), SpongeBob SquarePants: Creature from the Krusty Krab (GC/Wii)
    if v == 0x800
        && h.b(0x24) == h.b(0xb0)
        && h.b(0x24) == h.b(h.b(0xf0) as u64 + 0x08)
        && h.b(0xc0).wrapping_mul(4).wrapping_add(h.b(0xc4)) == h.b(0xe0)
        && (h.b(0xe0).wrapping_add(h.b(0xe4).wrapping_mul(0x40)) == hs || h.b(0xe0).wrapping_add(h.b(0xe4).wrapping_mul(0x08)) == hs)
    {
        s.samples = h.sb(0x20);
        s.rate = h.sb(0x24);
        s.flags = h.b(0x2c);
        s.loop_start = h.sb(0xd8);
        s.loop_end = h.sb(0xdc);
        s.tracks = h.sb(0x70);
        s.kind = Some(Kind::Dsp);
        s.interleave = if s.tracks >= 2 { 0x8000 } else { 0x10000 };
        return Some(s);
    }
    let table_size = |h: &H| {
        let per = if h.l(0x3c) & 0x200 != 0 { 0x08 + 0x38 } else { 0x08 };
        h.l(0x40).wrapping_add(h.l16(0x48) * 4).wrapping_add(h.l16(0x4a) * per)
    };
    // SpongeBob SquarePants: Creature from the Krusty Krab (PS2), Big Bumpin' (Xbox), Sneak King (Xbox)
    if v == 0x800 && h.l(0x08) == 0 && h.l(0x0c) != hs && hs == table_size(h) {
        s.loop_start = h.sl(0x24);
        s.samples = h.sl(0x30);
        s.loop_end = h.sl(0x34);
        s.rate = h.sl(0x38);
        s.flags = h.l(0x3c);
        s.tracks = h.u8(0x4e) as i32;
        let psx_like = body.chunks(16).all(|f| f[0] >> 4 <= 5 && f.get(1).is_some_and(|&b| b <= 7));
        if psx_like {
            s.kind = Some(Kind::Psx);
            s.interleave = if s.tracks > 2 { 0x4000 } else { 0x8000 };
        } else {
            s.kind = Some(Kind::Xbox);
            s.interleave = if s.tracks > 1 { 0x9000 } else { 0xD800 };
        }
        return Some(s);
    }
    // Tak and the Guardians of Gross (PS2), SpongeBob's Atlantis SquarePantis (PS2)
    if v == 0x800 && h.l(0x08) != 0 && h.l(0x0c) == hs && hs == table_size(h) {
        s.loop_start = h.sl(0x24);
        s.samples = h.sl(0x30);
        s.loop_end = h.sl(0x34);
        s.rate = h.sl(0x38);
        s.flags = h.l(0x3c);
        s.channels = h.sl(0x70);
        s.kind = Some(Kind::Psx);
        s.interleave = if s.channels > 4 { 0x4000 } else { 0x8000 };
        return Some(s);
    }
    // Tak (Wii), The House of the Dead: Overkill (Wii), All Star Karate (Wii), Karaoke Revolution (Wii)
    if (v == 0x800 || v == 0x700) && h.b(0x08) != 0 && h.b(0x0c) == hs && h.b(0x7c) != 0 && h.b(0x38) == h.b(h.b(0x7c) as u64 + 0x38) {
        s.samples = h.sb(0x30);
        s.loop_end = h.sb(0x34);
        s.rate = h.sb(0x38);
        s.flags = h.b(0x3c) & !1;
        s.channels = h.sb(0x70);
        s.kind = Some(Kind::Dsp);
        s.interleave = if s.channels > 4 { 0x4000 } else { 0x8000 };
        return Some(s);
    }
    // The House of the Dead: Overkill (PS3), Karaoke Revolution (PS3)
    if (v == 0x800 || v == 0x700) && h.b(0x08) != 0 && h.b(0x0c) == hs && h.b(0x7c) == 0 {
        s.samples = h.sb(0x30);
        s.loop_end = h.sb(0x34);
        s.rate = h.sb(0x38);
        s.flags = h.b(0x3c);
        s.channels = h.sb(0x70);
        if h.sb(0x78) != 0 {
            s.tracks = s.channels / 2;
            s.samples = s.loop_end;
            s.interleave = h.sb(0xa0) as u32 as u64;
            s.kind = Some(Kind::Mpeg);
        } else {
            s.interleave = if s.channels > 4 { 0x4000 } else { 0x8000 };
            s.flags &= !1;
            s.kind = Some(Kind::Psx);
        }
        return Some(s);
    }
    // SpongeBob's Surf & Skate Roadtrip (X360)
    if (v == 0x800 || v == 0x700) && h.b(0x08) != 0 && h.b(0x0c) == 0x124 && h.b(0x8c) == 0x180 {
        s.samples = h.sb(0x30);
        s.loop_end = h.sb(0x34);
        s.rate = h.sb(0x38);
        s.flags = h.b(0x3c);
        s.channels = h.sb(0x70);
        s.kind = Some(Kind::Xma2);
        return Some(s);
    }
    None
}

/// Loop points as vgmstream keeps them: dropped unless 0 <= start < end <= samples.
fn vgm_loop(t: Track, start: i64, end: i64) -> Track {
    if start >= 0 && start < end && end as u64 <= t.samples { t.looped(start as u64, end as u64) } else { t }
}
