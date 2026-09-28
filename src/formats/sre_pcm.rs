//! .SRE+.PCM - Capcom's header+data container [Viewtiful Joe (PS2)] (vgmstream
//! meta/sre_pcm.c): the .SRE lists PS-ADPCM streams (subsongs) that are in the .PCM next to
//! it. Known only by extension.

use std::io;

use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "SRE+PCM",
    magics: &[],
    magic_at: 0,
    exts: &["sre"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 || ctx.size() < 0x10 {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, 0x10)?;
    let table1_entries = le32(&h, 0x00) as i32;
    let table1_offset = le32(&h, 0x04) as u64;
    let table2_entries = le32(&h, 0x08) as i32;
    let table2_offset = le32(&h, 0x0c) as u64;
    if table1_entries <= 0 || table1_entries >= 0x100 || table1_entries as u64 * 0x60 + table1_offset != table2_offset {
        return Ok(vec![]);
    }
    if table2_entries < 1 || table2_offset + table2_entries as u64 * 0x20 > ctx.size() {
        return Ok(vec![]);
    }
    let Some((pcm_entry, pcm)) = ctx.sibling("pcm") else { return Ok(vec![]) };
    let pcm_size = pcm.size;
    let table = ctx.bytes(table2_offset, table2_entries as usize * 0x20)?;
    let mut found = Vec::new();
    for i in 0..table2_entries as usize {
        let e = i * 0x20;
        let channels = le32(&table, e) as i32;
        let rate = le16(&table, e + 0x04) as u32;
        let start = le32(&table, e + 0x08) as u64;
        let stream_size = le32(&table, e + 0x0c) as u64;
        let loop_start = le32(&table, e + 0x10) as u64;
        let loop_end = le32(&table, e + 0x14) as u64;
        let loop_flag = le32(&table, e + 0x18) != 0;
        if !(1..=8).contains(&channels) || !sane_rate(rate) || stream_size == 0 || start + stream_size > pcm_size {
            return Ok(vec![]);
        }
        let channels = channels as u16;
        let samples = psx::bytes_to_samples(stream_size, channels);
        if samples == 0 {
            return Ok(vec![]);
        }
        // Whole 0x1000 blocks are read, even in the last row.
        let size = if channels > 1 { stream_size.next_multiple_of(0x1000 * channels as u64) } else { stream_size };
        let data = Data::at(pcm_entry, start, size);
        let mut t = Track::new(ctx.entry, 0, "SRE+PCM", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(0x1000)));
        if loop_flag {
            let (ls, le) = (psx::bytes_to_samples(loop_start, 1), psx::bytes_to_samples(loop_end, 1));
            if ls < le && le <= samples {
                t = t.looped(ls, le);
            }
        }
        found.push(Found::new(t, ctx.size()));
    }
    Ok(found)
}
