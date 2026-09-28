//! MPC3 - from Paradigm games [Spy Hunter (PS2), MX Rider (PS2), Terminator 3 (PS2)]
//! (vgmstream meta/mpc3.c).

use std::io;

use super::{Ctx, Found, Parser, be32, le32, sane_rate};
use crate::codecs::{Codec, mpc3};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "MPC3",
    magics: &[b"MPC3"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x1c)?;
    if &h[0..4] != b"MPC3" || be32(&h, 0x04) != 0x0001_1400 {
        return Ok(vec![]);
    }
    let channels = le32(&h, 0x08);
    let rate = le32(&h, 0x0c);
    let samples = le32(&h, 0x10);
    let block = le32(&h, 0x14) as u64;
    let size = le32(&h, 0x18) as u64;
    let start = off + 0x1c;
    if !(1..=2).contains(&channels) || !sane_rate(rate) || samples == 0 || samples > 0x0fff_ffff || size == 0 {
        return Ok(vec![]);
    }
    // vgmstream wants the data to end the file.
    if start + size > ctx.size() || (off == 0 && ctx.ext() == "mc3" && start + size != ctx.size()) {
        return Ok(vec![]);
    }
    let block_size = block * 4 * channels as u64 + 4;
    if block_size < 0x0c {
        return Ok(vec![]);
    }
    let samples = samples as u64 * 10; // counted in sub-blocks of 10 samples
    let data = Data::at(ctx.entry, start, size);
    let t = Track::new(ctx.entry, off, "MPC3", channels as u16, rate, samples, data, Codec::Mpc3(mpc3::Params { block_size }));
    Ok(vec![Found::new(t, start + size)])
}
