//! SMPL - from Homura (PS2) (vgmstream meta/smpl.c). Mono VAG clones: .v0 is the left
//! channel, .v1 the right.
//!
//! vgmstream plays a .v0 with its .v1 next to it as one stereo stream ("dual stereo");
//! here each file is its own mono track.

use std::io;

use super::{Ctx, Found, Parser, be32, label, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_start_ok, vgm_loop};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "SMPL",
    magics: &[b"SMPL"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x40)?;
    if &h[0..4] != b"SMPL" {
        return Ok(vec![]);
    }
    let size = be32(&h, 0x0c) as u64;
    let rate = be32(&h, 0x10);
    let loop_start = le32(&h, 0x30) as i32 as i64; // .v1 has none
    let start = off + 0x40;
    if !sane_rate(rate) || size <= 0x10 {
        return Ok(vec![]);
    }
    let size = size - 0x10;
    if start + size > ctx.size() + 0x800 || !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(size, 1);
    let data = Data::at(ctx.entry, start, size);
    let mut t = Track::new(ctx.entry, off, "SMPL", 1, rate, samples, data, Codec::Psx(psx::Params::default()));
    t.dual_ok = off == 0; // .v0 + .v1 play as stereo (see scan::pair_dual)
    let t = vgm_loop(t, loop_start != 0, loop_start, samples as i64);
    Ok(vec![Found::new(t, (start + size).min(ctx.size())).label(label(&h[0x20..0x30]))])
}
