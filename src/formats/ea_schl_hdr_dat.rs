//! EA HDR+DAT speech banks (vgmstream meta/ea_schl_hdr_dat.c), v1 (2004-2005, e.g. Need for
//! Speed: Hot Pursuit 2 PS2) and v2 (2006-2014): a small .HDR table of sound offsets (in
//! blocks) into the .DAT, which holds SCHl streams (or VAGp files, found by the VAG format).

use std::io;

use super::ea_schl::{self, Rd};
use super::{Ctx, Found, Parser};

pub const PARSER: Parser = Parser { name: "EA HDR/DAT", magics: &[], magic_at: 0, exts: &["hdr"], locate: None, parse };

struct Hdr {
    table: u64,
    num_params: u64,
    num_sounds: u64,
    mult: u64,
}

/// The .HDR's layout if it's one (v1 first, like vgmstream), given the .DAT's first id and size.
fn header(h: &mut Rd, dat_id: u32, dat_size: u64) -> io::Result<Option<Hdr>> {
    let schl = u32::from_be_bytes(*b"SCHl");
    let vagp = u32::from_be_bytes(*b"VAGp");
    if h.size() < 0x10 {
        return Ok(None);
    }
    // v1
    if h.u16(0x0a, true)? == 0 && h.u16(0x0c, true)? == 0 && (dat_id == schl || dat_id == vagp) {
        let num_params = (h.u8(0x04)? & 0x7f) as u64;
        let num_sounds = h.u8(0x05)? as u64;
        let mult = h.u8(0x07)? as u64 * 0x100 + 0x100;
        if (h.u8(0x06)? as u64) <= num_sounds
            && (h.u16(0x08, false)? as u64 * mult <= dat_size || h.u16(0x08, true)? as u64 * mult <= dat_size)
            && num_sounds > 0
        {
            return Ok(Some(Hdr { table: 0x0c, num_params, num_sounds, mult }));
        }
    }
    // v2
    if h.u32(0x0c, true)? == 0 && h.u16(0x10, true)? == 0 && dat_id == schl {
        let num_params = (h.u8(0x02)? & 0x7f) as u64;
        let num_sounds = h.u8(0x03)? as u64;
        let mult = h.u8(0x09)? as u64 * 0x100 + 0x100;
        if (h.u8(0x08)? as u64) <= num_sounds
            && (h.u16(0x0a, false)? as u64 * mult <= dat_size || h.u16(0x0a, true)? as u64 * mult <= dat_size)
            && num_sounds > 0
        {
            return Ok(Some(Hdr { table: 0x10, num_params, num_sounds, mult }));
        }
    }
    Ok(None)
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 {
        return Ok(vec![]);
    }
    let Some((dat_entry, mut dat)) = ctx.sibling("dat") else { return Ok(vec![]) };
    let dat_id = Rd(&mut dat).id(0)?;
    let dat_size = dat.size;
    let Some(h) = header(&mut Rd(&mut ctx.r), dat_id, dat_size)? else { return Ok(vec![]) };
    let entry = ctx.entry;
    let stride = 2 + h.num_params;
    if h.table + stride * h.num_sounds > ctx.size() {
        return Ok(vec![]);
    }
    let mut found = Vec::new();
    for i in 0..h.num_sounds {
        let at = h.table + stride * i;
        let (sound_offset, params) = {
            let mut r = Rd(&mut ctx.r);
            (r.u16(at, true)? as u64 * h.mult, r.b(at + 2, h.num_params as usize)?)
        };
        if Rd(&mut dat).id(sound_offset)? != u32::from_be_bytes(*b"SCHl") {
            continue; // VAGp sounds are found in the .DAT by the VAG format
        }
        if let Some(s) = ea_schl::load_schl(&mut dat, dat_entry, sound_offset)? {
            let name = params.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ");
            found.push(Found::new(s.track(entry, 0, "EA HDR/DAT"), ctx.size()).label((!name.is_empty()).then_some(name)));
        }
    }
    Ok(found)
}

/// SCHl streams in a .DAT with its .HDR are listed through the .HDR.
pub fn claims(ctx: &mut Ctx) -> io::Result<bool> {
    if ctx.ext() != "dat" {
        return Ok(false);
    }
    let Some((_, mut hdr)) = ctx.sibling("hdr") else { return Ok(false) };
    let dat_id = Rd(&mut ctx.r).id(0)?;
    let size = ctx.size();
    Ok(header(&mut Rd(&mut hdr), dat_id, size)?.is_some())
}
