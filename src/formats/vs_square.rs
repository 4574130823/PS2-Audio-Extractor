//! VS - VagStream from Square Sounds Co. games [Final Fantasy X (PS2) voices, Unlimited Saga
//! (PS2) voices, All Star Pro-Wrestling 2/3 (PS2) music] (vgmstream meta/vs_square.c,
//! layout/blocked_vs_square.c): PS-ADPCM in 0x800 blocks per channel, each starting with a
//! 0x20 "VS" header.

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "VS (Square)",
    magics: &[b"VS\0\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

const BLOCK: u64 = 0x800;
const HEADER: u64 = 0x20;

/// vgmstream's `spu2_pitch_to_sample_rate_rounded`.
fn pitch_to_rate_rounded(pitch: u32) -> u64 {
    let val = 48000 * pitch as u64 / 4096;
    let r = val % 10;
    if r < 5 { val - r } else { val + (10 - r) }
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    // 04: flags, 08: block number, 0c: blocks left, 10: pitch, 14: volume, 18/1c: null
    if &h[0..4] != b"VS\0\0" || le32(&h, 0x18) != 0 || le32(&h, 0x1c) != 0 {
        return Ok(vec![]);
    }
    let flags = le32(&h, 0x04);
    let pitch = le32(&h, 0x10);
    // Flags: stereo; some Front Mission 4 voices also have 0x100.
    if flags & !0x101 != 0 || pitch > 0x4000 || le32(&h, 0x14) > 0x100 {
        return Ok(vec![]);
    }
    let rate = pitch_to_rate_rounded(pitch);
    if !sane_rate(rate as u32) {
        return Ok(vec![]);
    }
    let channels: u16 = if flags & 1 != 0 { 2 } else { 1 };
    let step = BLOCK * channels as u64;
    let size = ctx.size();
    if off + step > size || !psx::plausible(&ctx.bytes(off + HEADER, 0x100)?) {
        return Ok(vec![]);
    }
    // vgmstream counts blocks to the end of the .vs file. Inside something bigger, follow
    // the blocks while their headers continue this stream (blocks left counting down).
    let blocks = if off == 0 && ctx.ext() == "vs" {
        size.div_ceil(step)
    } else {
        let mut n = 1u64;
        let mut left = le32(&h, 0x0c);
        loop {
            let b = off + n * step;
            if left == 0 || b + step > size {
                break;
            }
            let nh = ctx.bytes(b, 0x20)?;
            if &nh[0..4] != b"VS\0\0" || le32(&nh, 0x04) != flags || le32(&nh, 0x0c) != left - 1 {
                break;
            }
            left -= 1;
            n += 1;
        }
        n
    };
    let pieces: Vec<(u64, u64)> =
        (0..blocks).flat_map(|k| (0..channels as u64).map(move |c| (off + k * step + HEADER + BLOCK * c, BLOCK - HEADER))).collect();
    let samples = blocks * psx::bytes_to_samples(BLOCK - HEADER, 1);
    let data = Data::blocks(ctx.entry, pieces);
    let t = Track::new(ctx.entry, off, "VS", channels, rate as u32, samples, data, Codec::Psx(psx::Params::interleaved(BLOCK - HEADER)));
    Ok(vec![Found::new(t, (off + blocks * step).min(size))])
}
