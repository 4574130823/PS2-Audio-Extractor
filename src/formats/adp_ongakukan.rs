//! Ongakukan RIFF with "ADP" extension [Train Simulator: Midousuji-sen (PS2), Mobile Train
//! Simulator (PSP)] (vgmstream meta/adp_ongakukan.c): a PCM WAV header whose data was
//! replaced by Ongakukan ADPCM (a quarter of the size). Known by the .adp extension (as
//! a signature, "RIFF" would be taken for a plain WAV), and the data runs to the end.

use std::io;

use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, ongakukan};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "ADP",
    magics: &[],
    magic_at: 0,
    exts: &["adp"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 || ctx.size() <= 0x2c {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, 0x2c)?;
    if &h[0..4] != b"RIFF" || &h[8..12] != b"WAVE" || &h[0x0c..0x10] != b"fmt " {
        return Ok(vec![]);
    }
    let start = 0x2cu64;
    let data_size = ctx.size() - start;
    let expected = (le32(&h, 0x04) as i64) - 0x24;
    let diff = expected - data_size as i64 * 4;
    if !(0..=14).contains(&diff) {
        return Ok(vec![]);
    }
    let fmt_size = le32(&h, 0x10) as usize;
    if !(0x10..=0x12).contains(&fmt_size) {
        return Ok(vec![]);
    }
    let rate = le32(&h, 0x18);
    if le16(&h, 0x14) != 1 || le16(&h, 0x16) != 1 || le16(&h, 0x22) != 16 || !sane_rate(rate) {
        return Ok(vec![]);
    }
    let fact = 0x14 + fmt_size;
    if !(0x24..=0x28).contains(&fact) || (&h[0x24..0x28] != b"data" && &h[fact..fact + 4] != b"fact") {
        return Ok(vec![]);
    }
    let samples = data_size * 2;
    let data = Data::at(ctx.entry, start, data_size);
    let t = Track::new(ctx.entry, 0, "ADP", 1, rate, samples, data, Codec::Ongakukan(ongakukan::Params {}));
    Ok(vec![Found::new(t, ctx.size())])
}
