//! VSF - from Square Enix PS2 games between 2004-2006 [Musashi: Samurai Legend (PS2), Front
//! Mission 5 (PS2)] (vgmstream meta/vsf.c): mono or stereo PS-ADPCM (0x400 interleave), rate
//! as an SPU2 pitch.

use std::io;

use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "VSF",
    magics: &[b"VSF\0"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

/// vgmstream's `spu2_pitch_to_sample_rate_rounded`.
fn pitch_to_rate_rounded(pitch: i32) -> i64 {
    let val = 48000i64 * pitch as i64 / 4096;
    let r = val % 10;
    if r < 5 { val - r } else { val + (10 - r) }
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x40)?;
    if &h[0..4] != b"VSF\0" || le32(&h, 0x0c) != 0x0001_0000 {
        return Ok(vec![]);
    }
    let channel_size = le32(&h, 0x10) as u64 * 0x10;
    let loop_start = le32(&h, 0x18) as u64 * 0x10;
    let flags = le32(&h, 0x1c);
    let rate = pitch_to_rate_rounded(le32(&h, 0x20) as i32);
    // Known flags: stereo, loop, 0x10 (common, unknown), short header. The rest is 0xFF.
    if flags & !0x113 != 0 || h[0x2c..0x40].iter().any(|&b| b != 0xff) || !(0..=96000).contains(&rate) || !sane_rate(rate as u32) {
        return Ok(vec![]);
    }
    let channels: u16 = if flags & 1 != 0 { 2 } else { 1 };
    let start = off + if flags & 0x100 != 0 { 0x80 } else { 0x800 };
    let size = if channels > 1 { channel_size.next_multiple_of(0x400) * 2 } else { channel_size };
    if channel_size == 0 || start + size > ctx.size() || !psx::plausible(&ctx.bytes(start, 0x100.min(size as usize))?) {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(channel_size, 1);
    let data = Data::at(ctx.entry, start, size);
    let mut t = Track::new(ctx.entry, off, "VSF", channels, rate as u32, samples, data, Codec::Psx(psx::Params::interleaved(0x400)));
    if flags & 2 != 0 {
        let ls = psx::bytes_to_samples(loop_start, 1);
        if ls < samples {
            t = t.looped(ls, samples);
        }
    }
    Ok(vec![Found::new(t, start + size)])
}
