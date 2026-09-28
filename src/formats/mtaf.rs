//! MTAF: Konami's stream format in Metal Gear Solid 3: Snake Eater / Subsistence (vgmstream
//! meta/mtaf.c): "MTAF" + "HEAD" + track info, "DATA" at 0x7f8, audio at 0x800.

use std::io;

use super::vag::vgm_loop;
use super::{Ctx, Found, Parser, label, le32};
use crate::codecs::{Codec, mtaf};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "MTAF",
    magics: &[b"MTAF"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x800)?;
    if &h[0..4] != b"MTAF" || &h[0x40..0x44] != b"HEAD" || le32(&h, 0x44) != 0xB0 || &h[0x7f8..0x7fc] != b"DATA" {
        return Ok(vec![]);
    }
    let loop_start = le32(&h, 0x58) as i32;
    let loop_end = le32(&h, 0x5c) as i32;
    let block = le32(&h, 0x60) as i32;
    let channels = block / 0x110 * 2;
    let looped = le32(&h, 0x70) & 1 != 0;
    if channels <= 0 || channels > 32 || block % 0x110 != 0 || loop_end <= 0 {
        return Ok(vec![]);
    }
    let samples = loop_end as u64;
    let data_off = off + 0x800;
    let size = samples.div_ceil(mtaf::FRAME_SAMPLES) * mtaf::FRAME * (channels as u64 / 2);
    if data_off >= ctx.size() {
        return Ok(vec![]);
    }
    // the file name, xor'd
    let name = if h[0x20] != 0 { label(&h[0x20..0x40].iter().map(|b| b ^ 0xff).collect::<Vec<u8>>()) } else { None };
    let mut t = Track::new(ctx.entry, off, "MTAF", channels as u16, 48000, samples, Data::at(ctx.entry, data_off, size), Codec::Mtaf(mtaf::Params {}));
    if looped {
        t = vgm_loop(t, loop_start as i64, loop_end as i64);
    }
    Ok(vec![Found::new(t, (data_off + size).min(ctx.size())).label(name)])
}
