//! .acx: CRI's simple container of ADX (or Ogg) sounds [Baroque (SAT), Persona 3 (PS2),
//! THE iDOLM@STER: Live For You (X360)] (vgmstream meta/acx.c). The header has no
//! signature, so it's found by extension; the ADX inside are also found on their own
//! (same tracks) when an .acx is buried in an archive.

use std::io;

use super::{Ctx, Found, Parser, adx, be32, ogg_vorbis};

pub const PARSER: Parser = Parser {
    name: "ACX",
    magics: &[],
    magic_at: 0,
    exts: &["acx"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 8)?;
    if be32(&h, 0) != 0 {
        return Ok(vec![]);
    }
    let count = be32(&h, 4) as u64;
    if count == 0 || count > 256 {
        return Ok(vec![]);
    }
    let size = ctx.size() - off;
    let table = ctx.bytes(off + 8, count as usize * 8)?;
    let mut found = Vec::new();
    for i in 0..count as usize {
        let (so, ss) = (be32(&table, i * 8) as u64, be32(&table, i * 8 + 4) as u64);
        if so < 8 + count * 8 || so >= size || ss == 0 || so + ss > size {
            return Ok(vec![]);
        }
        let id = ctx.u32be(off + so)?;
        let f = if id == u32::from_be_bytes(*b"OggS") {
            ogg_vorbis::parse_at(ctx, off + so, ss)?
        } else if id & 0xFFFF_0000 == 0x8000_0000 {
            adx::parse_at(ctx, off + so, ss, 0)?
        } else {
            None
        };
        // vgmstream fails the subsongs it can't open
        if let Some(mut f) = f {
            f.track.format = "ACX";
            f.end = off + size;
            found.push(f);
        }
    }
    Ok(found)
}
