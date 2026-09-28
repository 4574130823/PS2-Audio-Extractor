//! XA2 - from Acclaim games [RC Revenge Pro (PS2), XGIII: Extreme G Racing (PS2)] (vgmstream
//! meta/xa2_acclaim.c): interleaved PS-ADPCM at 44100 Hz after a 0x800 header of channels,
//! interleave and per-channel sizes. Known only by extension.

use std::io;

use super::{Ctx, Found, Parser, be32, le32};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "XA2",
    magics: &[],
    magic_at: 0,
    exts: &["xa2"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 || ctx.size() <= 0x800 {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, 0x800)?;
    let channels = le32(&h, 0x00);
    if channels == 0 || channels > 0x10 {
        return Ok(vec![]);
    }
    let (interleave, sizes) = if le32(&h, 0x04) > 0x1000 {
        // RC Revenge Pro: no interleave field
        (if channels > 2 { 0x400u64 } else { 0x1000 }, 0x04usize)
    } else {
        (le32(&h, 0x04) as u64, 0x08)
    };
    // One size per channel, then nothing.
    if (0..channels as usize).any(|i| be32(&h, sizes + 4 * i) == 0) || be32(&h, sizes + 4 * channels as usize) != 0 {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    if channels > 1 && (interleave == 0 || interleave % 0x10 != 0) {
        return Ok(vec![]);
    }
    let body = ctx.size() - 0x800;
    let samples = psx::bytes_to_samples(body, channels);
    if samples == 0 || !psx::plausible(&ctx.bytes(0x800, 0x100.min(body as usize))?) {
        return Ok(vec![]);
    }
    let size = if channels > 1 { body.next_multiple_of(interleave * channels as u64) } else { body };
    let data = Data::at(ctx.entry, 0x800, size);
    let t = Track::new(ctx.entry, 0, "XA2", channels, 44100, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, ctx.size())])
}
