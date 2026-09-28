//! ILD - from Tose(?) games [Battle of Sunrise (PS2), Nightmare Before Christmas: Oogie's
//! Revenge (PS2)] (vgmstream meta/ild.c): interleaved PS-ADPCM, with a 0x20 header per
//! channel after the main one (the first gives interleave, rate and loop points).

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "ILD",
    magics: &[b"ILD\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x34)?;
    if &h[0..4] != b"ILD\0" {
        return Ok(vec![]);
    }
    let channels = le32(&h, 0x04);
    let start = le32(&h, 0x08) as u64;
    let data_size = le32(&h, 0x0c) as u64;
    // First channel header: null, header size (0x20), size, interleave, 1, rate, loops.
    let interleave = le32(&h, 0x14 + 0x0c) as u64;
    let rate = le32(&h, 0x14 + 0x14);
    let loop_start = le32(&h, 0x14 + 0x18) as u64;
    let loop_end = le32(&h, 0x14 + 0x1c) as u64;
    if !(1..=8).contains(&channels) || !sane_rate(rate) || le32(&h, 0x14) != 0 || le32(&h, 0x18) != 0x20 {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    if channels > 1 && (interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000) {
        return Ok(vec![]);
    }
    if start < 0x14 + 0x20 * channels as u64 {
        return Ok(vec![]);
    }
    let start = off + start;
    let size = if channels > 1 { data_size.next_multiple_of(interleave * channels as u64) } else { data_size };
    if data_size == 0 || start + size > ctx.size() || !psx::plausible(&ctx.bytes(start, 0x100.min(size as usize))?) {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(data_size, channels);
    if samples == 0 {
        return Ok(vec![]);
    }
    let data = Data::at(ctx.entry, start, size);
    let mut t = Track::new(ctx.entry, off, "ILD", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    if le32(&h, 0x2c) as i32 > 0 {
        let (ls, le) = (psx::bytes_to_samples(loop_start, 1), psx::bytes_to_samples(loop_end, 1));
        if ls < le && le <= samples {
            t = t.looped(ls, le);
        }
    }
    Ok(vec![Found::new(t, start + size)])
}
