//! Ogg Vorbis (vgmstream meta/ogg_vorbis.c, standard "OggS" streams): sample count from
//! the last page's granule position (vorbisfile's `ov_pcm_total`), loop points from the
//! many comment conventions games use. The encrypted/obfuscated PC variants aren't ported.

use std::io;

use super::vag::vgm_loop;
use super::{Ctx, Found, Parser, sane_rate};
use crate::codecs::{Codec, vorbis};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "OGG",
    magics: &[b"OggS"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let size = ctx.size() - off;
    Ok(parse_at(ctx, off, size)?.into_iter().collect())
}

/// Ogg CRC-32 (polynomial 0x04c11db7, no reflection, zero init).
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &b in data {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04c1_1db7 } else { crc << 1 };
        }
    }
    crc
}

struct Page {
    flags: u8,
    granule: i64,
    serial: u32,
    /// payload offset and segment sizes
    body: u64,
    segments: Vec<u8>,
    size: u64,
}

fn page(ctx: &mut Ctx, at: u64, limit: u64, check_crc: bool) -> io::Result<Option<Page>> {
    if at + 27 > limit {
        return Ok(None);
    }
    let h = ctx.bytes(at, 27)?;
    if &h[0..4] != b"OggS" || h[4] != 0 {
        return Ok(None);
    }
    let nseg = h[26] as usize;
    let segments = ctx.bytes(at + 27, nseg)?;
    let body_len: u64 = segments.iter().map(|&s| s as u64).sum();
    let size = 27 + nseg as u64 + body_len;
    if at + size > limit {
        return Ok(None);
    }
    if check_crc {
        let mut all = ctx.bytes(at, size as usize)?;
        let stored = u32::from_le_bytes(all[22..26].try_into().unwrap());
        all[22..26].fill(0);
        if crc32(&all) != stored {
            return Ok(None);
        }
    }
    Ok(Some(Page {
        flags: h[5],
        granule: i64::from_le_bytes(h[6..14].try_into().unwrap()),
        serial: u32::from_le_bytes(h[14..18].try_into().unwrap()),
        body: at + 27 + nseg as u64,
        segments,
        size,
    }))
}

/// An Ogg Vorbis stream at `off`, within `limit` bytes (a whole file, or an .acx entry).
pub(crate) fn parse_at(ctx: &mut Ctx, off: u64, limit: u64) -> io::Result<Option<Found>> {
    let end_limit = off + limit.min(ctx.size() - off);
    let Some(first) = page(ctx, off, end_limit, true)? else { return Ok(None) };
    if first.flags & 0x02 == 0 {
        return Ok(None); // must begin a stream
    }
    let serial = first.serial;
    // walk the stream's pages: the first 3 packets are the headers
    let mut packets: Vec<Vec<u8>> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut at = off;
    let mut last_granule = -1i64;
    let mut end = off;
    let mut pages = 0u64;
    while let Some(p) = page(ctx, at, end_limit, pages < 3)? {
        pages += 1;
        if p.serial == serial {
            if packets.len() < 3 {
                let body = ctx.bytes(p.body, p.segments.iter().map(|&s| s as usize).sum())?;
                let mut pos = 0usize;
                for &s in &p.segments {
                    cur.extend_from_slice(&body[pos..pos + s as usize]);
                    pos += s as usize;
                    if s < 255 {
                        packets.push(std::mem::take(&mut cur));
                    }
                }
            }
            if p.granule != -1 {
                last_granule = p.granule;
            }
            end = at + p.size;
            if p.flags & 0x04 != 0 {
                break;
            }
        }
        at += p.size;
    }
    if packets.len() < 3 {
        return Ok(None);
    }
    let id = &packets[0];
    if id.len() < 30 || &id[0..7] != b"\x01vorbis" || u32::from_le_bytes(id[7..11].try_into().unwrap()) != 0 {
        return Ok(None);
    }
    let channels = id[11] as u16;
    let rate = u32::from_le_bytes(id[12..16].try_into().unwrap());
    if !(1..=8).contains(&channels) || !sane_rate(rate) || id[29] & 1 == 0 {
        return Ok(None);
    }
    if packets[1].len() < 7 || &packets[1][0..7] != b"\x03vorbis" || packets[2].len() < 7 || &packets[2][0..7] != b"\x05vorbis" {
        return Ok(None);
    }
    let comments = read_comments(&packets[1][7..]);
    let samples = last_granule;
    if samples <= 0 {
        return Ok(None);
    }
    let lp = loops(&comments, rate as i64);
    let mut t = Track::new(
        ctx.entry,
        off,
        "OGG",
        channels,
        rate,
        samples as u64,
        Data::at(ctx.entry, off, end - off),
        Codec::Vorbis(vorbis::Params { disable_reordering: lp.disable_reordering }),
    );
    if lp.flag {
        let mut e = if lp.length_found {
            lp.start as i64 + lp.length as i64
        } else if lp.end_found {
            lp.end as i64
        } else {
            samples
        };
        if e > samples {
            e = samples;
        }
        t = vgm_loop(t, lp.start as i64, e);
    }
    Ok(Some(Found::new(t, end).label(lp.name)))
}

fn read_comments(b: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let rd = |p: usize| b.get(p..p + 4).map(|x| u32::from_le_bytes(x.try_into().unwrap()) as usize);
    let Some(vlen) = rd(0) else { return out };
    let mut p = 4 + vlen;
    let Some(n) = rd(p) else { return out };
    p += 4;
    for _ in 0..n.min(10000) {
        let Some(len) = rd(p) else { break };
        p += 4;
        let Some(s) = b.get(p..p + len) else { break };
        out.push(String::from_utf8_lossy(s).into_owned());
        p += len;
    }
    out
}

#[derive(Default)]
struct Loops {
    flag: bool,
    start: i32,
    end: i32,
    length: i32,
    end_found: bool,
    length_found: bool,
    disable_reordering: bool,
    name: Option<String>,
}

/// C's `atol`: leading spaces, a sign, digits.
fn atol(s: &str) -> i64 {
    let s = s.trim_start();
    let (neg, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut v: i64 = 0;
    for c in s.bytes().take_while(|c| c.is_ascii_digit()) {
        v = v.wrapping_mul(10).wrapping_add((c - b'0') as i64);
    }
    if neg { -v } else { v }
}

/// sscanf-style "%d<sep>%d...": the ints read before the first mismatch.
fn scan_ints(s: &str, seps: &[&str]) -> Vec<i32> {
    let mut out = Vec::new();
    let mut rest = s;
    for i in 0..=seps.len() {
        let t = rest.trim_start();
        let len = t.char_indices().take_while(|&(k, c)| c.is_ascii_digit() || (k == 0 && (c == '-' || c == '+'))).count();
        if len == 0 || (len == 1 && !t.as_bytes()[0].is_ascii_digit()) {
            break;
        }
        out.push(atol(&t[..len]) as i32);
        rest = &t[len..];
        if i < seps.len() {
            match rest.strip_prefix(seps[i]) {
                Some(r) => rest = r,
                None => break,
            }
        }
    }
    out
}

fn after_last<'a>(s: &'a str, c: char) -> &'a str {
    s.rfind(c).map(|p| &s[p + 1..]).unwrap_or("")
}

/// vgmstream's loop comment conventions, in its order.
fn loops(comments: &[String], rate: i64) -> Loops {
    let mut l = Loops::default();
    for c in comments {
        let starts = |p: &str| c.starts_with(p);
        if ["loop_start=", "LOOP_START=", "LOOPPOINT=", "COMMENT=LOOPPOINT=", "LOOPSTART=", "um3.stream.looppoint.start=", "LOOP_BEGIN=", "LoopStart=", "LOOP=", "XIPH_CUE_LOOPSTART=", "LOOPS="]
            .iter()
            .any(|p| starts(p))
        {
            l.start = atol(after_last(c, '=')) as i32;
            l.flag = l.start >= 0;
        } else if starts("LOOPLENGTH=") {
            l.length = atol(after_last(c, '=')) as i32;
            l.length_found = true;
        } else if ["loop_end=", "LOOP_END=", "LoopEnd=", "XIPH_CUE_LOOPEND=", "LOOPE="].iter().any(|p| starts(p)) {
            l.end = atol(after_last(c, '=')) as i32;
            l.end_found = true;
            l.flag = true;
        } else if starts("title=-lps") {
            l.start = atol(&c[10..]) as i32;
            l.flag = l.start >= 0;
        } else if starts("album=-lpe") {
            l.end = atol(&c[10..]) as i32;
            l.end_found = true;
            l.flag = true;
        } else if starts("lp=") || starts("LOOPDEFS=") || starts("COMMENT=loop(") {
            let v = scan_ints(after_last(c, if starts("COMMENT=loop(") { '(' } else { '=' }), &[","]);
            if let Some(&a) = v.first() {
                l.start = a;
            }
            if let Some(&b) = v.get(1) {
                l.end = b;
            }
            l.end_found = true;
            l.flag = true;
        } else if starts("omment=LOOPSTART=") {
            if let Some(p) = c.find("=LOOPSTART=") {
                let v = scan_ints(&c[p + 11..], &[",LOOPEND="]);
                if let Some(&a) = v.first() {
                    l.start = a;
                }
                if let Some(&b) = v.get(1) {
                    l.end = b;
                }
            }
            l.end_found = true;
            l.flag = true;
        } else if starts("MarkerNum=0002") {
            l.flag = true;
        } else if starts("M=7F") {
            let hex = c[4..].trim_start();
            let digits: String = hex.chars().take_while(|ch| ch.is_ascii_hexdigit()).collect();
            let v = i64::from_str_radix(&digits, 16).ok().map(|v| v as i32);
            if l.flag && l.start < 0 && l.end <= 0 {
                if let Some(v) = v {
                    l.start = v;
                }
            } else if l.flag && l.start >= 0 && l.end <= 0 {
                if let Some(v) = v {
                    l.end = v;
                }
                l.end_found = true;
            }
        } else if starts("LOOPMS=") {
            l.start = (atol(after_last(c, '=')) * rate / 1000) as i32;
            l.flag = l.start >= 0;
        } else if starts("COMMENT=- loopTime ") || starts("COMMENT=-loopTime ") {
            let v = c.rfind(' ').map(|p| atol(&c[p..])).unwrap_or(0);
            l.start = ((v as f32 / 1000.0f32) * rate as f32) as i32;
            l.flag = l.start >= 0;
        } else if starts("COMMENT=*loopsample,") {
            let v = scan_ints(&c["COMMENT=*loopsample,".len()..], &[",", ",", ","]);
            if v.len() >= 2 {
                l.start = v[1];
            }
            if v.len() >= 3 {
                l.end = v[2];
            }
            if v.len() == 4 {
                l.flag = true;
                l.end_found = true;
            }
        } else if starts("COMMENT=SetSample ") {
            let v = scan_ints(&c["COMMENT=SetSample ".len()..], &[",", ","]);
            if v.len() >= 2 {
                l.start = v[1];
            }
            if v.len() >= 3 {
                l.end = v[2];
                l.flag = true;
                l.end_found = true;
            }
        } else if starts("L=") {
            l.start = atol(after_last(c, '=')) as i32;
            l.flag = true;
        } else if starts("ENCODER=ogg_vorbis_encode/") {
            l.disable_reordering = true;
        } else if starts("TITLE=") {
            l.name = super::label(c[6..].as_bytes());
        }
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_loops() {
        let c = |v: &[&str]| loops(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>(), 44100);
        let l = c(&["LOOPSTART=1000", "LOOPLENGTH=500"]);
        assert!(l.flag && l.start == 1000 && l.length_found && l.length == 500);
        let l = c(&["lp=10,20"]);
        assert!(l.flag && l.start == 10 && l.end == 20);
        let l = c(&["COMMENT=*loopsample,0,5,90,-1"]);
        assert!(l.flag && l.start == 5 && l.end == 90);
        assert_eq!(scan_ints("12,LOOPEND=34", &[",LOOPEND="]), vec![12, 34]);
    }
}
