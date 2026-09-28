//! .seb - Game Arts games [Grandia (PS1), Grandia II/III/X (PS2)] (vgmstream meta/seb.c).
//! No signature: found by extension only (.seb, and .gms for the unnamed files of the
//! .stz+.idx bigfiles; vgmstream also tries extensionless files, which isn't done here as
//! the header is too loose for that).

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_start_ok, rows, vgm_loop};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "SEB",
    magics: &[],
    magic_at: 0,
    exts: &["seb", "gms"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x24)?;
    let channels = le32(&h, 0x00) as i32;
    let rate = le32(&h, 0x04);
    let file_size = ctx.size();
    // 0x10/0x18: loop start/end offsets
    if !(1..=2).contains(&channels) || !sane_rate(rate) || le32(&h, 0x10) as u64 > file_size || le32(&h, 0x18) as u64 > file_size {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    let looped = le32(&h, 0x20) == 0;
    let samples = le32(&h, 0x1c) as i32;
    let loop_start = le32(&h, 0x14) as i32;
    let (start, interleave) = (off + 0x800, 0x800u64);
    if samples <= 0 || !psx_start_ok(ctx, start)? {
        return Ok(vec![]);
    }
    let samples = samples as u64;
    let per_channel = samples.div_ceil(28) * 16;
    let size = rows(per_channel, interleave, channels);
    if start + per_channel * channels as u64 > file_size + interleave * 2 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, start, size);
    let t = Track::new(ctx.entry, off, "SEB", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    let t = vgm_loop(t, looped, loop_start as i64, samples as i64);
    Ok(vec![Found::new(t, (start + size).min(file_size))])
}
