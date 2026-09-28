//! MCG - from PS2 Namco games [Gunvari Collection + Time Crisis (PS2), NamCollection (PS2)]
//! (vgmstream meta/mcg.c): two VAGp headers (left, right) then interleaved PS-ADPCM, stereo
//! or (three stereo pairs' worth of data) 6 channels.

use std::io;

use super::{Ctx, Found, Parser, be32, label, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "MCG",
    magics: &[b"MCG\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    if &h[0..4] != b"MCG\0" || le32(&h, 0x18) != 0 || le32(&h, 0x1c) != 0 {
        return Ok(vec![]);
    }
    let vagp_l = le32(&h, 0x04) as u64;
    let vagp_r = le32(&h, 0x08) as u64;
    let start = le32(&h, 0x0c) as u64;
    let track_size = le32(&h, 0x10) as u64; // stereo size
    let interleave = le32(&h, 0x14) as u64;
    if vagp_l < 0x20 || vagp_r < 0x20 || vagp_l + 0x30 > start || vagp_r + 0x30 > start || start > 0x10000 {
        return Ok(vec![]);
    }
    if !ctx.is(off + vagp_l, b"VAGp")? || !ctx.is(off + vagp_r, b"VAGp")? {
        return Ok(vec![]);
    }
    if interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000 || track_size == 0 {
        return Ok(vec![]);
    }
    let avail = ctx.size() - off;
    // Nothing in the header tells 6-channel files (*M.GCM) apart: vgmstream checks the data
    // size against the stereo size. Inside something bigger the file size isn't known, so
    // look for the null frame each channel starts with (only 6 channels have one there).
    let channels: u16 = if off == 0 {
        match avail.checked_sub(start) {
            Some(d) if d == track_size * 3 => 6,
            Some(d) if d == track_size => 2,
            _ => return Ok(vec![]),
        }
    } else if start + track_size * 3 <= avail && all_null(ctx, off + start, interleave)? {
        6
    } else {
        2
    };
    let data_size = track_size * if channels == 6 { 3 } else { 1 };
    if start + data_size > avail {
        return Ok(vec![]);
    }
    let l = ctx.bytes(off + vagp_l, 0x30)?;
    let r = ctx.bytes(off + vagp_r, 0x30)?;
    let channel_size = be32(&l, 0x0c) as u64; // without padding
    let rate = be32(&l, 0x10);
    if channel_size != be32(&r, 0x0c) as u64 || rate != be32(&r, 0x10) || !sane_rate(rate) {
        return Ok(vec![]);
    }
    let data_off = off + start;
    if !psx::plausible(&ctx.bytes(data_off, 0x100.min(data_size as usize))?) {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(channel_size, 1);
    if samples == 0 {
        return Ok(vec![]);
    }
    let size = data_size.next_multiple_of(interleave * channels as u64);
    let data = Data::at(ctx.entry, data_off, size);
    let t = Track::new(ctx.entry, off, "MCG", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    // Both VAGp use the same name (sometimes with an L/R letter).
    Ok(vec![Found::new(t, data_off + data_size).label(label(&l[0x20..0x30]))])
}

/// Whether channels 3 to 6 start with a null frame (as channels 1 and 2 do).
fn all_null(ctx: &mut Ctx, data: u64, interleave: u64) -> io::Result<bool> {
    for c in 2..6 {
        if ctx.bytes(data + c * interleave, 16)?.iter().any(|&b| b != 0) {
            return Ok(false);
        }
    }
    Ok(true)
}
