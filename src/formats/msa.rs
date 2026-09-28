//! MSA - from Success games [Psyvariar -Complete Edition- (PS2), Konohana Pack: 3tsu no
//! Jikenbo (PS2)] (vgmstream meta/msa.c): stereo PS-ADPCM after a 0x14 header. Known only by
//! extension (the header starts with zeros).

use std::io;

use super::{Ctx, Found, Parser, be32, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "MSA",
    magics: &[],
    magic_at: 0,
    exts: &["msa"],
    locate: None,
    parse,
};

/// vgmstream's `ps_check_format`: every frame in the range has a valid predictor and flag.
fn ps_check_format(ctx: &mut Ctx, off: u64, max: u64) -> io::Result<bool> {
    let end = (off + max).min(ctx.size());
    if end <= off {
        return Ok(true);
    }
    let buf = ctx.bytes(off, ((end + 1).min(ctx.size()) - off) as usize)?;
    let mut i = 0;
    while (off + i as u64) < end {
        let flags = buf.get(i + 1).copied().unwrap_or(0xff); // past the end reads as -1
        if buf[i] >> 4 > 5 || flags > 7 {
            return Ok(false);
        }
        i += 16;
    }
    Ok(true)
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    const START: u64 = 0x14;
    if off != 0 || ctx.size() <= START {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, 0x14)?;
    if be32(&h, 0x00) != 0 || be32(&h, 0x08) != 0 || !ps_check_format(ctx, START, 0x100)? {
        return Ok(vec![]);
    }
    let file_size = ctx.size();
    let data_size = le32(&h, 0x04) as u64; // wrong, see below
    let channel_size = le32(&h, 0x0c);
    let rate = match le32(&h, 0x10) {
        0 => 44100, // Psyvariar's AME.MSA
        r => r,
    };
    if !sane_rate(rate) {
        return Ok(vec![]);
    }
    let interleave: u64 = if channel_size != 0 { 0x6000 } else { 0x4000 }; // Konohana Pack / Psyvariar
    let row = interleave * 2;
    let mut samples = psx::bytes_to_samples(data_size, 2);
    // MSAs are strangely truncated: data after the last whole block is silence or garbage.
    if data_size > file_size {
        let usable = file_size - START;
        samples = psx::bytes_to_samples(usable - usable % row, 2);
    }
    if samples == 0 {
        return Ok(vec![]);
    }
    let frames = samples.div_ceil(28);
    let size = (frames * 16).next_multiple_of(interleave) * 2;
    let data = Data::at(ctx.entry, START, size);
    let t = Track::new(ctx.entry, 0, "MSA", 2, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, file_size)])
}
