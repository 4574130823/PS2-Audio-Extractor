//! EA ABK ("ABKC") sound banks (vgmstream meta/ea_schl_abk.c), 6th-gen EA games: a table of
//! modules/players listing sounds that are either in the embedded BNK (RAM sounds) or SCHl
//! streams in the companion .AST file (streamed, or intro + loop pairs). Also inside the
//! .AMB/.AMX containers of EA Redwood Shores games, which the "ABKC" signature finds.

use std::io;

use super::ea_schl::{self, Rd, Sound};
use super::{Ctx, Found, Parser};

pub const PARSER: Parser = Parser { name: "EA ABK", magics: &[b"ABKC"], magic_at: 0, exts: &[], locate: None, parse };

const MAX_TABLES: usize = 0x400;

/// Entries (offsets of the 0x0c sound entries) of the ABK at `off`, and the embedded BNK's
/// offset (0 if none). None if it isn't a (SCHl-era) ABK.
fn entries(r: &mut Rd, off: u64) -> io::Result<Option<(bool, Vec<u64>, u64, u64)>> {
    let size = r.size();
    if off + 0x24 > size || r.id(off)? != u32::from_be_bytes(*b"ABKC") {
        return Ok(None);
    }
    let be = r.guess_be(off + 0x1c)?;
    let num_modules = r.u16(off + 0x0a, be)? as u64;
    let mut modules_table = r.u32(off + 0x1c, be)? as u64;
    let bnk_offset = r.u32(off + 0x20, be)? as u64;
    if bnk_offset != 0 {
        if off + bnk_offset + 8 > size {
            return Ok(None);
        }
        let m = r.b(off + bnk_offset, 4)?;
        if &m != b"BNKl" && &m != b"BNKb" {
            return Ok(None); // newer (EAAC) ABK
        }
    }
    if num_modules == 0 || num_modules > 0x1000 {
        return Ok(None);
    }
    let in_file = |o: u64, n: u64| off + o + n <= size;
    let mut tables: Vec<u64> = Vec::new();
    let mut list = Vec::new();
    let mut end = off + 0x24;
    for _ in 0..num_modules {
        if !in_file(modules_table, 0x3c) {
            return Ok(None);
        }
        let mut num_players = r.u8(off + modules_table + 0x24)? as u64;
        let module_data = r.u32(off + modules_table + 0x2c, be)? as u64;
        if num_players == 0xff || !in_file(modules_table + 0x3c, num_players * 4) {
            return Ok(None);
        }
        for j in 0..num_players {
            let player = r.u32(off + modules_table + 0x3c + 4 * j, be)? as u64;
            if !in_file(module_data + player, 8) {
                return Ok(None);
            }
            let samples_table = r.u32(off + module_data + player + 4, be)? as u64;
            if tables.contains(&samples_table) {
                continue;
            }
            if tables.len() >= MAX_TABLES || !in_file(samples_table, 4) {
                return Ok(None);
            }
            tables.push(samples_table);
            let num_sounds = r.u32(off + samples_table, be)? as u64;
            if num_sounds > 0x10000 || !in_file(samples_table + 4, 0x0c * num_sounds) {
                return Ok(None);
            }
            end = end.max(off + samples_table + 4 + 0x0c * num_sounds);
            for k in 0..num_sounds {
                let e = samples_table + 4 + 0x0c * k;
                let t = r.u8(off + e)?;
                if t == 0 && r.u32(off + e + 4, be)? == 0 {
                    continue; // dummies pointing at sound 0 of the BNK
                }
                list.push(off + e);
            }
        }
        num_players += r.u8(off + modules_table + 0x27)? as u64;
        end = end.max(off + modules_table + 0x3c + num_players * 4);
        modules_table += 0x3c + num_players * 4;
    }
    if list.is_empty() {
        return Ok(None);
    }
    Ok(Some((be, list, if bnk_offset != 0 { off + bnk_offset } else { 0 }, end)))
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let Some((be, list, bnk, mut end)) = entries(&mut Rd(&mut ctx.r), off)? else { return Ok(vec![]) };
    let entry = ctx.entry;
    let mut ast = ctx.sibling("ast");
    let mut found = Vec::new();
    for e in list {
        let (t, a, b) = {
            let mut r = Rd(&mut ctx.r);
            (r.u8(e)?, r.u32(e + 4, be)? as u64, r.u32(e + 8, be)? as u64)
        };
        let sound: Option<Sound> = match t {
            0 if bnk != 0 => ea_schl::load_bnk(&mut ctx.r, entry, bnk, a as usize, true)?.map(|s| s.sound),
            1 => match ast.as_mut() {
                Some((ai, ar)) => {
                    if Rd(ar).id(a)? == u32::from_be_bytes(*b"SCHl") { ea_schl::load_schl(ar, *ai, a)? } else { None }
                }
                None => None,
            },
            2 => match ast.as_mut() {
                Some((ai, ar)) => {
                    let schl = u32::from_be_bytes(*b"SCHl");
                    if Rd(ar).id(a)? == schl && Rd(ar).id(b)? == schl {
                        match (ea_schl::load_schl(ar, *ai, a)?, ea_schl::load_schl(ar, *ai, b)?) {
                            (Some(intro), Some(body)) => {
                                let loop_start = intro.samples;
                                intro.append(body).map(|mut s| {
                                    s.loops = Some((loop_start, s.samples));
                                    s
                                })
                            }
                            _ => None,
                        }
                    } else {
                        None
                    }
                }
                None => None,
            },
            _ => None,
        };
        if let Some(s) = sound {
            if s.data.entry == entry {
                end = end.max(s.end);
            }
            found.push(s);
        }
    }
    Ok(found.into_iter().map(|s| Found::new(s.track(entry, off, "EA ABK"), end)).collect())
}

/// SCHl streams in an .AST next to an ABK bank (or .AMB/.AMX) are listed through the bank.
pub fn claims(ctx: &mut Ctx) -> io::Result<bool> {
    if ctx.ext() != "ast" {
        return Ok(false);
    }
    if let Some((_, mut r)) = ctx.sibling("abk") {
        if entries(&mut Rd(&mut r), 0)?.is_some() {
            return Ok(true);
        }
    }
    Ok(ctx.sibling("amb").is_some() || ctx.sibling("amx").is_some())
}
