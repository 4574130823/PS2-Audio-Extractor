//! P2BT/MOVE/VISA - from Konami/KCE Studio games [Pop'n Music 7/8/Best (PS2), AirForce Delta
//! Strike (PS2)] (vgmstream meta/p2bt_move_visa.c): interleaved PS-ADPCM after a 0x800
//! header (the same header under three ids).

use std::io;

use super::{Ctx, Found, Parser, label, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "P2BT/MOVE/VISA",
    magics: &[b"P2BT", b"MOVE", b"VISA"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x38)?;
    if !matches!(&h[0..4], b"P2BT" | b"MOVE" | b"VISA") {
        return Ok(vec![]);
    }
    let rate = le32(&h, 0x08);
    let loop_start = le32(&h, 0x0c) as u64;
    let data_size = le32(&h, 0x10) as u64; // without padding
    let interleave = le32(&h, 0x14) as u64; // usually 0x10, sometimes 0x400
    let channels = le32(&h, 0x20);
    if !(1..=8).contains(&channels) || !sane_rate(rate) {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    if channels > 1 && (interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000) {
        return Ok(vec![]);
    }
    let start = off + 0x800;
    // The last block is shorter: the data is exactly `data_size` long.
    if data_size == 0 || start + data_size > ctx.size() || !psx::plausible(&ctx.bytes(start, 0x100.min(data_size as usize))?) {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(data_size, channels);
    if samples == 0 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, start, data_size);
    let mut t = Track::new(ctx.entry, off, "P2BT/MOVE/VISA", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    if loop_start != 0 {
        let ls = psx::bytes_to_samples(loop_start, channels);
        if ls < samples {
            t = t.looped(ls, samples);
        }
    }
    Ok(vec![Found::new(t, start + data_size).label(label(&h[0x28..0x38]))])
}
