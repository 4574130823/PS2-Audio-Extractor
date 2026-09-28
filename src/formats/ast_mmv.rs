//! AST - from Marvelous(?) games [Katekyou Hitman Reborn! Dream Hyper Battle! (PS2),
//! Binchou-tan: Shiawasegoyomi (PS2)] (vgmstream meta/ast_mmv.c). Shares the "AST\0" id
//! with ast_mv; this one stores the file's size.

use std::io;

use super::{Ctx, Found, Parser, label, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_start_ok, rows};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "AST (Marvelous)",
    magics: &[b"AST\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x40)?;
    if &h[0..4] != b"AST\0" {
        return Ok(vec![]);
    }
    let file_size = le32(&h, 0x04) as u64;
    let rate = le32(&h, 0x08);
    let channels = le32(&h, 0x0c);
    let interleave = le32(&h, 0x10) as u64;
    // 0x14: blocks, 0x18: ?, 0x1c: f32 duration
    let end = off + file_size;
    let standalone = off == 0 && ctx.ext() == "ast";
    if file_size <= 0x100 + 0x10 || end > ctx.size() || (standalone && end != ctx.size()) {
        return Ok(vec![]);
    }
    if !(1..=8).contains(&channels) || !sane_rate(rate) {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    if channels > 1 && (interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000) {
        return Ok(vec![]);
    }
    let start = off + 0x100;
    if !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let body = file_size - 0x100;
    let samples = psx::bytes_to_samples(body, channels);
    let data = Data::at(ctx.entry, start, rows(body.div_ceil(channels as u64), interleave, channels));
    let t = Track::new(ctx.entry, off, "AST", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, end).label(label(&h[0x20..0x40]))])
}
