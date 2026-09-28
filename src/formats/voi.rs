//! .VOI - from Raw Danger (PS2) (vgmstream meta/voi.c): 16-bit PCM after a 0x800 header of
//! channels, size and a rate/interleave mode. Known only by extension.

use std::io;

use super::{Ctx, Found, Parser, le32};
use crate::codecs::{Codec, pcm};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "VOI",
    magics: &[],
    magic_at: 0,
    exts: &["voi"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 || ctx.size() <= 0x800 {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, 0x0c)?;
    let channels = le32(&h, 0x00);
    if channels != 1 && channels != 2 {
        return Ok(vec![]);
    }
    let size = ctx.size();
    if le32(&h, 0x04) as u64 * 2 + 0x800 != size {
        return Ok(vec![]);
    }
    let (rate, interleave) = match le32(&h, 0x08) {
        0 => (48000, 0x200u64),
        1 => (24000, 0x100),
        _ => return Ok(vec![]),
    };
    let channels = channels as u16;
    let body = size - 0x800;
    let samples = pcm::bytes_to_samples(body, channels, 16);
    if samples == 0 {
        return Ok(vec![]);
    }
    let body = if channels > 1 { body.next_multiple_of(interleave * channels as u64) } else { body };
    let data = Data::at(ctx.entry, 0x800, body);
    let t = Track::new(ctx.entry, 0, "VOI", channels, rate, samples, data, Codec::Pcm(pcm::Params::le16(interleave)));
    Ok(vec![Found::new(t, size)])
}
