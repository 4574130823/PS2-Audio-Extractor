//! VGS - from Princess Soft games [Gin no Eclipse (PS2), Metal Wolf REV (PS2)] (vgmstream
//! meta/vgs_ps.c): a stereo VAG clone, big-endian header, 0x20000 or 0x8000 interleave.

use std::io;

use super::{Ctx, Found, Parser, be32, label, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "VGS (PS)",
    magics: &[b"VGS\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    const START: u64 = 0x30;
    let h = ctx.bytes(off, 0x30)?;
    if &h[0..4] != b"VGS\0" {
        return Ok(vec![]);
    }
    let channel_size = be32(&h, 0x0c) as u64;
    let rate = be32(&h, 0x10);
    if !sane_rate(rate) || channel_size == 0 {
        return Ok(vec![]);
    }
    // vgmstream takes the data as the rest of the file. Inside something bigger, the file
    // ends after both channels' data.
    let file_end = if off == 0 { ctx.size() } else { (off + START + channel_size.next_multiple_of(0x10) * 2).min(ctx.size()) };
    if off + START >= file_end {
        return Ok(vec![]);
    }
    let data_size = file_end - off - START;
    // The second channel starts with a null frame: that gives the interleave.
    let null_at = |ctx: &mut Ctx, at: u64| -> io::Result<bool> { Ok(at + 4 <= file_end && ctx.u32be(at)? == 0) };
    let interleave = if null_at(ctx, off + START + 0x20000)? {
        0x20000u64
    } else if null_at(ctx, off + START + 0x8000)? {
        0x8000 // Ishikura Noboru no Igo Kouza: Chuukyuuhen (PS2)
    } else {
        return Ok(vec![]);
    };
    let start = off + START;
    let samples = psx::bytes_to_samples(channel_size, 1);
    if samples == 0 || !psx::plausible(&ctx.bytes(start, 0x100)?) || !psx::plausible(&ctx.bytes(start + interleave, 0x100)?) {
        return Ok(vec![]);
    }
    // The last block is shorter, so the data is exactly the rest of the file.
    let data = Data::at(ctx.entry, start, data_size);
    let t = Track::new(ctx.entry, off, "VGS", 2, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, file_end).label(label(&h[0x20..0x30]))])
}
