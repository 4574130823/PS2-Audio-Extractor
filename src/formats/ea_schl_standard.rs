//! EA SCHl streams and BNK banks (vgmstream meta/ea_schl_standard.c: ea_schl, ea_bnk and
//! ea_schl_video), from EA games of ~1997-2010 (sx.exe). SCHl streams (.asf/.str/.sng/...,
//! also inside videos and bigfiles) start with "SCHl", or "SHxx" per language in videos;
//! banks with "BNKl"/"BNKb". PS2 ones are mostly PS-ADPCM or EA-XA.

use std::io;

use super::ea_schl::{self, LANGS, Rd};
use super::{Ctx, Found, Parser};

pub const PARSER: Parser = Parser {
    name: "EA SCHl",
    magics: &[
        b"SCHl", b"BNKl", b"BNKb", b"SHEN", b"SHFR", b"SHGE", b"SHDE", b"SHIT", b"SHSP", b"SHES", b"SHMX", b"SHRU", b"SHJA", b"SHJP",
        b"SHPL", b"SHBR",
    ],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let head = ctx.bytes(off, 4)?;
    if &head == b"BNKl" || &head == b"BNKb" {
        // (music banks in a .MUS are listed through its map file)
        if super::ea_schl_map_mpf_mus::claims(ctx)? {
            return Ok(vec![]);
        }
        return bnk(ctx, off);
    }
    // Streams that a companion index file lists are found through it instead.
    if super::ea_schl_hdr_dat::claims(ctx)? || super::ea_schl_abk::claims(ctx)? || super::ea_schl_map_mpf_mus::claims(ctx)? {
        return Ok(vec![]);
    }
    if &head[0..2] == b"SH" {
        // A group of per-language headers: one subsong each (ea_schl_video).
        let mut found = Vec::new();
        let mut pos = off;
        let be = Rd(&mut ctx.r).guess_be(off + 4)?;
        loop {
            let (h, size) = {
                let mut rd = Rd(&mut ctx.r);
                (rd.b(pos, 4)?, rd.u32(pos + 4, be)? as u64)
            };
            if &h[0..2] != b"SH" || !LANGS.iter().any(|l| l[..] == h[2..4]) || size == 0 {
                break;
            }
            let entry = ctx.entry;
            match ea_schl::load_schl(&mut ctx.r, entry, pos)? {
                Some(s) => {
                    let end = s.end;
                    found.push(Found::new(s.track(entry, pos, "EA SCHl"), end));
                }
                None => break,
            }
            pos += size;
        }
        return Ok(found);
    }
    let entry = ctx.entry;
    Ok(match ea_schl::load_schl(&mut ctx.r, entry, off)? {
        Some(s) => {
            let end = s.end;
            vec![Found::new(s.track(entry, off, "EA SCHl"), end)]
        }
        None => vec![],
    })
}

/// A standalone bank: every non-empty sound, in order.
fn bnk(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let entry = ctx.entry;
    let list = ea_schl::bnk_real_sounds(&mut ctx.r, off)?;
    if list.is_empty() || list.len() > 4096 {
        return Ok(vec![]);
    }
    let mut sounds = Vec::new();
    for i in list {
        // (the n-th non-empty entry is table entry i; an entry vgmstream can't open is
        // skipped, like it fails that subsong)
        if let Some(s) = ea_schl::load_bnk(&mut ctx.r, entry, off, i, true)? {
            sounds.push(s);
        }
    }
    let end = sounds.iter().map(|s| s.sound.end.max(s.header)).max().unwrap_or(off);
    Ok(sounds.into_iter().map(|s| Found::new(s.sound.track(entry, off, "EA BNK"), end)).collect())
}
