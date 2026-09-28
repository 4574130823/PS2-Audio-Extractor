//! WB - from Psychonauts (PS2) (vgmstream meta/pwb.c): a bank of mono PS-ADPCM sounds at
//! 24000 Hz, one subsong per entry.

use std::io;

use super::{Ctx, Found, Parser, le32};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "PWB",
    magics: &[b"WB\x02\x00"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x30)?;
    // 04: null, 08/0c: header offset/size (0x20), 10/14: entries offset/size, 18/1c: data
    // offset/size, 20: always 1, 24: entries, 28: entry size, 2c: data offset
    if &h[0..4] != b"WB\x02\x00" || le32(&h, 0x04) != 0 || le32(&h, 0x20) != 1 {
        return Ok(vec![]);
    }
    let avail = ctx.size() - off;
    let entries = le32(&h, 0x10) as u64;
    let entries_size = le32(&h, 0x14) as u64;
    let data_offset = le32(&h, 0x18) as u64;
    let data_size = le32(&h, 0x1c) as u64;
    let total = le32(&h, 0x24) as i32;
    let entry_size = le32(&h, 0x28) as u64;
    if total <= 0 || total > 10000 || entry_size < 0x18 || entries < 0x30 {
        return Ok(vec![]);
    }
    let total = total as u64;
    if entries + entries_size.max(total * entry_size) > avail || data_offset + data_size > avail || data_offset < entries {
        return Ok(vec![]);
    }
    let table = ctx.bytes(off + entries, (total * entry_size) as usize)?;
    let mut found = Vec::new();
    for i in 0..total as usize {
        let e = i * entry_size as usize;
        let stream_offset = data_offset + le32(&table, e + 0x08) as u64;
        let stream_size = le32(&table, e + 0x0c) as u64;
        let loop_start = le32(&table, e + 0x10);
        let loop_end = le32(&table, e + 0x14).wrapping_add(loop_start);
        if stream_size == 0 || stream_offset + stream_size > data_offset + data_size {
            return Ok(vec![]);
        }
        if !psx::plausible(&ctx.bytes(off + stream_offset, 0x100.min(stream_size as usize))?) {
            return Ok(vec![]);
        }
        let samples = psx::bytes_to_samples(stream_size, 1);
        if samples == 0 {
            return Ok(vec![]);
        }
        let data = Data::at(ctx.entry, off + stream_offset, stream_size);
        let mut t = Track::new(ctx.entry, off, "PWB", 1, 24000, samples, data, Codec::Psx(psx::Params::default()));
        if loop_end != 0 {
            let (ls, le) = (psx::bytes_to_samples(loop_start as u64, 1), psx::bytes_to_samples(loop_end as u64, 1));
            if ls < le && le <= samples {
                t = t.looped(ls, le);
            }
        }
        found.push(Found::new(t, off + data_offset + data_size));
    }
    Ok(found)
}
