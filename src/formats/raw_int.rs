//! Raw 16-bit PCM known only by its extension (vgmstream meta/raw_int.c): .int is stereo,
//! .wp2 four channels, 48000 Hz, 0x200 byte blocks per channel [PaRappa The Rapper 2 (PS2),
//! Amplitude (PS2), R-Type Final (PS2)]. .int files that look like PS-ADPCM are left alone.

use std::io;

use super::{Ctx, Found, Parser};
use crate::codecs::{Codec, pcm};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "INT",
    magics: &[],
    magic_at: 0,
    exts: &["int", "wp2"],
    locate: None,
    parse,
};

/// vgmstream's `ps_check_format`: every frame in the range has a valid predictor and flag.
fn ps_check_format(ctx: &mut Ctx, off: u64, max: u64) -> io::Result<bool> {
    let end = (off + max).min(ctx.size());
    if end <= off {
        return Ok(true);
    }
    // One byte more, for the flag of a frame starting at the last byte.
    let buf = ctx.bytes(off, ((end + 1).min(ctx.size()) - off) as usize)?;
    let mut i = 0;
    while (off + i as u64) < end {
        let predictor = buf[i] >> 4;
        let flags = buf.get(i + 1).copied().unwrap_or(0xff); // past the end reads as -1
        if predictor > 5 || flags > 7 {
            return Ok(false);
        }
        i += 16;
    }
    Ok(true)
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let ext = ctx.ext();
    if off != 0 || !(ext == "int" || ext == "wp2") {
        return Ok(vec![]);
    }
    let channels: u16 = if ext == "wp2" { 4 } else { 2 };
    if ps_check_format(ctx, 0, 0x100000)? {
        return Ok(vec![]);
    }
    let size = ctx.size();
    let samples = pcm::bytes_to_samples(size, channels, 16);
    if samples == 0 {
        return Ok(vec![]);
    }
    let row = 0x200 * channels as u64;
    let data = Data::at(ctx.entry, 0, size.next_multiple_of(row));
    let t = Track::new(ctx.entry, 0, "INT", channels, 48000, samples, data, Codec::Pcm(pcm::Params::le16(0x200)));
    Ok(vec![Found::new(t, size)])
}
