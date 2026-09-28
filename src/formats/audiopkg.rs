//! .AUDIOPKG - from Inevitable / Midway Austin games [Area 51 (PS2/Xbox/GC/PC), The Hobbit
//! (PS2/Xbox/GC/PC)] (vgmstream meta/audiopkg.c).
//!
//! A package of cues ("descriptors", named by "identifiers") pointing to sample indices,
//! which point to one (mono) or two (stereo) sample headers. Each index is a track, named
//! after the identifiers that play it.

use std::io;

use super::{Ctx, Found, Parser, be16, be32, le16, le32};
use crate::codecs::{Codec, ima, pcm, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "AUDIOPKG",
    magics: &[b"v1.5", b"v1.6", b"v1.7", b"v1.8"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Platform {
    Ps2,
    Xbox,
    Gc,
    Pc,
}

/// Reads at file offsets relative to the package, in its byte order.
struct R<'a, 'b> {
    ctx: &'a mut Ctx<'b>,
    base: u64,
    big: bool,
}

impl R<'_, '_> {
    fn u32(&mut self, at: u64) -> io::Result<u32> {
        let b = self.ctx.bytes(self.base + at, 4)?;
        Ok(if self.big { be32(&b, 0) } else { le32(&b, 0) })
    }
    fn u16(&mut self, at: u64) -> io::Result<u16> {
        let b = self.ctx.bytes(self.base + at, 2)?;
        Ok(if self.big { be16(&b, 0) } else { le16(&b, 0) })
    }
}

struct Pkg {
    strings: u64,
    identifiers: u64,
    identifiers_index: u64,
    descriptors_index: u64,
    descriptors: u64,
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let id = ctx.bytes(off, 0x14)?;
    if &id[0..3] != b"v1." {
        return Ok(vec![]);
    }
    let mut version = id[3].wrapping_sub(b'0') as u32;
    if !(5..=8).contains(&version) {
        return Ok(vec![]);
    }
    let platform = match &id[0x10..0x14] {
        b"Wind" => Platform::Pc,
        b"Xbox" => Platform::Xbox,
        b"Play" => Platform::Ps2,
        b"Game" => Platform::Gc,
        _ => return Ok(vec![]),
    };
    if platform == Platform::Pc && version == 6 {
        version = 5;
    }
    let big = platform == Platform::Gc;
    let size = ctx.size() - off;
    let mut r = R { ctx, base: off, big };

    // Package header.
    let mut o = 0x40 + match version {
        5 => 0x60,
        6 => 0x70,
        _ => 0x80,
    };
    let descriptors = r.u32(o)? as i32;
    let identifiers = r.u32(o + 0x04)? as i32;
    let descriptors_size = r.u32(o + 0x08)? as u64;
    let strings_size = r.u32(o + 0x0c)? as u64;
    let lipsyncs_size = r.u32(o + 0x10)? as u64;
    let musicdata_size = r.u32(o + 0x14)? as u64;
    let breakpoints_size = r.u32(o + 0x18)? as u64;
    let mut headers = [0i32; 3];
    let mut indices = [0i32; 3];
    let mut sizes = [0i32; 3];
    for i in 0..3 {
        headers[i] = r.u32(o + 0x1c + 4 * i as u64)? as i32;
        indices[i] = r.u32(o + 0x28 + 4 * i as u64)? as i32;
        sizes[i] = r.u32(o + 0x40 + 4 * i as u64)? as i32;
    }
    o += 0x4c;
    if version >= 6 {
        o += 4;
    }
    if indices[1] != 0 || headers[1] != 0 {
        return Ok(vec![]); // 'warm' samples: not seen
    }
    if lipsyncs_size != 0 && (musicdata_size != 0 || breakpoints_size != 0) {
        return Ok(vec![]);
    }
    let bad = |v: i32| !(0..=0x10000).contains(&v);
    if bad(descriptors) || bad(identifiers) || headers.iter().chain(&indices).any(|&v| bad(v)) || sizes.iter().any(|&v| !(0..=0x100).contains(&v)) {
        return Ok(vec![]);
    }
    let strings = o;
    o += strings_size + lipsyncs_size + breakpoints_size + musicdata_size;
    let identifiers_index = o;
    o += identifiers as u64 * 8;
    let descriptors_index = o;
    o += descriptors as u64 * 4;
    let descriptors_off = o;
    o += descriptors_size;
    let indices_count: i32 = indices.iter().sum();
    let extras = indices.iter().filter(|&&n| n != 0).count() as u64;
    let indices_off = o;
    o += (indices_count as u64 + extras) * 2;
    let headers_count: i32 = headers.iter().sum();
    let headers_off = o;
    o += (0..3).map(|i| headers[i] as u64 * sizes[i] as u64).sum::<u64>();
    if o > size || indices_count > headers_count || indices_count <= 0 {
        return Ok(vec![]);
    }
    let pkg = Pkg { strings, identifiers: identifiers as u64, identifiers_index, descriptors_index, descriptors: descriptors_off };

    let mut found = Vec::new();
    let mut tracks_end = off + o;
    for sub in 0..indices_count {
        // Subsong to temperature (0 = hot, 2 = cold) + index.
        let mut left = sub;
        let mut target = indices_off;
        let mut temp = 0usize;
        for (i, &n) in indices.iter().enumerate() {
            if n == 0 {
                continue;
            }
            if left >= n {
                target += (n as u64 + 1) * 2;
                left -= n;
                continue;
            }
            target += left as u64 * 2;
            temp = i;
            break;
        }
        let i0 = r.u16(target)? as i32;
        let i1 = r.u16(target + 2)? as i32;
        let channels = i1 - i0;
        if !(1..=2).contains(&channels) {
            continue;
        }
        let head_size = sizes[temp] as u64;
        let mut head = headers_off;
        for i in 0..3 {
            if headers[i] == 0 {
                continue;
            }
            if i < temp {
                head += headers[i] as u64 * sizes[i] as u64;
                continue;
            }
            head += i0 as u64 * sizes[i] as u64;
            break;
        }
        if head + head_size * channels as u64 > size || head_size < 0x28 {
            continue;
        }
        let stream_offset = r.u32(head + 0x04)? as u64;
        let stream_size = r.u32(head + 0x08)? as u64;
        let kind = r.u32(head + 0x14)? as i32;
        let samples = r.u32(head + 0x18)? as i32;
        let rate = r.u32(head + 0x1c)? as i32;
        let loop_start = r.u32(head + 0x20)? as i32;
        let loop_end = r.u32(head + 0x24)? as i32;
        let (stream_offset2, interleaved) = if channels == 2 {
            let o2 = r.u32(head + head_size + 0x04)? as u64;
            (o2, o2 == stream_offset)
        } else {
            (0, false)
        };
        let Some(name) = names(&mut r, &pkg, temp, left as u32)? else { return Ok(vec![]) };
        if samples <= 0 || !(1..=96000).contains(&rate) || stream_offset + stream_size > size || stream_offset2 > size {
            continue;
        }
        let ch = channels as u64;
        // Codec, interleave (for interleaved stereo), frame bytes, samples per frame.
        let (codec, il, note): (fn(u64) -> Codec, u64, Option<&str>) = match (kind, platform) {
            (0, Platform::Ps2) => (|il| Codec::Psx(psx::Params::interleaved(il)), 0x8000, None),
            (0, Platform::Xbox) if temp == 2 => (|_| Codec::None, 0x8000, Some("Streamed Xbox audio isn't supported")),
            (0, Platform::Xbox) => (|il| Codec::Ima(ima::Params::new(ima::Kind::XboxMono, il)), 0x8000, None),
            (0, Platform::Pc) => (|il| Codec::Ima(ima::Params::new(ima::Kind::XboxMono, il)), 0x9000, None),
            (0, Platform::Gc) => (|_| Codec::None, 0x8000, Some("GameCube DSP ADPCM audio isn't supported")),
            (1, _) if big => continue,
            (1, _) => (|il| Codec::Pcm(pcm::Params::le16(il)), 2, None),
            (2, _) => (|_| Codec::None, 0, Some("MP3 audio isn't supported")),
            _ => continue,
        };
        let frames_to = |bytes: u64, f: u64| bytes.div_ceil(f) * f;
        let (data, codec) = if note.is_some() || channels == 1 {
            (Data::at(r.ctx.entry, off + stream_offset, stream_size), codec(0))
        } else if interleaved {
            // Whole rows, as vgmstream reads them.
            let row = il * ch;
            (Data::at(r.ctx.entry, off + stream_offset, frames_to(stream_size, row)), codec(il))
        } else {
            // Two mono streams: taken in turns, 0x8000 at a time (a whole number of frames).
            let step = if matches!(kind, 0) && platform != Platform::Ps2 { 0x24 * 0x380 } else { 0x8000 };
            let mut pieces = Vec::new();
            let mut at = 0;
            while at < stream_size {
                pieces.push((off + stream_offset + at, step));
                pieces.push((off + stream_offset2 + at, step));
                at += step;
            }
            (Data::blocks(r.ctx.entry, pieces), codec(step))
        };
        tracks_end = tracks_end.max(off + stream_offset + stream_size).max(off + stream_offset2 + stream_size);
        let mut t = Track::new(r.ctx.entry, off, "AUDIOPKG", channels as u16, rate as u32, samples as u64, data, codec);
        t.note = note.map(str::to_string);
        if loop_end > 0 && loop_start >= 0 && loop_start < loop_end && loop_end <= samples {
            t = t.looped(loop_start as u64, loop_end as u64);
        }
        let label = if name.is_empty() { None } else { super::label(name.as_bytes()) };
        found.push(Found::new(t, 0).label(label));
    }
    for f in found.iter_mut() {
        f.end = tracks_end.min(r.ctx.size());
    }
    Ok(found)
}

/// Names of the identifiers whose cue plays this sample, joined with "; ". None when the
/// package's tables are broken (vgmstream then fails).
fn names(r: &mut R, pkg: &Pkg, temp: usize, index: u32) -> io::Result<Option<String>> {
    let mut out: Vec<String> = Vec::new();
    for i in 0..pkg.identifiers {
        let at = pkg.identifiers_index + i * 8;
        let string = (r.u16(at)? as u64 + pkg.strings) & 0xffff;
        let descriptor = r.u16(at + 2)? as u64;
        let d = r.u32(pkg.descriptors_index + 4 * descriptor)?;
        if d >> 16 != 0 {
            return Ok(None);
        }
        let d = d as u64 + pkg.descriptors;
        if descriptor_uses(r, pkg, d, 0, temp, index)? {
            let b = r.ctx.bytes(r.base + string, 0x100)?;
            out.push(b.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect());
        }
    }
    Ok(Some(out.join("; ")))
}

fn descriptor_uses(r: &mut R, pkg: &Pkg, mut o: u64, depth: u32, temp: usize, index: u32) -> io::Result<bool> {
    if depth > 1 {
        return Ok(false);
    }
    let header = r.u16(o)?;
    let kind = (header >> 14) & 3;
    let params = (header >> 13) & 1;
    o += 4;
    if params != 0 {
        o += r.u16(o)? as u64;
    }
    let items = match kind {
        0 => 1,
        1 => {
            let n = r.u16(o)?;
            o += 2;
            n
        }
        2 => {
            let n = r.u16(o)?;
            o += 2 + 8;
            n
        }
        _ => {
            let n = r.u16(o)?;
            o += 2 + 2 * n as u64;
            n
        }
    };
    for _ in 0..items {
        if kind == 1 {
            o += 2;
        }
        let ih = r.u16(o)?;
        let itype = (ih >> 14) & 3;
        let iparams = (ih >> 13) & 1;
        let value = (ih & 0x1fff) as u32;
        if itype == 3 {
            let d = r.u32(pkg.descriptors_index + 4 * value as u64)? as u64 + pkg.descriptors;
            return descriptor_uses(r, pkg, d, depth + 1, temp, index);
        }
        if itype as usize == temp && value == index {
            return Ok(true);
        }
        o += 4;
        if iparams != 0 {
            o += r.u16(o)? as u64;
        }
    }
    Ok(false)
}
