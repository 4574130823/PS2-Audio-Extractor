//! HD+BD / HBD: Sony's PS2 sound bank (vgmstream meta/hd_bd.c). The .HD header lists
//! samples that are PS-ADPCM in the matching .BD, or right after the header when both are
//! pasted together (.HBD).

use std::io;

use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "HD/BD",
    magics: &[b"IECSsreV"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x40)?;
    if &h[0..8] != b"IECSsreV" || &h[0x10..0x18] != b"IECSdaeH" {
        return Ok(vec![]);
    }
    let hd_size = le32(&h, 0x1c) as u64;
    let bd_size = le32(&h, 0x20) as u64;
    let vagi = le32(&h, 0x30) as u64;
    if hd_size < 0x40 || vagi + 0x10 > hd_size || off + hd_size > ctx.size() || bd_size == 0 {
        return Ok(vec![]);
    }
    let hd = ctx.bytes(off, hd_size as usize)?;
    let v = vagi as usize;
    if &hd[v..v + 8] != b"IECSigaV" {
        return Ok(vec![]);
    }
    let rd = |at: usize| -> Option<u32> { hd.get(at..at + 4).map(|b| le32(b, 0)) };
    let mut total = le32(&hd, v + 0x0c) as i32 as i64;
    if !(0..10000).contains(&total) {
        return Ok(vec![]);
    }
    // often there is an extra subsong
    if rd(v + 0x10 + 4 * total as usize).unwrap_or(0) != 0 {
        total += 1;
    }
    // PrincessSoft: the last one is a dummy pointing at the end of the .bd
    if total > 0 {
        let last = rd(v + 0x10 + 4 * (total as usize - 1)).unwrap_or(0) as usize;
        if last != 0 && rd(v + last).map(|o| o as u64) == Some(bd_size) {
            total -= 1;
        }
    }
    let total = total as usize;
    if total == 0 {
        return Ok(vec![]);
    }

    // Where the samples are: the .BD next to a standalone .HD, or pasted after the header
    // (.HBD files; banks inside archives, sometimes after alignment padding).
    let standalone = off == 0 && ctx.size() == hd_size;
    let (bd_entry, bd_base, separate) = if standalone {
        match ctx.sibling("bd") {
            Some((e, r)) if r.size == bd_size => (e, 0, true),
            _ => return Ok(vec![]),
        }
    } else {
        let end = off + hd_size;
        let mut base = None;
        for c in [end, end.next_multiple_of(0x10), end.next_multiple_of(0x800)] {
            if c + bd_size <= ctx.size() && psx::plausible(&ctx.bytes(c, 0x200.min(bd_size as usize))?) {
                base = Some(c);
                break;
            }
        }
        match base {
            Some(b) => (ctx.entry, b, false),
            None => return Ok(vec![]),
        }
    };

    let info_at = |i: usize| rd(v + 0x10 + 4 * i).map(|r| v + r as usize);
    let mut found = Vec::new();
    for target in 0..total {
        let Some(info) = info_at(target) else { return Ok(vec![]) };
        if info + 8 > hd.len() {
            continue;
        }
        let stream = le32(&hd, info) as u64;
        let rate = le16(&hd, info + 4) as u32;
        let (flags, unknown) = (hd[info + 6], hd[info + 7]);
        if flags > 1 || (unknown != 0 && unknown != 0xff) || !sane_rate(rate) {
            continue; // vgmstream refuses these subsongs
        }
        // size up to the next entry's (different) offset
        let mut next = 0u64;
        for i in target..total - 1 {
            next = info_at(i + 1).and_then(|n| rd(n)).unwrap_or(0) as u64;
            if next != stream {
                break;
            }
        }
        if next == 0 || next == stream {
            next = bd_size;
        }
        if next <= stream || next > bd_size {
            continue;
        }
        let size = next - stream;
        let samples = psx::bytes_to_samples(size, 1);
        if samples == 0 {
            continue;
        }
        let data = Data::at(bd_entry, bd_base + stream, size);
        let mut t = Track::new(ctx.entry, off, "HD/BD", 1, rate, samples, data, Codec::Psx(psx::Params::default()));
        if flags & 1 != 0 {
            t = t.looped(0, samples); // the bank's loop flag loops the whole sample
        }
        // A pasted .BD is part of this bank, so nothing else is looked for inside it.
        found.push(Found::new(t, if separate { off + hd_size } else { bd_base + bd_size }));
    }
    Ok(found)
}
