//! .VSV - from Square Enix games [Dawn of Mana: Seiken Densetsu 4 (PS2), Kingdom Hearts
//! Re:Chain of Memories (PS2), Romancing SaGa (PS2)] (vgmstream meta/vsv.c). No signature:
//! found by extension only.

use std::io;

use super::ps2p::vgm_loop;
use super::{Ctx, Found, Parser, le16, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "VSV",
    magics: &[],
    magic_at: 0,
    exts: &["vsv", "psh"],
    locate: None,
    parse,
};

const INTERLEAVE: u64 = 0x800;

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 || ctx.size() < 0x10 {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, 0x10)?;
    if h[0x03] > 0x64 || h[0x0a] != 0 {
        return Ok(vec![]);
    }
    let is_rs = le16(&h, 0x00) == 0;
    let adjust = le16(&h, 0x04) as u64;
    let loop_word = le16(&h, 0x06) as u64;
    let loop_start = (loop_word & 0x7fff) * INTERLEAVE;
    let loop_flag = loop_word & 0x8000 != 0;
    let rate = le16(&h, 0x08) as u32;
    let flags = h[0x0b];
    let full_size = le16(&h, 0x0c) as u64 * INTERLEAVE;
    let channels: u64 = if flags & 1 != 0 { 2 } else { 1 };
    let mut data_size = full_size;
    if !is_rs {
        let discard = adjust & 0x07ff;
        match data_size.checked_sub((0x800 - discard) * channels) {
            Some(s) => data_size = s,
            None => return Ok(vec![]),
        }
    }
    if !sane_rate(rate) || data_size == 0 || full_size > ctx.size() {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(data_size, channels as u16);
    let mut loop_start_sample = psx::bytes_to_samples(loop_start, channels as u16) as i64;
    if is_rs {
        loop_start_sample -= psx::bytes_to_samples(channels * INTERLEAVE, channels as u16) as i64;
        loop_start_sample -= psx::bytes_to_samples(0x200 * channels, channels as u16) as i64;
    }
    // The data starts at 0, over the header, which vgmstream reads as zeros: in its place
    // goes any frame whose flag byte is 7 or more (decoding to silence, like a zeroed one).
    // The sample rate's high byte, at 0x09, is one (the rate is at least 4000).
    let probe = ctx.bytes(0x10, 0x100.min((full_size - 0x10) as usize))?;
    if !psx::plausible(&probe) {
        return Ok(vec![]);
    }
    let data = Data::blocks(ctx.entry, vec![(0x08, 0x10), (0x10, full_size - 0x10)]);
    let mut t = Track::new(ctx.entry, 0, "VSV", channels as u16, rate, samples, data, Codec::Psx(psx::Params::interleaved(INTERLEAVE)));
    if loop_flag {
        t = vgm_loop(t, loop_start_sample, samples as i64);
    }
    Ok(vec![Found::new(t, full_size)])
}
