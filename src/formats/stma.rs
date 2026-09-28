//! STM - from Angel Studios/Rockstar San Diego games [Red Dead Revolver (PS2), Spy Hunter 2
//! (PS2/Xbox)] (vgmstream meta/stma.c): "STMA" (little endian: DVI IMA or PCM) or "AMTS"
//! (big endian, GameCube: DSP or PCM), data at 0x800.

use std::io;

use super::{Ctx, Found, Parser, be16, be32, le32, sane_rate};
use crate::codecs::{Codec, ima, pcm};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "STM",
    magics: &[b"STMA", b"AMTS"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x40)?;
    let big = match &h[0..4] {
        b"STMA" => false,
        b"AMTS" => true,
        _ => return Ok(vec![]),
    };
    let r32 = |at: usize| if big { be32(&h, at) } else { le32(&h, at) };
    let interleave = r32(0x08);
    let rate = r32(0x0c);
    let bps = r32(0x10);
    let channels = r32(0x14);
    let data_size = r32(0x18) as u64;
    let loop_end_off = r32(0x1c) as u64;
    if !sane_rate(rate) || !(1..=8).contains(&channels) || data_size == 0 || !(bps == 4 || bps == 16) {
        return Ok(vec![]);
    }
    let data_off = off + 0x800;
    if data_off + data_size > ctx.size() {
        return Ok(vec![]);
    }
    // vgmstream wants the data to fill the file exactly; standalone files must.
    if off == 0 && matches!(ctx.ext().as_str(), "stm" | "lstm") && data_off + data_size != ctx.size() {
        return Ok(vec![]);
    }
    let channels = channels as u16;
    let (mut loop_flag, mut loop_start) = (false, 0u64);
    if big {
        loop_flag = be16(&h, 0x2c) != 0;
    } else if le32(&h, 0x20) == 1 {
        loop_flag = true;
        loop_start = le32(&h, 0x24) as u64;
    }
    let loop_bytes = loop_end_off.saturating_sub(0x800);
    // Whole interleave rows, as vgmstream reads them (no shorter last block).
    let row = if bps == 4 { if interleave == 0xc000 { 0x80 } else { 0x40 } } else { 2 } * channels as u64;
    let data = Data::at(ctx.entry, data_off, data_size.div_ceil(row) * row);
    let t = match (bps, big) {
        (4, false) => {
            let il = if interleave == 0xc000 { 0x80 } else { 0x40 };
            let samples = ima::bytes_to_samples(data_size, channels);
            let t = Track::new(ctx.entry, off, "STM", channels, rate, samples, data, Codec::Ima(ima::Params::new(ima::Kind::Dvi, il)));
            if loop_flag { vgm_loop(t, loop_start as i32 as i64, ima::bytes_to_samples(loop_bytes, channels) as i64) } else { t }
        }
        (4, true) => {
            // GameCube DSP (its own header from 0x20).
            let samples = be32(&h, 0x20) as u64;
            let mut t = Track::new(ctx.entry, off, "STM", channels, rate, samples, data, Codec::None);
            t.note = Some("GameCube DSP ADPCM audio isn't supported".into());
            t
        }
        _ => {
            let params = if big { pcm::Params::be16(2) } else { pcm::Params::le16(2) };
            let samples = pcm::bytes_to_samples(data_size, channels, 16);
            let t = Track::new(ctx.entry, off, "STM", channels, rate, samples, data, Codec::Pcm(params));
            if loop_flag { vgm_loop(t, loop_start as i32 as i64, pcm::bytes_to_samples(loop_bytes, channels, 16) as i64) } else { t }
        }
    };
    if t.samples == 0 {
        return Ok(vec![]);
    }
    Ok(vec![Found::new(t, data_off + data_size)])
}

/// Loop points as vgmstream keeps them: dropped unless 0 <= start < end <= samples.
fn vgm_loop(t: Track, start: i64, end: i64) -> Track {
    if start >= 0 && start < end && end as u64 <= t.samples { t.looped(start as u64, end as u64) } else { t }
}
