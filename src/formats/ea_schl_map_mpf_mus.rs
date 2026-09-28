//! EA MAP/MPF + MUS interactive music (vgmstream meta/ea_schl_map_mpf_mus.c): a PathFinder
//! .MAP/.MPF ("PFDx") lists music segments that are SCHl streams (or BNK sounds) in a
//! companion .MUS. MAP: Need for Speed II/III, SSX; MPF v3-v5: SSX Tricky/3, NFS
//! Underground 2, Harry Potter, NFS Most Wanted... Also .MSB/.MSX containers (007: From
//! Russia with Love, The Godfather), which name their .MUS.
//!
//! Pairs vgmstream can't open without a .txtm list aren't found here either.

use std::io;

use super::ea_schl::{self, Rd, Sound};
use super::{Ctx, Found, Parser};
use crate::disc::Reader;

pub const PARSER: Parser = Parser { name: "EA MPF/MUS", magics: &[], magic_at: 0, exts: &["map", "lin", "mpf", "msb", "msx"], locate: None, parse };

/// open_mapfile_pair's name table: (map name, mus names by track). '*' is the name's prefix.
const PAIRS: [(&str, &str); 7] = [
    ("MUS_CTRL.MPF", "MUS_STR.MUS"),
    ("mus_ctrl.mpf", "mus_str.mus"),
    ("AKA_Mus.mpf", "Track.mus"),
    ("SSX4FE.mpf", "TrackFE.mus"),
    ("SSX4Path.mpf", "Track.mus"),
    ("SSX4.mpf", "moments0.mus,main.mus,load_loop0.mus"),
    ("*.mpf", "*_main.mus"),
];

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The .MUS for `track` of the map file `ctx` is on (open_mapfile_pair).
fn open_pair(ctx: &Ctx, map_name: &str, track: i64) -> Option<(usize, Reader)> {
    if track == 0 {
        if let Some(p) = ctx.sibling("mus") {
            return Some(p);
        }
    }
    if track < 0 {
        return None;
    }
    for (map, mus) in PAIRS {
        let Some(name) = mus.split(',').nth(track as usize) else { continue };
        let target = if let Some(suffix) = map.strip_prefix('*') {
            if map_name.len() < suffix.len() || !map_name.ends_with(suffix) {
                continue;
            }
            format!("{}{}", &map_name[..map_name.len() - suffix.len()], &name[1..])
        } else {
            if map_name != map {
                continue;
            }
            name.to_string()
        };
        if let Some(p) = ctx.sibling_named(&target) {
            return Some(p);
        }
    }
    if let Some(plus) = map_name.find('+') {
        return ctx.sibling_named(&map_name[..plus]);
    }
    None
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 {
        return Ok(vec![]);
    }
    let ext = ctx.ext();
    let map_name = file_name(ctx.path()).to_string();
    if ext == "msb" || ext == "msx" {
        return msb(ctx);
    }
    // init_vgmstream_ea_map_mus (v0/v1), then init_vgmstream_ea_mpf_mus_schl (v3-v5)
    if let Some(v) = map_mus(ctx, &map_name)? {
        return Ok(v);
    }
    if ext == "mpf" {
        return mpf(ctx, 0, ctx.size(), &map_name, None);
    }
    Ok(vec![])
}

/// .MAP + .MUS (PathFinder v0/v1).
fn map_mus(ctx: &mut Ctx, map_name: &str) -> io::Result<Option<Vec<Found>>> {
    let mut r = Rd(&mut ctx.r);
    if r.size() < 0x0c || r.b(0, 4)? != b"PFDx" || r.u8(4)? > 1 {
        return Ok(None);
    }
    let num_sounds = r.u8(6)? as u64;
    let num_events = r.u8(7)? as u64;
    let num_sections = r.u8(0x0b)? as u64;
    let section = 0x0c + num_sounds * 0x1c + num_events * num_sections;
    if num_sounds == 0 || section + 4 * num_sounds > r.size() {
        return Ok(Some(vec![]));
    }
    let offsets: Vec<u64> = (0..num_sounds).map(|i| r.u32(section + 4 * i, true).map(|v| v as u64)).collect::<io::Result<_>>()?;
    let Some((mi, mut mus)) = open_pair(ctx, map_name, 0) else { return Ok(Some(vec![])) };
    let label = file_name(&ctx.entries[mi].path).to_string();
    let mut found = Vec::new();
    for o in offsets {
        if Rd(&mut mus).id(o)? != u32::from_be_bytes(*b"SCHl") {
            continue;
        }
        if let Some(s) = ea_schl::load_schl(&mut mus, mi, o)? {
            found.push(Found::new(s.track(ctx.entry, 0, "EA MAP/MUS"), ctx.size()).label(Some(label.clone())));
        }
    }
    Ok(Some(found))
}

/// .MSB/.MSX: a PFDx at 0x50 and the name of its .MUS.
fn msb(ctx: &mut Ctx) -> io::Result<Vec<Found>> {
    let mut r = Rd(&mut ctx.r);
    if r.size() < 0x60 || r.u32(0, false)? != 0 || r.u32(4, false)? != 0 {
        return Ok(vec![]);
    }
    let be = r.guess_be(0x08)?;
    if r.u32(0x08, be)? != 0x20 || r.u32(0x20, be)? != 0x05 {
        return Ok(vec![]);
    }
    let mpf_size = r.u32(0x24, be)? as u64;
    let name = super::label(&r.b(0x30, 0x20)?).unwrap_or_default();
    if name.is_empty() {
        return Ok(vec![]);
    }
    let map_name = file_name(ctx.path()).to_string();
    let size = ctx.size();
    mpf(ctx, 0x50, mpf_size.min(size - 0x50), &map_name, Some(&name))
}

/// Everything init_vgmstream_ea_mpf_mus_schl_main lists, for the PFDx at `base` (`size` bytes).
fn mpf(ctx: &mut Ctx, base: u64, size: u64, map_name: &str, mus_name: Option<&str>) -> io::Result<Vec<Found>> {
    if size < 0x40 || size > 0x100_0000 {
        return Ok(vec![]);
    }
    let buf = ctx.bytes(base, size as usize)?;
    let big_endian = match &buf[0..4] {
        b"PFDx" => true,
        b"xDFP" => false,
        _ => return Ok(vec![]),
    };
    let be = big_endian;
    // reads relative to the PFDx; past its end (clamped file) they give 0
    let rd = |_: &mut (), o: u64, n: u8| -> io::Result<u32> {
        if o + n as u64 > size {
            return Ok(0);
        }
        let o = o as usize;
        Ok(match n {
            1 => buf[o] as u32,
            2 => (if be { super::be16(&buf, o) } else { super::le16(&buf, o) }) as u32,
            _ => if be { super::be32(&buf, o) } else { super::le32(&buf, o) },
        })
    };
    let mut r = ();
    let version = rd(&mut r, 4, 1)?;
    let sub_version = rd(&mut r, 5, 1)?;
    if !(3..=5).contains(&version) || (version == 5 && sub_version > 3) {
        return Ok(vec![]);
    }
    let num_tracks = rd(&mut r, 0x0d, 1)?;
    let num_sections = rd(&mut r, 0x0e, 1)?;
    let num_events = rd(&mut r, 0x0f, 1)?;
    let num_routers = rd(&mut r, 0x10, 1)?;
    let num_vars = rd(&mut r, 0x11, 1)?;
    let num_nodes = rd(&mut r, 0x12, 2)?;
    let u32be_at = |_: &mut (), o: u64| -> io::Result<u32> { if o + 4 > size { Ok(0) } else { Ok(super::be32(&buf, o as usize)) } };

    let (tracks_table, samples_table, eof_offset, off_mult);
    let mut tracks_data = 0u32;
    match (version, sub_version) {
        (3, 1) | (3, 2) | (3, 4) => {
            let mut section = 0x24u32;
            let entry = rd(&mut r, (section + (num_nodes.wrapping_sub(1)) * 2) as u64, 2)?.wrapping_mul(4);
            let subentry = if sub_version == 4 {
                let v = u32be_at(&mut r, entry as u64 + 4)?;
                if big_endian { (v >> 19) & 0x1f } else { (v >> 16) & 0x1f }
            } else {
                rd(&mut r, entry as u64 + 0x0b, 1)?
            };
            section = entry.wrapping_add(0x0c + subentry * 4);
            section = section.wrapping_add((num_events * num_tracks * num_sections).next_multiple_of(4));
            section = section.wrapping_add(num_routers * 4 + num_vars * 4);
            tracks_table = rd(&mut r, section as u64, 4)?.wrapping_mul(4);
            if sub_version == 4 {
                samples_table = tracks_table.wrapping_add((num_tracks + 1) * 4);
                eof_offset = rd(&mut r, tracks_table as u64 + num_tracks as u64 * 4, 4)?.wrapping_mul(4);
            } else {
                samples_table = tracks_table.wrapping_add(num_tracks * 4);
                eof_offset = size as u32;
            }
            off_mult = 4u64;
        }
        (4, _) => {
            let section = 0x20u32;
            let entry = rd(&mut r, (section + num_nodes.wrapping_sub(1) * 2) as u64, 2)?.wrapping_mul(4);
            let v = u32be_at(&mut r, entry as u64 + 4)?;
            let sub = if big_endian { (v >> 15) & 0x0f } else { (v >> 20) & 0x0f };
            let section = entry.wrapping_add(0x10 + sub * 4);
            let entry = rd(&mut r, (section + num_events.wrapping_sub(1) * 2) as u64, 2)?.wrapping_mul(4);
            let v = u32be_at(&mut r, entry as u64 + 0x0c)?;
            let sub = if big_endian { (v >> 10) & 0x3f } else { (v >> 8) & 0x3f };
            let section = entry.wrapping_add(0x10 + sub * 0x10).wrapping_add(num_routers * 4);
            tracks_table = rd(&mut r, section as u64, 4)?.wrapping_mul(4);
            samples_table = tracks_table.wrapping_add((num_tracks + 1) * 4);
            eof_offset = rd(&mut r, tracks_table as u64 + num_tracks as u64 * 4, 4)?.wrapping_mul(4);
            off_mult = 0x80;
        }
        (5, _) => {
            tracks_table = rd(&mut r, 0x2c, 4)?;
            tracks_data = rd(&mut r, 0x30, 4)?;
            samples_table = rd(&mut r, 0x34, 4)?;
            eof_offset = rd(&mut r, 0x38, 4)?;
            off_mult = 0x80;
            if rd(&mut r, tracks_data as u64 + 4, 2)? == 0 && rd(&mut r, samples_table as u64, 4)? > 2 {
                return Ok(vec![]); // SNR/SNS version
            }
        }
        _ => return Ok(vec![]),
    }
    let _ = tracks_data;
    let total_streams = eof_offset.wrapping_sub(samples_table) / 8;
    if total_streams == 0 || total_streams > 0x10000 || samples_table as u64 + total_streams as u64 * 8 > size {
        return Ok(vec![]);
    }

    let mut found = Vec::new();
    for target in 1..=total_streams {
        // find the track this sample belongs to
        let mut track_start = total_streams;
        let mut track_end = total_streams;
        let mut i: i64 = num_tracks as i64 - 1;
        let mut entry_offset = 0u32;
        let mut is_ram = false;
        let mut checksum = 0u32;
        while i >= 0 {
            track_end = track_start;
            if version == 5 {
                entry_offset = rd(&mut r, tracks_table as u64 + i as u64 * 4, 4)?.wrapping_mul(4);
                track_start = rd(&mut r, entry_offset as u64, 4)?;
                if track_start == 0 && i != 0 {
                    i -= 1;
                    continue;
                }
                if track_start <= target - 1 {
                    let subbanks = rd(&mut r, entry_offset as u64 + 4, 2)?;
                    checksum = u32be_at(&mut r, entry_offset as u64 + 8)?;
                    is_ram = subbanks != 0;
                    break;
                }
            } else {
                track_start = rd(&mut r, tracks_table as u64 + i as u64 * 4, 4)?.wrapping_mul(4);
                track_start = track_start.wrapping_sub(samples_table) / 8;
                if track_start <= target - 1 {
                    break;
                }
            }
            i -= 1;
        }
        let pair = match mus_name {
            Some(n) => ctx.sibling_named(n),
            None => open_pair(ctx, map_name, i),
        };
        let Some((mi, mut mus)) = pair else { continue };
        let mut m = Rd(&mut mus);
        let bnk_magic = if big_endian { u32::from_be_bytes(*b"BNKb") } else { u32::from_be_bytes(*b"BNKl") };
        if version < 5 {
            is_ram = m.id(0)? == bnk_magic;
        }
        let sound_offset = rd(&mut r, samples_table as u64 + (target as u64 - 1) * 8, 4)?;
        let label = Some(file_name(&ctx.entries[mi].path).to_string());
        let sound: Option<Sound> = if is_ram {
            let bnk_offset = if version < 5 { 0 } else { 0x100 };
            let index = sound_offset & 0xffff;
            let bnk_index = sound_offset >> 16;
            let (mut mi, mut mus) = (mi, mus);
            if version == 5 && bnk_index != 0 {
                // the .MUS named like this one but ending in the bank number
                let name = file_name(&ctx.entries[mi].path).to_string();
                let (stem, ext) = super::split_ext(&name);
                if stem.len() <= 1 {
                    continue;
                }
                let other = format!("{}{}.{}", &stem[..stem.len() - 1], bnk_index, ext);
                match ctx.sibling_named(&other) {
                    Some((i2, r2)) => {
                        mi = i2;
                        mus = r2;
                    }
                    None => continue,
                }
            }
            let mut m = Rd(&mut mus);
            let total = m.u16(bnk_offset + 6, be)? as u32;
            if version == 5 {
                let ck = u32be_at(&mut r, entry_offset as u64 + 0x14 + 0x10 * bnk_index as u64)?;
                if ck != 0 && m.id(0)? != ck {
                    continue;
                }
            }
            if m.id(bnk_offset)? != bnk_magic {
                continue;
            }
            let segments = if target < track_end {
                let next = rd(&mut r, samples_table as u64 + target as u64 * 8, 4)?;
                if next >> 16 == bnk_index { (next & 0xffff).wrapping_sub(index) } else { total.wrapping_sub(index) }
            } else {
                total.wrapping_sub(index)
            };
            if segments == 0 || segments > 256 {
                continue;
            }
            let mut acc: Option<Sound> = None;
            let mut ok = true;
            for k in 0..segments {
                match ea_schl::load_bnk(&mut mus, mi, bnk_offset, (index + k) as usize, true)? {
                    Some(b) => {
                        acc = match acc {
                            None => Some(b.sound),
                            Some(a) => a.append(b.sound),
                        };
                        if acc.is_none() {
                            ok = false;
                            break;
                        }
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                continue;
            }
            acc.map(|mut s| {
                s.loops = None;
                s
            })
        } else {
            if version == 5 && checksum != 0 && m.id(0)? != checksum {
                continue;
            }
            let o = sound_offset as u64 * off_mult;
            if m.id(o)? != u32::from_be_bytes(*b"SCHl") {
                continue;
            }
            ea_schl::load_schl(&mut mus, mi, o)?
        };
        if let Some(s) = sound {
            found.push(Found::new(s.track(ctx.entry, base, "EA MPF/MUS"), base + size).label(label));
        }
    }
    Ok(found)
}

/// SCHl streams in a .MUS that a map file lists are found through it.
pub fn claims(ctx: &mut Ctx) -> io::Result<bool> {
    let name = file_name(ctx.path()).to_string();
    let is_map = |ctx: &Ctx, i: usize| -> bool {
        let e = &ctx.entries[i];
        ctx.game.reader(e).and_then(|mut r| r.bytes(0, 4)).map(|b| &b == b"PFDx" || &b == b"xDFP").unwrap_or(false)
    };
    if ctx.ext() == "mus" {
        for ext in ["mpf", "map", "lin"] {
            if let Some((i, _)) = ctx.sibling(ext) {
                if is_map(ctx, i) {
                    return Ok(true);
                }
            }
        }
    }
    // named pairs (MUS_STR.MUS next to MUS_CTRL.MPF, x_main.mus next to x.mpf, ...)
    for (map, mus) in PAIRS {
        for (k, m) in mus.split(',').enumerate() {
            let _ = k;
            let map_file = if let (Some(ms), Some(mp)) = (m.strip_prefix('*'), map.strip_prefix('*')) {
                match name.strip_suffix(ms) {
                    Some(prefix) => format!("{prefix}{mp}"),
                    None => continue,
                }
            } else if name == m {
                map.to_string()
            } else {
                continue;
            };
            if let Some((i, _)) = ctx.sibling_named(&map_file) {
                if is_map(ctx, i) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}
