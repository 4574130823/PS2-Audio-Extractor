//! RSD - from Radical Entertainment games [The Simpsons: Road Rage / Hit & Run, Dark Summit,
//! Hulk, Crash Tag Team Racing, Scarface...] (vgmstream meta/rsd.c).
//!
//! The header has no data size: vgmstream plays to the end of the file. Standalone .rsd
//! files do that too; inside other files (Radical's .rcf archives) the data runs as far as
//! it looks like its codec (PS-ADPCM, Xbox/Radical IMA). PCM can only be found standalone.

use std::io;

use super::{Ctx, Found, Parser, be32, label, le16, le32, sane_rate, split_ext};
use crate::codecs::{Codec, ima, pcm, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "RSD",
    magics: &[b"RSD2", b"RSD3", b"RSD4", b"RSD6"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x80)?;
    if &h[0..3] != b"RSD" {
        return Ok(vec![]);
    }
    let codec = be32(&h, 0x04);
    let channels = le32(&h, 0x08);
    let rate = le32(&h, 0x10);
    let (interleave, start, name) = match h[3] {
        b'2' | b'3' => {
            let start = if &h[4..8] == b"GADP" { 0xa0 } else { le32(&h, 0x18) as u64 };
            (le32(&h, 0x14) as u64, start, None)
        }
        b'4' => {
            let early = matches!(&h[4..8], b"PCM " | b"PCMB" | b"GADP") && !ctx.is(off + 0x80, b"----")?;
            (0, if early { 0x80 } else { 0x800 }, None)
        }
        b'6' => {
            // The dev's file path: keep the file name, without its extension.
            let name = label(&ctx.bytes(off + 0x18, 0x100)?).map(|n| {
                let file = n.rsplit(['/', '\\']).next().unwrap_or(&n).to_string();
                split_ext(&file).0.to_string()
            });
            (0, 0x800, name.filter(|n| !n.is_empty()))
        }
        _ => return Ok(vec![]),
    };
    if !(1..=8).contains(&channels) || !sane_rate(rate) || !(0x14..=0x10000).contains(&start) {
        return Ok(vec![]);
    }
    let ch = channels as u16;
    let chs = channels as u64;
    let data_off = off + start;
    if data_off >= ctx.size() {
        return Ok(vec![]);
    }
    let standalone = off == 0 && matches!(ctx.ext().as_str(), "rsd" | "rsp");
    let avail = ctx.size() - data_off;
    let mut note = None;
    let (size, samples, codec) = match &codec.to_be_bytes() {
        b"PCM " | b"PCMB" => {
            if !standalone {
                return Ok(vec![]);
            }
            let p = if &h[4..8] == b"PCM " { pcm::Params::le16(2) } else { pcm::Params::be16(2) };
            (avail, pcm::bytes_to_samples(avail, ch, 16), Codec::Pcm(p))
        }
        b"VAG " => {
            let il = if interleave == 0 { 0x10 } else { interleave };
            if il > 0x10000 {
                return Ok(vec![]);
            }
            let size = if standalone { avail } else { run_length(ctx, data_off, avail, 16, psx::plausible)? };
            let probe = ctx.bytes(data_off, 0x100.min(size) as usize)?;
            if size == 0 || !psx::plausible(&probe) {
                return Ok(vec![]);
            }
            // Whole interleave rows, as vgmstream reads them (no shorter last block).
            let rows = if ch > 1 { size.div_ceil(il * chs) * il * chs } else { size };
            (rows, psx::bytes_to_samples(size, ch), Codec::Psx(psx::Params::interleaved(il)))
        }
        b"XADP" => {
            let frame = 0x24 * chs;
            let ok = |f: &[u8]| ima::xbox_plausible(f, ch);
            let size = if standalone { avail } else { run_length(ctx, data_off, avail, frame, ok)? };
            let probe = ctx.bytes(data_off, (frame * 4).min(size) as usize)?;
            if size == 0 || !ima::xbox_plausible(&probe, ch) {
                return Ok(vec![]);
            }
            let kind = if ch > 2 { ima::Kind::XboxMch } else { ima::Kind::Xbox };
            (size, ima::xbox_bytes_to_samples(size, ch), Codec::Ima(ima::Params::new(kind, 0)))
        }
        b"RADP" => {
            let frame = 0x14 * chs;
            let ok = |f: &[u8]| (0..chs as usize).all(|c| (0..=88).contains(&(le16(f, 4 * c) as i16)));
            let size = if standalone { avail } else { run_length(ctx, data_off, avail, frame, ok)? };
            if size < frame {
                return Ok(vec![]);
            }
            (size, size / 0x14 / chs * 32, Codec::Ima(ima::Params::new(ima::Kind::Rad, 0)))
        }
        b"GADP" | b"WADP" | b"OOGV" | b"WMA " | b"AT3+" | b"XMA " => {
            if !standalone {
                return Ok(vec![]);
            }
            let what = match &h[4..8] {
                b"GADP" | b"WADP" => "GameCube/Wii DSP ADPCM",
                b"OOGV" => "Ogg Vorbis",
                b"WMA " => "WMA",
                b"AT3+" => "ATRAC3plus",
                _ => "XMA",
            };
            note = Some(format!("{what} audio isn't supported"));
            let samples = if matches!(&h[4..8], b"GADP" | b"WADP") { avail / chs / 8 * 14 } else { 0 };
            (avail, samples, Codec::None)
        }
        _ => return Ok(vec![]),
    };
    let data = Data::at(ctx.entry, data_off, size);
    let mut t = Track::new(ctx.entry, off, "RSD", ch, rate, samples, data, codec);
    t.note = note;
    Ok(vec![Found::new(t, (data_off + size).min(ctx.size())).label(name)])
}

/// Bytes from `start` that are whole frames of `frame` bytes passing `ok`, up to `max`.
fn run_length(ctx: &mut Ctx, start: u64, max: u64, frame: u64, ok: impl Fn(&[u8]) -> bool) -> io::Result<u64> {
    let chunk = (0x10000 / frame).max(1) * frame;
    let mut len = 0u64;
    while len + frame <= max {
        let n = chunk.min((max - len) / frame * frame);
        let buf = ctx.bytes(start + len, n as usize)?;
        for f in buf.chunks_exact(frame as usize) {
            if !ok(f) {
                return Ok(len);
            }
            len += frame;
        }
    }
    Ok(len)
}
