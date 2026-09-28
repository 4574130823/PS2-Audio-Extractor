//! SFX0 - from Monster Games [NASCAR Heat 2002 (PS2/Xbox), NASCAR: Dirt to Daytona (PS2/GC),
//! Excite Truck (Wii), ExciteBots (Wii)] (vgmstream meta/sfx0_monster.c, both the current
//! and the early header). No signature: .sfx/.sf0 files whose sizes add up.

use std::io;

use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, ima, pcm, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "SFX0",
    magics: &[],
    magic_at: 0,
    exts: &["sfx", "sf0"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 || ctx.size() < 0x20 {
        return Ok(vec![]);
    }
    let h = ctx.bytes(0, 0x20)?;
    let data_size = le32(&h, 0x00) as u64;
    if data_size == 0 {
        return Ok(vec![]);
    }
    if let Some(f) = current(ctx, &h, data_size)? {
        return Ok(vec![f]);
    }
    if ctx.ext() == "sfx" && let Some(f) = early(ctx, &h, data_size)? {
        return Ok(vec![f]);
    }
    Ok(vec![])
}

fn current(ctx: &mut Ctx, h: &[u8], data_size: u64) -> io::Result<Option<Found>> {
    let head_size = le32(h, 0x04) as u64;
    if head_size == 0 || data_size + head_size != ctx.size() {
        return Ok(None);
    }
    let mut loop_flag = h[0x08] != 0;
    let extra_flag = h[0x09];
    let mut codec = le16(h, 0x0c);
    let channels = le16(h, 0x0e);
    let rate = le32(h, 0x10);
    let (config1, config2) = (le32(h, 0x18), le32(h, 0x1c));
    if channels != 1 || !sane_rate(rate) {
        return Ok(None);
    }
    if codec == 0 && extra_flag == 0 && head_size <= 0x20 {
        codec = 0x0002; // .sf0 mini files
        loop_flag = false;
    }
    let (samples, codec, note) = match (codec, config1, config2) {
        (0xcfff, 0x0004_0002, 0) => (psx::bytes_to_samples(data_size, 1), Codec::Psx(psx::Params::default()), None),
        (0x0069, 0x0004_0024, 0x0040_0002) => {
            (ima::xbox_bytes_to_samples(data_size, 1), Codec::Ima(ima::Params::new(ima::Kind::Xbox, 0)), None)
        }
        (0x0001, 0x0010_0002, 0) => (pcm::bytes_to_samples(data_size, 1, 16), Codec::Pcm(pcm::Params::le16(0)), None),
        (0x0002, 0x0010_0000, 0) => (pcm::bytes_to_samples(data_size, 1, 16), Codec::Pcm(pcm::Params::be16(0)), None),
        (0x0000, 0x0010_0000, 0) => (data_size / 8 * 14, Codec::None, Some("Wii DSP ADPCM audio isn't supported")),
        _ => return Ok(None),
    };
    Ok(track(ctx, head_size, data_size, rate, samples, codec, note, loop_flag))
}

fn early(ctx: &mut Ctx, h: &[u8], data_size: u64) -> io::Result<Option<Found>> {
    let head_size = 0x16;
    if data_size + head_size != ctx.size() {
        return Ok(None);
    }
    let codec = le16(h, 0x04);
    let channels = le16(h, 0x06);
    let rate = le32(h, 0x08);
    if codec != 0xcfff || channels != 1 || !sane_rate(rate) || le32(h, 0x10) != 0x0004_0002 || le16(h, 0x14) != 0x6164 {
        return Ok(None);
    }
    let loop_flag = h[0x17] == 0x06;
    let samples = psx::bytes_to_samples(data_size, 1);
    Ok(track(ctx, head_size, data_size, rate, samples, Codec::Psx(psx::Params::default()), None, loop_flag))
}

#[allow(clippy::too_many_arguments)]
fn track(ctx: &Ctx, start: u64, size: u64, rate: u32, samples: u64, codec: Codec, note: Option<&str>, loop_flag: bool) -> Option<Found> {
    if samples == 0 {
        return None;
    }
    let mut t = Track::new(ctx.entry, 0, "SFX0", 1, rate, samples, Data::at(ctx.entry, start, size), codec);
    t.note = note.map(str::to_string);
    if loop_flag {
        t = t.looped(0, samples);
    }
    Some(Found::new(t, start + size))
}
