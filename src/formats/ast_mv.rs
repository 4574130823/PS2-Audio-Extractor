//! AST - from MicroVision lib games [P.T.O. IV (PS2), Naval Ops: Warship Gunner (PS2)]
//! (vgmstream meta/ast_mv.c). Shares the "AST\0" id with ast_mmv (tried first).

use std::io;

use super::{Ctx, Found, Parser, be32, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_start_ok, rows};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "AST (MicroVision)",
    magics: &[b"AST\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x14)?;
    if &h[0..4] != b"AST\0" {
        return Ok(vec![]);
    }
    let rate = le32(&h, 0x04);
    let interleave = le32(&h, 0x08) as u64;
    let data_size = le32(&h, 0x0c) as u64; // file size, header included
    let check = be32(&h, 0x10);
    // 0x20002000: Naval Ops (garbage up to the data), 0: P.T.O. IV. (.ikm in Zwei is a
    // variation with loops.)
    if check != 0x2000_2000 && check != 0 {
        return Ok(vec![]);
    }
    let (channels, start) = (2u16, off + 0x800);
    if !sane_rate(rate) || interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000 || data_size <= 0x800 + 0x20 {
        return Ok(vec![]);
    }
    let body = data_size - 0x800;
    if start + body > ctx.size() + interleave * 2 || !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(body, channels);
    let data = Data::at(ctx.entry, start, rows(body.div_ceil(2), interleave, channels));
    let t = Track::new(ctx.entry, off, "AST", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, (start + body).min(ctx.size()))])
}
