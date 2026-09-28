//! RKV - from Legacy of Kain: Blood Omen 2 (PS2) (vgmstream meta/rkv.c). No signature:
//! found by extension. (The GameCube version's .rkv uses NGC DSP, which this app can't
//! decode, and isn't PS2 audio: skipped.)

use std::io;

use super::ps2p::vgm_loop;
use super::{Ctx, Found, Parser, be32, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "RKV",
    magics: &[],
    magic_at: 0,
    exts: &["rkv"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let size = ctx.size();
    if off != 0 || size <= 0x800 {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, 0x28)?;
    if be32(&h, 0x24) != 0 {
        return Ok(vec![]); // GameCube
    }
    let ho = if le32(&h, 0) == 0 { 4 } else { 0 };
    let channels: u64 = match le32(&h, ho + 0x0c) {
        0 => 1,
        1 => 2,
        _ => return Ok(vec![]),
    };
    let loop_flag = le32(&h, ho + 4) != 0xffff_ffff;
    let rate = le32(&h, ho) as i32;
    let data_size = size - 0x800;
    let samples = psx::bytes_to_samples(data_size, channels as u16);
    if rate <= 0 || !sane_rate(rate as u32) || samples == 0 || !psx::plausible(&ctx.bytes(0x800, 0x100.min(data_size as usize))?) {
        return Ok(vec![]);
    }
    // (the short last row is split evenly between channels, as vgmstream's
    // interleave_last_block_size does)
    let data = Data::at(ctx.entry, 0x800, data_size);
    let mut t = Track::new(ctx.entry, 0, "RKV", channels as u16, rate as u32, samples, data, Codec::Psx(psx::Params::interleaved(0x400)));
    if loop_flag {
        t = vgm_loop(t, le32(&h, ho + 4) as i32 as i64, le32(&h, ho + 8) as i32 as i64);
    }
    Ok(vec![Found::new(t, size)])
}
