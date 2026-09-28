//! SVAG - from Konami Tokyo games [OZ (PS2), Neo Contra (PS2), Silent Hill 2 (PS2)]
//! (vgmstream meta/svag_kcet.c): interleaved PS-ADPCM after a 0x800 header.

use std::io;

use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "SVAG (KCET)",
    magics: &[b"Svag"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    if &h[0..4] != b"Svag" {
        return Ok(vec![]);
    }
    let channels = le16(&h, 0x0c);
    let data_size = le32(&h, 0x04) as u64;
    let rate = le32(&h, 0x08);
    let interleave = le32(&h, 0x10) as u64;
    if !(1..=8).contains(&channels) || !sane_rate(rate) {
        return Ok(vec![]);
    }
    if channels > 1 && (interleave == 0 || interleave % 0x10 != 0 || interleave > 0x10000) {
        return Ok(vec![]);
    }
    // Stereo files repeat the header at 0x400 [Silent Hill 2], or have more padding there
    // (a "KCE-Tokyo Design..." phrase) [Silent Scope 2].
    if channels > 1 && !ctx.is(off + 0x400, b"Svag")? && !ctx.is(off + 0x400, b"Desi")? {
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
    let mut t = Track::new(ctx.entry, off, "SVAG", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    if le32(&h, 0x14) == 1 {
        let ls = (le32(&h, 0x18) as u64 * channels as u64) / channels as u64 / 16 * 28;
        if ls < samples {
            t = t.looped(ls, samples);
        }
    }
    Ok(vec![Found::new(t, start + data_size)])
}
