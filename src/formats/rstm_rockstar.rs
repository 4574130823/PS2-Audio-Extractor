//! RSTM - from Rockstar games [Midnight Club 3, Bully - Canis Canim Edit (PS2)] (vgmstream
//! meta/rstm_rockstar.c). Bully's playlist banks get their names from the .LST next to them.

use std::collections::HashMap;
use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate, split_ext};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_start_ok, vgm_loop};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "RSTM",
    magics: &[b"RSTM"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x24)?;
    if &h[0..4] != b"RSTM" {
        return Ok(vec![]);
    }
    let rate = le32(&h, 0x08);
    let channels = le32(&h, 0x0c);
    // 0x10-0x18: padding
    let size = le32(&h, 0x18) as i32 as i64;
    let loop_start = le32(&h, 0x1c) as i32 as i64;
    let loop_end = le32(&h, 0x20) as i32 as i64;
    let start = off + 0x800;
    if !(1..=8).contains(&channels) || !sane_rate(rate) || size < 0x10 {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    let size = size as u64;
    if start + size > ctx.size() + 0x800 || !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let to_samples = |b: i64| if b < 0 { -1 } else { psx::bytes_to_samples(b as u64, channels) as i64 };
    let samples = psx::bytes_to_samples(size, channels);
    let data = Data::at(ctx.entry, start, size.div_ceil(0x10 * channels as u64) * 0x10 * channels as u64);
    let t = Track::new(ctx.entry, off, "RSTM", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(0x10)));
    let t = vgm_loop(t, loop_end != size as i64, to_samples(loop_start), to_samples(loop_end));
    if ctx.names.is_none() {
        ctx.names = Some(playlist_names(ctx).unwrap_or_default());
    }
    let label = ctx.names.as_ref().and_then(|n| n.get(&off)).cloned();
    Ok(vec![Found::new(t, (start + size).min(ctx.size())).label(label)])
}

/// Bully's AUDIO/PLAYLIST/*.BIN start with a "Hash" table of (name hash, offset, size),
/// and the names are listed in the .LST next to it (MUSIC1.BIN, a copy of MUSIC.BIN,
/// shares MUSIC.LST). The hash is Jenkins' one-at-a-time of the lowercase name.
fn playlist_names(ctx: &mut Ctx) -> io::Result<HashMap<u64, String>> {
    let mut out = HashMap::new();
    if !ctx.is(0, b"Hash")? {
        return Ok(out);
    }
    let count = ctx.u32le(4)? as u64;
    if count == 0 || count > 1 << 20 || 8 + count * 12 > ctx.size() {
        return Ok(out);
    }
    let (stem, _) = split_ext(ctx.path());
    let file = stem.rsplit('/').next().unwrap_or(stem);
    let base = file.trim_end_matches(|c: char| c.is_ascii_digit());
    let Some((_, mut r)) = ctx.sibling("lst").or_else(|| ctx.sibling_named(&format!("{base}.LST"))) else {
        return Ok(out);
    };
    if r.size > 16 << 20 {
        return Ok(out);
    }
    let list = r.bytes(0, r.size as usize)?;
    let by_hash: HashMap<u32, &str> = std::str::from_utf8(&list)
        .unwrap_or("")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| (joaat(l.to_ascii_lowercase().as_bytes()), l))
        .collect();
    let table = ctx.bytes(8, count as usize * 12)?;
    for e in table.chunks_exact(12) {
        if let Some(name) = by_hash.get(&le32(e, 0)) {
            let (name, _) = split_ext(name);
            out.insert(le32(e, 4) as u64, name.to_string());
        }
    }
    Ok(out)
}

/// Jenkins' one-at-a-time hash.
fn joaat(s: &[u8]) -> u32 {
    let mut h = 0u32;
    for &c in s {
        h = h.wrapping_add(c as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h.wrapping_add(h << 15)
}
