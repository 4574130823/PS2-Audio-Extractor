//! XAVS - Reflections audio and video+audio container [Stuntman (PS2)] (vgmstream
//! meta/xavs.c). Chunks of video and PCM audio (up to 3 tracks = subsongs); each track's
//! chunks joined make interleaved stereo PCM16.

use std::io;

use super::ps2p::ZERO;
use super::{Ctx, Found, Parser, le16, le32};
use crate::codecs::{Codec, pcm};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "XAVS",
    magics: &[b"XAVS"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x18)?;
    if &h[0..4] != b"XAVS" {
        return Ok(vec![]);
    }
    let total = le16(&h, 0x0c) as usize;
    if total == 0 {
        return Ok(vec![]);
    }
    let size = ctx.size();
    // A standalone .xav is read to its end, like vgmstream (which skips unknown bytes 4 at
    // a time); elsewhere the chunks end at "_EOS" or at anything unknown.
    let whole = off == 0 && ctx.ext() == "xav";

    // Sample rate and interleave: from the first audio chunk (only video and 0x21 chunks
    // may come before it).
    let mut pos = off + 0x18;
    let (rate, interleave) = loop {
        if pos + 4 > size {
            return Ok(vec![]);
        }
        let w = ctx.u32le(pos)?;
        let (id, csize) = (w & 0xff, (w >> 8) as u64);
        match id {
            _ if id & 0xf0 == 0x40 => break (48000, 0x200),
            _ if id & 0xf0 == 0x60 => break (24000, 0x100),
            0x56 => pos += 4 + csize,
            0x21 => pos += 4,
            _ => return Ok(vec![]),
        }
    };

    // The chunks of each track (0x41 = track 1, 0x61..0x63 = tracks 1..3).
    let mut tracks: Vec<Vec<(u64, u64)>> = vec![Vec::new(); 3];
    let mut pos = off + 0x18;
    let mut buf = Vec::new();
    let mut buf_off = u64::MAX;
    let mut eos = false;
    while pos < size {
        if pos + 4 > buf_off.wrapping_add(buf.len() as u64) || pos < buf_off {
            buf_off = pos;
            buf = ctx.bytes(pos, 0x10000.min((size - pos) as usize).max(4))?;
        }
        let w = le32(&buf, (pos - buf_off) as usize);
        let (id, csize) = (w & 0xff, (w >> 8) as u64);
        let known = matches!(id, 0x41 | 0x61 | 0x62 | 0x63 | 0x56 | 0x21 | 0x5f);
        if !whole && (!known || id == 0x5f) {
            if id != 0x5f {
                return Ok(vec![]); // not ending with "_EOS": not trusted
            }
            pos += 4;
            eos = true;
            break;
        }
        match id {
            0x41 | 0x61 | 0x62 | 0x63 => {
                if !whole && pos + 4 + csize > size {
                    return Ok(vec![]);
                }
                if csize > 0 {
                    tracks[(id & 0x0f) as usize - 1].push((pos + 4, csize.min(size.saturating_sub(pos + 4))));
                }
                pos += 4 + csize;
            }
            0x56 => {
                if !whole && pos + 4 + csize > size {
                    return Ok(vec![]);
                }
                pos += 4 + csize;
            }
            _ => pos += 4,
        }
    }
    if !whole && !eos {
        return Ok(vec![]);
    }
    let end = pos.min(size);

    let mut found = Vec::new();
    for pieces in tracks.into_iter().take(total) {
        let bytes: u64 = pieces.iter().map(|p| p.1).sum();
        let samples = pcm::bytes_to_samples(bytes, 2, 16);
        if samples == 0 {
            continue;
        }
        // A short last row: vgmstream reads the rest of it as zeros.
        let mut pieces = pieces;
        let row = interleave * 2;
        if bytes % row != 0 {
            pieces.push((ZERO, row - bytes % row));
        }
        let data = Data::blocks(ctx.entry, pieces);
        let t = Track::new(ctx.entry, off, "XAVS", 2, rate, samples, data, Codec::Pcm(pcm::Params::le16(interleave)));
        found.push(Found::new(t, end));
    }
    Ok(found)
}
