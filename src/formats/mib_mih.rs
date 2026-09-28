//! MIH+MIB - SCEE MultiStream interleaved bank (header+data) [namCollection: Ace Combat 2
//! (PS2), Rampage: Total Destruction (PS2)], and MIC, the same merged in one file [Rogue
//! Trooper (PS2), The Sims 2 (PS2)] (vgmstream meta/mib_mih.c). Found by extension (.mib
//! with its .mih, .mic and extensionless files): the header has no signature.

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::disc::Reader;
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "MIH/MIB",
    magics: &[],
    magic_at: 0,
    exts: &["mib", "mic", ""],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 {
        return Ok(vec![]);
    }
    if ctx.ext() == "mib" {
        let Some((_, mut hr)) = ctx.sibling("mih") else { return Ok(vec![]) };
        let mut header_offset = 0u64;
        let first = u32at(&mut hr, 0)? as u64;
        if first != 0x40 {
            // Marc Ecko's Getting Up (PS2): a name first
            if first > 0x1000 || u32at(&mut hr, 4 + first)? != 0x40 || u32at(&mut hr, 8 + first)? != 0x40 {
                return Ok(vec![]);
            }
            header_offset = 4 + first + 4;
        }
        let body_size = ctx.size();
        let Some(t) = multistream(&mut hr, header_offset, ctx.entry, 0, body_size, None)? else { return Ok(vec![]) };
        if !psx::plausible(&ctx.bytes(0, 0x100.min(body_size as usize))?) {
            return Ok(vec![]);
        }
        return Ok(vec![Found::new(t, body_size)]);
    }
    // MIC: header and data in one file.
    let size = ctx.size();
    if size <= 0x40 || u32at(&mut ctx.r, 0)? != 0x40 {
        return Ok(vec![]);
    }
    let entry = ctx.entry;
    let Some(t) = multistream(&mut ctx.r, 0, entry, 0x40, size, Some(size))? else { return Ok(vec![]) };
    if !psx::plausible(&ctx.bytes(0x40, 0x100.min((size - 0x40) as usize))?) {
        return Ok(vec![]);
    }
    let t = Track { format: "MIC", ..t };
    Ok(vec![Found::new(t, size)])
}

fn u32at(r: &mut Reader, off: u64) -> io::Result<u32> {
    Ok(u32::from_le_bytes(r.bytes(off, 4)?.try_into().unwrap()))
}

/// The shared header: interleaved PS-ADPCM in `body_entry` from `start`. `strict_size`:
/// the data must fit in the file (for merged files, which are also looked for without
/// an extension).
fn multistream(hr: &mut Reader, ho: u64, body_entry: usize, start: u64, body_size: u64, strict_size: Option<u64>) -> io::Result<Option<Track>> {
    let h = hr.bytes(ho, 0x18)?;
    let frame_last = (le32(&h, 0x04) >> 8) as u64;
    let channels = le32(&h, 0x08) as u64;
    let rate = le32(&h, 0x0c);
    let frame_size = le32(&h, 0x10) as u64;
    let mut frame_count = le32(&h, 0x14) as u64;
    if !(1..=8).contains(&channels) || !sane_rate(rate) || frame_size == 0 || frame_size % 0x10 != 0 || frame_size > 0x100000 || frame_last > frame_size {
        return Ok(None);
    }
    if frame_count == 0 {
        if body_size < start {
            return Ok(None);
        }
        frame_count = (body_size - start) / (frame_size * channels);
    }
    let mut data_size = frame_count * frame_size;
    if frame_last != 0 {
        data_size -= frame_size - frame_last;
    }
    data_size *= channels;
    let full = frame_count * frame_size * channels;
    if data_size == 0 || start >= body_size {
        return Ok(None);
    }
    if let Some(s) = strict_size {
        if start + full > s.next_multiple_of(frame_size * channels) {
            return Ok(None);
        }
    }
    let samples = psx::bytes_to_samples(data_size, channels as u16);
    let data = Data::at(body_entry, start, full);
    Ok(Some(Track::new(body_entry, 0, "MIH/MIB", channels as u16, rate, samples, data, Codec::Psx(psx::Params::interleaved(frame_size)))))
}
