//! SKEX - from SCE America second party devs [Syphon Filter: Dark Mirror (PS2/PSP), MLB
//! 2004 (PS2), NBA 06 (PS2)] (vgmstream meta/skex.c). A pack of whole files (VAG, VPK,
//! AT3) listed in a table: in the .skx itself or in a .tbl next to it. VAGs are read here,
//! VPKs by the VPK parser; AT3 (ATRAC3) entries and the Vita (AT9) version are skipped.

use std::io;

use super::ps2p::vag_subfile;
use super::{Ctx, Found, Parser, le16, le32};
use crate::disc::Reader;

pub const PARSER: Parser = Parser {
    name: "SKEX",
    magics: &[b"SKEX"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x1c)?;
    if &h[0..4] != b"SKEX" {
        return Ok(vec![]);
    }
    let size = ctx.size();
    let version = le16(&h, 0x04);
    let head_offset = le32(&h, 0x10) as u64;
    let head_size = le32(&h, 0x14) as u64;
    let entries = le16(&h, 0x18) as u64;
    if size - off <= 0x100 {
        return Ok(vec![]);
    }
    // The table: inside (rare), or the .tbl.
    let (mut tbl, tbl_base): (Reader, u64) = if head_offset != 0 && head_size != 0 {
        if head_offset + head_size > size - off {
            return Ok(vec![]);
        }
        (ctx.game.reader(&ctx.entries[ctx.entry])?, off)
    } else {
        if off != 0 {
            return Ok(vec![]);
        }
        match ctx.sibling("tbl") {
            Some((_, r)) => (r, 0),
            None => return Ok(vec![]),
        }
    };
    let (pos, entry_size, type_at) = match version {
        0x1070 => (head_offset, 0x0c, 4),
        0x2010 => (head_offset, 8, 7),
        0x2040 | 0x2070 | 0x3000 | 0x3200 => {
            if tbl.bytes(tbl_base + head_offset, 4)? != b"STBL" {
                return Ok(vec![]);
            }
            (head_offset + 0x50, 8, 7)
        }
        _ => return Ok(vec![]), // 0x5100: Vita (AT9)
    };
    if entries == 0 || entries > 0x10000 {
        return Ok(vec![]);
    }
    let table = tbl.bytes(tbl_base + pos, ((entries + 1) * entry_size) as usize)?;
    let modern = version >= 0x2040;
    let mut subs = Vec::new();
    let mut prev = 0u64;
    for i in 0..entries as usize {
        let e = i * entry_size as usize;
        let curr = le32(&table, e) as u64;
        let kind = if type_at == 4 { le32(&table, e + 4) } else { table[e + 7] as u32 };
        match kind {
            0x00 | 0x01 | 0x0e if modern => continue,
            0x05 | 0x0c => {}
            0x09 | 0x0b if modern => {}
            _ => return Ok(vec![]), // vgmstream fails the whole bank
        }
        if curr == prev {
            continue;
        }
        prev = curr;
        let next = le32(&table, e + entry_size as usize) as u64;
        subs.push((curr, next.wrapping_sub(curr) & 0xffff_ffff, kind));
    }
    let mut end = if tbl_base == off && head_offset != 0 { off + head_offset + head_size } else { off };
    for &(o, s, _) in &subs {
        if off + o + s <= size {
            end = end.max(off + o + s);
        }
    }
    let mut found = Vec::new();
    for (o, s, kind) in subs {
        let at = off + o;
        if s == 0 || at + s > size {
            continue;
        }
        match kind {
            0x05 | 0x0c => {
                if let Some(v) = vag_subfile(&mut ctx.r, at, s)? {
                    let name = v.name.clone();
                    found.push(Found::new(v.track(ctx.entry, off, "SKEX"), end).label(name));
                }
            }
            0x0b => {
                for f in (super::vpk::PARSER.parse)(ctx, at)? {
                    found.push(Found { end, ..f });
                }
            }
            _ => {} // AT3
        }
    }
    Ok(found)
}
