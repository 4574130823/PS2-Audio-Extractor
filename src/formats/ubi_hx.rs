//! Ubisoft HXAudio banks (.hxd/.hxc/.hx2/.hxg/.hxx/.hx3; vgmstream meta/ubi_hx.c) [Rayman
//! Arena/M, Rayman 3, XIII, Largo Winch]: an index of resource objects; wave objects hold a
//! pseudo-RIFF (PS-ADPCM on PS2, Ubi ADPCM/PCM on PC) with the data inside the bank or in an
//! external stream file. Known by extension only (the index is at the end, no signature).

use std::io;

use super::ea_schl::Rd;
use super::{Ctx, Found, Parser, sane_rate};
use crate::codecs::{Codec, pcm, psx, ubi};
use crate::disc::Reader;
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser { name: "Ubi HX", magics: &[], magic_at: 0, exts: &["hxd", "hxc", "hx2", "hxg", "hxx", "hx3"], locate: None, parse };

#[derive(Clone, Copy, PartialEq, Debug)]
enum HxCodec {
    Pcm,
    Ubi,
    Psx,
    Dsp,
    Xima,
    Atrac3,
    Xma2,
    Mp3,
}

#[derive(Default)]
struct Hx {
    be: bool,
    codec: Option<HxCodec>,
    cuuid1: u32,
    cuuid2: u32,
    stream_offset: u64,
    stream_size: u64,
    channels: u32,
    sample_rate: u32,
    is_external: bool,
    resource_name: String,
    internal_name: String,
}

const WAVE_CLASSES: [&str; 8] = [
    "CPCWaveFileIdObj",
    "CPS2WaveFileIdObj",
    "CGCWaveFileIdObj",
    "CXBoxWaveFileIdObj",
    "CXBoxStaticHWWaveFileIdObj",
    "CXBoxStreamHWWaveFileIdObj",
    "CPS3StaticAC3WaveFileIdObj",
    "CPS3StreamAC3WaveFileIdObj",
];
const OTHER_CLASSES: [&str; 12] = [
    "CEventResData",
    "CProgramResData",
    "CActorResData",
    "CRandomResData",
    "CTreeBank",
    "CTreeRes",
    "CSwitchResData",
    "CPCWavResData",
    "CPS2WavResData",
    "CGCWavResData",
    "CXBoxWavResData",
    "CPS3WavResData",
];
const WAVRES: [&str; 5] = ["CPCWavResData", "CPS2WavResData", "CGCWavResData", "CXBoxWavResData", "CPS3WavResData"];

struct R<'a, 'b> {
    r: &'a mut Rd<'b>,
    be: bool,
}

impl R<'_, '_> {
    fn u32(&mut self, o: u64) -> io::Result<u32> {
        self.r.u32(o, self.be)
    }
    fn s32(&mut self, o: u64) -> io::Result<i32> {
        Ok(self.r.u32(o, self.be)? as i32)
    }
    fn u16(&mut self, o: u64) -> io::Result<u16> {
        self.r.u16(o, self.be)
    }
    fn u8(&mut self, o: u64) -> io::Result<u8> {
        self.r.u8(o)
    }
    /// read_string_sz: `size` bytes, up to the first NUL.
    fn string(&mut self, o: u64, size: u32) -> io::Result<Option<String>> {
        if size > 255 || o + size as u64 > self.r.size() {
            return Ok(None);
        }
        let b = self.r.b(o, size as usize)?;
        let s: Vec<u8> = b.into_iter().take_while(|&c| c != 0).collect();
        Ok(String::from_utf8(s).ok())
    }
}

/// One index entry: class, header offset, header size, links, and where the next one is.
struct Entry {
    class: String,
    header_offset: u64,
    header_size: u64,
    links: Vec<(u32, u32)>,
    lang_links: Vec<(u32, u32)>,
}

fn read_index(r: &mut R) -> io::Result<Option<(u32, Vec<Entry>)>> {
    let size = r.r.size();
    let index_offset = r.u32(0)? as u64;
    if index_offset + 0x0c > size || r.u32(index_offset)? != 0x58444E49 {
        return Ok(None);
    }
    let index_type = r.u32(index_offset + 4)?;
    if index_type != 1 && index_type != 2 {
        return Ok(None);
    }
    let count = r.s32(index_offset + 8)?;
    if !(0..=100000).contains(&count) {
        return Ok(None);
    }
    let mut off = index_offset + 0x0c;
    let mut list = Vec::new();
    for _ in 0..count {
        if off + 4 > size {
            return Ok(None);
        }
        let class_size = r.u32(off)?;
        let Some(class) = r.string(off + 4, class_size)? else { return Ok(None) };
        off += 4 + class_size as u64;
        let header_offset = r.u32(off + 8)? as u64;
        let header_size = r.u32(off + 0x0c)? as u64;
        off += 0x10;
        if r.s32(off)? != 0 {
            return Ok(None); // unknown_count
        }
        off += 4;
        let (mut links, mut lang_links) = (Vec::new(), Vec::new());
        if index_type == 2 {
            let n = r.s32(off)?;
            if !(0..=10000).contains(&n) {
                return Ok(None);
            }
            off += 4;
            for _ in 0..n {
                links.push((r.u32(off)?, r.u32(off + 4)?));
                off += 8;
            }
            let n = r.s32(off)?;
            if !(0..=1000).contains(&n) {
                return Ok(None);
            }
            off += 4;
            for _ in 0..n {
                if r.u32(off + 4)? != 1 {
                    return Ok(None);
                }
                lang_links.push((r.u32(off + 8)?, r.u32(off + 0x0c)?));
                off += 0x10;
            }
        }
        if off > size {
            return Ok(None);
        }
        list.push(Entry { class, header_offset, header_size, links, lang_links });
    }
    Ok(Some((index_type, list)))
}

/// find_chunk_riff_ve: (chunk data offset, chunk size).
fn find_chunk(r: &mut R, id: u32, start: u64, max: u64) -> io::Result<Option<(u64, u64)>> {
    let end = (start + max).min(r.r.size());
    let mut o = start;
    while o < end {
        let t = r.u32(o)?;
        let s = r.u32(o + 4)?;
        if t == 0xFFFFFFFF || s == 0xFFFFFFFF {
            return Ok(None);
        }
        if t == id {
            return Ok(Some((o + 8, s as u64)));
        }
        o += 8 + s as u64;
    }
    Ok(None)
}

fn parse_header(r: &mut R, hx: &mut Hx, e: &Entry) -> io::Result<bool> {
    let mut off = e.header_offset;
    let class_size = r.u32(off)?;
    let Some(class) = r.string(off + 4, class_size)? else { return Ok(false) };
    off += 4 + class_size as u64;
    hx.cuuid1 = r.u32(off)?;
    hx.cuuid2 = r.u32(off + 4)?;
    off += 8;
    let mut stream_adjust = 0u64;
    if ["CPCWaveFileIdObj", "CPS2WaveFileIdObj", "CGCWaveFileIdObj", "CXBoxWaveFileIdObj"].contains(&class.as_str()) {
        let flag_type = r.u32(off)?;
        let stream_mode;
        if flag_type == 1 || flag_type == 2 {
            let unk = r.u32(off + 4)?;
            if unk != 0 && unk != 0xbe570a3d && unk != 0xbf8e147b {
                return Ok(false);
            }
            stream_mode = r.u32(off + 8)?;
            off += 0x10;
        } else if flag_type == 3 {
            off += 8;
            if class == "CGCWaveFileIdObj" {
                if r.u32(off)? != r.u32(off + 4)? {
                    return Ok(false);
                }
                stream_mode = r.u32(off + 4)?;
                off += 8;
            } else {
                stream_mode = r.u8(off)? as u32;
                off += 1;
            }
        } else {
            return Ok(false);
        }
        if stream_mode == 0x0a {
            stream_adjust = r.u32(off)? as u64;
            off += 4;
        }
        let riff_offset;
        match stream_mode {
            0x00 | 0x02 => riff_offset = off,
            0x01 | 0x03 | 0x07 | 0x0a => {
                let rs = r.u32(off)?;
                let Some(name) = r.string(off + 4, rs.min(0x27))? else { return Ok(false) };
                if rs > 0x100 {
                    return Ok(false);
                }
                hx.resource_name = name;
                riff_offset = off + 4 + rs as u64;
                hx.is_external = true;
            }
            _ => return Ok(false),
        }
        let riff_size = r.u32(riff_offset + 4)? as u64 + 8;
        if r.u32(riff_offset)? != 0x46464952 {
            return Ok(false);
        }
        hx.codec = Some(match r.u16(riff_offset + 0x14)? {
            1 => HxCodec::Pcm,
            2 => HxCodec::Ubi,
            3 => HxCodec::Psx,
            4 => HxCodec::Dsp,
            5 => HxCodec::Xima,
            0x55 => HxCodec::Mp3,
            _ => return Ok(false),
        });
        hx.channels = r.u16(riff_offset + 0x16)? as u32;
        hx.sample_rate = r.u32(riff_offset + 0x18)?;
        let max = riff_size.wrapping_sub(0x0c);
        if hx.is_external {
            if let Some((c, _)) = find_chunk(r, 0x78746164, riff_offset + 0x0c, max)? {
                hx.stream_size = r.u32(c)? as u64;
                hx.stream_offset = r.u32(c + 4)? as u64 + stream_adjust;
            } else if let (true, Some((c, s))) = (flag_type == 1 || flag_type == 2, find_chunk(r, 0x61746164, riff_offset + 0x0c, max)?) {
                hx.stream_size = s;
                hx.stream_offset = r.u32(c)? as u64 + stream_adjust;
            } else {
                return Ok(false);
            }
        } else {
            let Some((c, mut s)) = find_chunk(r, 0x61746164, riff_offset + 0x0c, max)? else { return Ok(false) };
            hx.stream_offset = c;
            if s > riff_size.wrapping_sub(c - riff_offset) || s == 0 {
                s = riff_size.wrapping_sub(c - riff_offset);
            }
            hx.stream_size = s;
        }
        Ok(true)
    } else if WAVE_CLASSES.contains(&class.as_str()) {
        // Xbox hardware / PS3 AC3 waves: not decodable here; still parsed for the listing
        hx.stream_offset = r.u32(off)? as u64;
        hx.stream_size = r.u32(off + 4)? as u64;
        off += 8;
        if r.u32(off)? != 1 {
            return Ok(false);
        }
        off += 8;
        let stream_mode = r.u8(off)?;
        off += 1;
        let cue_flag;
        if class.starts_with("CXBox") && !hx.be {
            let flags = r.u8(off + 1)?;
            let (ch, codec) = match flags {
                0x05 => (1, HxCodec::Pcm),
                0x09 => (2, HxCodec::Pcm),
                0x48 => (1, HxCodec::Xima),
                0x90 => (2, HxCodec::Xima),
                _ => return Ok(false),
            };
            hx.channels = ch;
            hx.codec = Some(codec);
            hx.sample_rate = ((r.u16(off + 2)? & 0x7fff) as u32) << 1;
            cue_flag = (r.u8(off + 3)? & 0x80) as u32;
            off += 4;
        } else if class.starts_with("CXBox") {
            hx.codec = Some(HxCodec::Xma2);
            hx.channels = r.u16(off + 2)? as u32;
            hx.sample_rate = r.u32(off + 4)?;
            cue_flag = r.u32(off + 0x34)?;
            off += 0x38;
            if hx.channels == 0 {
                return Ok(false);
            }
        } else {
            hx.codec = Some(HxCodec::Atrac3);
            hx.channels = r.u32(off + 8)?;
            hx.sample_rate = r.u32(off + 0x10)?;
            cue_flag = r.u32(off + 0x40)?;
            off += 0x44;
        }
        if cue_flag != 0 {
            let n = r.s32(off)?;
            if !(0..=10000).contains(&n) {
                return Ok(false);
            }
            off += 4;
            for _ in 0..n {
                off += 8 + r.u32(off + 4)? as u64;
            }
        }
        match stream_mode {
            0x00 | 0x02 => hx.stream_offset += off,
            0x01 | 0x03 | 0x07 => {
                let rs = r.u32(off)?;
                if rs > 0x100 {
                    return Ok(false);
                }
                let Some(name) = r.string(off + 4, rs.min(0x27))? else { return Ok(false) };
                hx.resource_name = name;
                hx.is_external = true;
            }
            _ => return Ok(false),
        }
        Ok(true)
    } else {
        Ok(false)
    }
}

/// parse_name: the WavRes linking to this wave (its name, when stored); fails for external
/// waves nothing links to, like vgmstream.
fn parse_name(r: &mut R, hx: &mut Hx, index_type: u32, list: &[Entry]) -> io::Result<bool> {
    if index_type == 1 {
        return Ok(true);
    }
    for e in list {
        let found = e.links.iter().chain(e.lang_links.iter()).any(|&l| l == (hx.cuuid1, hx.cuuid2));
        if found && WAVRES.contains(&e.class.as_str()) {
            let mut w = e.header_offset;
            let rs = r.u32(w)?;
            w += 4 + rs as u64 + 8 + 4;
            let internal = r.u32(w)?;
            if e.class == "CXBoxWavResData" && internal > 0x100 {
                return Ok(true);
            }
            if internal != 0 {
                if let Some(n) = r.string(w + 4, internal.min(255))? {
                    hx.internal_name = n;
                }
            }
            return Ok(true);
        }
    }
    Ok(!hx.is_external)
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 || ctx.size() < 0x20 {
        return Ok(vec![]);
    }
    let size = ctx.size();
    let entry = ctx.entry;
    let (be, index_type, list) = {
        let mut rd = Rd(&mut ctx.r);
        let name_size = rd.u32(4, true)?;
        if name_size == 0 || name_size & 0x00FFFF00 != 0 {
            return Ok(vec![]);
        }
        let be = rd.guess_be(0)?;
        let mut r = R { r: &mut rd, be };
        let Some((t, list)) = read_index(&mut r)? else { return Ok(vec![]) };
        (be, t, list)
    };
    // every class must be known (vgmstream fails on unknown ones)
    for e in &list {
        let c = e.class.as_str();
        if WAVE_CLASSES.contains(&c) {
            if !e.links.is_empty() {
                return Ok(vec![]);
            }
        } else if !OTHER_CLASSES.contains(&c) {
            return Ok(vec![]);
        }
    }
    let mut found = Vec::new();
    for e in list.iter() {
        if !WAVE_CLASSES.contains(&e.class.as_str()) {
            continue;
        }
        let mut hx = Hx { be, ..Default::default() };
        {
            let mut rd = Rd(&mut ctx.r);
            let mut r = R { r: &mut rd, be };
            if e.header_offset + e.header_size.max(8) > size || !parse_header(&mut r, &mut hx, e)? || !parse_name(&mut r, &mut hx, index_type, &list)? {
                continue;
            }
        }
        let (data_entry, mut reader): (usize, Option<Reader>) = if hx.is_external {
            let name = hx.resource_name.rsplit(['\\', '/']).next().unwrap_or("").to_string();
            match ctx.sibling_named(&name) {
                Some((i2, r2)) => (i2, Some(r2)),
                None => continue,
            }
        } else {
            (entry, None)
        };
        let label = if hx.internal_name.is_empty() {
            format!("{:08x}-{:08x}", hx.cuuid1, hx.cuuid2)
        } else {
            hx.internal_name.clone()
        };
        let ch = hx.channels;
        if !(1..=8).contains(&ch) || !sane_rate(hx.sample_rate) {
            continue;
        }
        let rd_size = reader.as_ref().map(|r| r.size).unwrap_or(size);
        if hx.stream_offset >= rd_size {
            continue;
        }
        let len = hx.stream_size.min(rd_size - hx.stream_offset);
        let ch16 = ch as u16;
        let data = Data::at(data_entry, hx.stream_offset, len);
        let codec = hx.codec.unwrap();
        let mut t = match codec {
            HxCodec::Psx => {
                let probe = match reader.as_mut() {
                    Some(r) => r.bytes(hx.stream_offset, 0x100.min(len as usize))?,
                    None => ctx.bytes(hx.stream_offset, 0x100.min(len as usize))?,
                };
                if !psx::plausible(&probe) {
                    continue;
                }
                Track::new(entry, 0, "Ubi HX", ch16, hx.sample_rate, psx::bytes_to_samples(hx.stream_size, ch16), data, Codec::Psx(psx::Params::interleaved(0x10)))
            }
            HxCodec::Pcm => {
                let p = if be { pcm::Params::be16(0) } else { pcm::Params::le16(0) };
                Track::new(entry, 0, "Ubi HX", ch16, hx.sample_rate, pcm::bytes_to_samples(hx.stream_size, ch16, 16), data, Codec::Pcm(p))
            }
            HxCodec::Ubi => {
                let hb = match reader.as_mut() {
                    Some(r) => r.bytes(hx.stream_offset, 0x30)?,
                    None => ctx.bytes(hx.stream_offset, 0x30)?,
                };
                if u32::from_le_bytes(hb[0..4].try_into().unwrap()) == 2 {
                    continue; // empty data vgmstream plays as silence [Rayman 3 demo (PC)]
                }
                let Some(h) = ubi::header(&hb, hx.stream_size) else { continue };
                if h.channels != ch {
                    continue;
                }
                Track::new(entry, 0, "Ubi HX", ch16, hx.sample_rate, h.samples(), data, Codec::Ubi(ubi::Params {}))
            }
            other => {
                let mut t = Track::new(entry, 0, "Ubi HX", ch16, hx.sample_rate, psx::bytes_to_samples(hx.stream_size, ch16), data, Codec::None);
                t.note = Some(format!("{other:?} audio isn't supported"));
                t
            }
        };
        t.offset = e.header_offset;
        found.push(Found::new(t, size).label(Some(label)));
    }
    Ok(found)
}
