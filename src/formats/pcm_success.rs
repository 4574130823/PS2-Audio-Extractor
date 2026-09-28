//! PCM - from Success (related) games [Metal Saga (PS2), Tetris Kiwamemichi (PS2), Duel
//! Masters: Rebirth of Super Dragon (PS2)] (vgmstream meta/pcm_success.c): PS-ADPCM in 0x800
//! blocks per channel, sizes and loops counted in blocks.

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "PCM (Success)",
    magics: &[b"PCM \x00\x00\x01\x00"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    const IL: i64 = 0x800;
    let h = ctx.bytes(off, 0x30)?;
    if &h[0..4] != b"PCM " || le32(&h, 0x04) != 0x0001_0000 {
        return Ok(vec![]);
    }
    let avail = ctx.size() - off;
    let size_field = le32(&h, 0x08) as u64; // data size without padding
    // Standalone, the file is the data plus some padding (as vgmstream checks); inside
    // something bigger, all that can be checked is that the data is there.
    if (off == 0 && size_field + 0x8000 < avail) || avail <= 0x800 {
        return Ok(vec![]);
    }
    let s32 = |at: usize| le32(&h, at) as i32 as i64;
    let rate = le32(&h, 0x0c);
    let channels = s32(0x10);
    let loop_flag = s32(0x14) != 0;
    if !(1..=8).contains(&channels) || !sane_rate(rate) {
        return Ok(vec![]);
    }
    let blocks = s32(0x18);
    if blocks <= 0 {
        return Ok(vec![]);
    }
    let mut data_size = (blocks * IL * channels) as u64;
    // Loops seem slightly off, so the 'adjust' values may need tweaking.
    let loop_start = s32(0x20) * IL * channels + s32(0x1c) * channels;
    let loop_end = s32(0x28) * IL * channels + (IL * channels - s32(0x24) * channels);
    let body = avail - 0x800;
    if data_size > body {
        if off != 0 {
            return Ok(vec![]);
        }
        data_size = body; // not always accurate and has padding
    }
    let channels = channels as u16;
    let start = off + 0x800;
    if !psx::plausible(&ctx.bytes(start, 0x100.min(data_size as usize))?) {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(data_size, channels);
    if samples == 0 {
        return Ok(vec![]);
    }
    let size = data_size.next_multiple_of(IL as u64 * channels as u64);
    let data = Data::at(ctx.entry, start, size);
    let mut t = Track::new(ctx.entry, off, "PCM", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(IL as u64)));
    if loop_flag && loop_start >= 0 && loop_end >= 0 {
        let ls = psx::bytes_to_samples(loop_start as u64, channels);
        let le = psx::bytes_to_samples(loop_end as u64, channels);
        if ls < le && le <= samples {
            t = t.looped(ls, le);
        }
    }
    let end = if off == 0 { ctx.size() } else { (start + size).min(ctx.size()) };
    Ok(vec![Found::new(t, end)])
}
