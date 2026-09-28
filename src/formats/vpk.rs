//! VPK - from SCE America second party devs [God of War (PS2), NBA 08 (PS3)] (vgmstream
//! meta/vpk.c).

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::formats::a2m::{psx_start_ok, rows, vgm_loop};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "VPK",
    magics: &[b" KPV"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x800)?;
    if &h[0..4] != b" KPV" {
        return Ok(vec![]);
    }
    // Sizes sometimes run a little into the padding (garbage / silent frames); kept as is.
    let chan_size = le32(&h, 0x04) as u64;
    let start = le32(&h, 0x08) as u64;
    let interleave = le32(&h, 0x0c) as u64 / 2; // even with more than 2 channels
    let rate = le32(&h, 0x10);
    let channels = le32(&h, 0x14);
    let loop_offset = le32(&h, 0x7fc) as u64; // per channel [Sly 2/3]
    if !(1..=8).contains(&channels) || !sane_rate(rate) || chan_size < 0x10 || start < 0x18 {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    if channels > 1 && (interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000) {
        return Ok(vec![]);
    }
    let data_off = off + start;
    let size = rows(chan_size, interleave, channels);
    if data_off + chan_size * channels as u64 > ctx.size() + 0x800 || !psx_start_ok(ctx, data_off)? {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(chan_size * channels as u64, channels);
    let data = Data::at(ctx.entry, data_off, size);
    let t = Track::new(ctx.entry, off, "VPK", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    let loop_start = psx::bytes_to_samples(loop_offset * channels as u64, channels) as i64;
    let t = vgm_loop(t, loop_offset != 0, loop_start, samples as i64);
    Ok(vec![Found::new(t, (data_off + size).min(ctx.size()))])
}
