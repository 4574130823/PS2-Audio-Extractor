//! .JOE - from Asobo Studio games [Up (PS2), Wall-E (PS2), Sitting Ducks (PS2)] (vgmstream
//! meta/joe.c). Stereo PS-ADPCM; the header layout depends on the game. No signature:
//! found by extension.

use std::io;

use super::ps2p::vgm_interleaved;
use super::{Ctx, Found, Parser, le32};
use crate::codecs::{Codec, psx};
use crate::track::Track;

pub const PARSER: Parser = Parser {
    name: "JOE",
    magics: &[],
    magic_at: 0,
    exts: &["joe"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 || ctx.size() < 0x20 {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, 0x10)?;
    let rate = le32(&h, 0) as i32;
    if !(8000..=48000).contains(&rate) {
        return Ok(vec![]);
    }
    let mut data_size = le32(&h, 4) as u64;
    let (u1, u2) = (le32(&h, 8), le32(&h, 0x0c));
    let file_size = ctx.size();
    let (interleave, start);
    if data_size == file_size.wrapping_sub(0x800) && u1 == 0x2000 && u2 == 0xffff_ffff {
        (interleave, start) = (0x2000, 0x800); // NYR
    } else if data_size / 2 == file_size - 0x10 && u1 == 0x0045_039a && u2 == 0x0010_8920 {
        data_size /= 2; // Super Farm
        (interleave, start) = (0x4000, 0x10);
    } else if data_size / 2 == file_size - 0x10 && u1 == 0xcccc_cccc && u2 == 0xcccc_cccc {
        data_size /= 2; // Sitting Ducks
        (interleave, start) = (0x8000, 0x10);
    } else if data_size == file_size - 0x10 && u1 == 0xcccc_cccc && u2 == 0xcccc_cccc {
        (interleave, start) = (0x8000, 0x10); // The Mummy: The Animated Series
    } else if data_size == file_size.wrapping_sub(0x4020) {
        (interleave, start) = (0x10, 0x4020); // Counter Terrorism Special Forces and later
    } else {
        return Ok(vec![]);
    }
    let channels = 2u64;
    let padding = find_padding(ctx, start, data_size, channels, interleave)?;
    let samples = psx::bytes_to_samples(data_size - padding, 2);
    if samples == 0 || !psx::plausible(&ctx.bytes(start, 0x100.min(data_size as usize))?) {
        return Ok(vec![]);
    }
    let data = vgm_interleaved(ctx.entry, start, data_size, channels, interleave, file_size);
    let t = Track::new(ctx.entry, 0, "JOE", 2, rate as u32, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, file_size)])
}

/// vgmstream's joe_find_padding: 0xCDCDCDCD or zero frames at the end (channel 0's).
fn find_padding(ctx: &mut Ctx, start: u64, data_size: u64, channels: u64, interleave: u64) -> io::Result<u64> {
    if data_size == 0 || interleave == 0 {
        return Ok(0);
    }
    let skip = interleave * (channels - 1);
    let mut offset = (start + data_size) as i64 - skip as i64;
    let min = start as i64;
    let mut padding = 0u64;
    let mut consumed = 0u64;
    while offset > min {
        offset -= 0x10;
        let b = ctx.bytes(offset as u64, 4)?;
        let pad = u32::from_be_bytes(b.try_into().unwrap());
        // (past the end of the file vgmstream reads -1, which isn't padding)
        if offset as u64 + 4 > ctx.size() || (pad != 0xcdcd_cdcd && pad != 0) {
            break;
        }
        padding += 0x10 * channels;
        consumed += 0x10;
        if consumed == interleave {
            consumed = 0;
            offset -= skip as i64;
        }
    }
    Ok(if padding >= data_size { 0 } else { padding })
}
