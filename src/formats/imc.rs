//! .IMC - from iNiS Gitaroo Man (PS2) (vgmstream meta/imc.c): single streams and the
//! containers holding them. No signature: found by extension.

use std::io;

use super::ps2p::{Sub, ps_find_padding, vgm_interleaved};
use super::{Ctx, Found, Parser, label, le32};
use crate::codecs::{Codec, psx};
use crate::track::Track;

pub const PARSER: Parser = Parser {
    name: "IMC",
    magics: &[],
    magic_at: 0,
    exts: &["imc"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 || ctx.size() < 0x10 {
        return Ok(vec![]);
    }
    let size = ctx.size();
    if let Some(t) = single(ctx, 0, size)? {
        return Ok(vec![Found::new(t, size)]);
    }
    // Container: count, then 0x20 entries (name, ..., offset at 0x10).
    let total = le32(&ctx.bytes(0, 4)?, 0) as u64;
    if total < 1 || 4 + total * 0x20 > size {
        return Ok(vec![]);
    }
    let table = ctx.bytes(4, (total * 0x20) as usize)?;
    let mut found = Vec::new();
    for i in 0..total as usize {
        let sub_off = le32(&table, i * 0x20 + 0x10) as u64;
        let next = if i + 1 == total as usize { size } else { le32(&table, (i + 1) * 0x20 + 0x10) as u64 };
        if sub_off < 4 + total * 0x20 || next <= sub_off || next > size {
            continue;
        }
        if let Some(t) = single(ctx, sub_off, next - sub_off)? {
            found.push(Found::new(t, size).label(label(&table[i * 0x20..i * 0x20 + 8])));
        }
    }
    Ok(found)
}

/// An IMC stream: the sub-file [base, base + size).
fn single(ctx: &mut Ctx, base: u64, size: u64) -> io::Result<Option<Track>> {
    if size < 0x10 {
        return Ok(None);
    }
    let h = ctx.bytes(base, 0x10)?;
    let channels = le32(&h, 0) as i32;
    let rate = le32(&h, 4) as i32;
    let interleave = (le32(&h, 8) as i32 as i64) * 0x10;
    let blocks = le32(&h, 0x0c) as i32 as i64;
    if !(1..=8).contains(&channels) || !(11025..=48000).contains(&rate) {
        return Ok(None);
    }
    if interleave <= 0 || blocks <= 0 || interleave * blocks + 0x10 != size as i64 {
        return Ok(None);
    }
    let (channels, interleave) = (channels as u64, interleave as u64);
    let mut data_size = size - 0x10;
    data_size = data_size.saturating_sub(ps_find_padding(&mut Sub::new(&mut ctx.r, base, size), 0x10, data_size, channels, interleave, false)?);
    let samples = psx::bytes_to_samples(data_size, channels as u16);
    if samples == 0 || !psx::plausible(&ctx.bytes(base + 0x10, 0x100.min(size as usize - 0x10))?) {
        return Ok(None);
    }
    let data = vgm_interleaved(ctx.entry, base + 0x10, size - 0x10, channels, interleave, base + size);
    Ok(Some(Track::new(ctx.entry, base, "IMC", channels as u16, rate as u32, samples, data, Codec::Psx(psx::Params::interleaved(interleave)))))
}
