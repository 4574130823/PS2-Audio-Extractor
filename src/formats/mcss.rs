//! Guerrilla's MSS - Found in ShellShock Nam '67 (PS2/Xbox), Killzone (PS2)
//! (vgmstream meta/mcss.c).

use std::io;

use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, ima, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "MSS",
    magics: &[b"MCSS"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    if &h[0..4] != b"MCSS" || le32(&h, 0x04) != 0x100 {
        return Ok(vec![]);
    }
    let start = le32(&h, 0x08) as u64;
    let data_size = le32(&h, 0x0c) as u64;
    let rate = le32(&h, 0x10);
    let channels = le16(&h, 0x16);
    let interleave = le32(&h, 0x18) as u64;
    let chan_size = le32(&h, 0x1c) as u64;
    if !sane_rate(rate) || !(1..=8).contains(&channels) || start < 0x20 || interleave == 0 || interleave > 0x10000 {
        return Ok(vec![]);
    }
    let ch = channels as u64;
    let data_off = off + start;
    if data_off >= ctx.size() {
        return Ok(vec![]);
    }
    let avail = ctx.size() - data_off;
    if interleave == 0x4800 {
        // Xbox IMA in stereo pairs; the header's sizes are off, the data runs to the end.
        if ch > 2 && !ch.is_multiple_of(2) {
            return Ok(vec![]);
        }
        let size = if off == 0 && ctx.ext() == "mss" { avail } else { data_size.min(avail) };
        let probe = ctx.bytes(data_off, 0x900.min(size) as usize)?;
        if size == 0 || !ima::xbox_plausible(&probe, channels.min(2)) {
            return Ok(vec![]);
        }
        let samples = ima::xbox_bytes_to_samples(size, channels);
        let data = Data::at(ctx.entry, data_off, size);
        let t = Track::new(ctx.entry, off, "MSS", channels, rate, samples, data, Codec::Ima(ima::Params::new(ima::Kind::Xbox, 0x2400)));
        return Ok(vec![Found::new(t, data_off + size)]);
    }
    if chan_size == 0 || chan_size * ch > data_size.max(avail) {
        return Ok(vec![]);
    }
    let size = if ch > 1 { chan_size.div_ceil(interleave) * interleave * ch } else { chan_size };
    let probe = ctx.bytes(data_off, 0x100.min(size.min(avail)) as usize)?;
    if !psx::plausible(&probe) {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(chan_size, 1);
    let data = Data::at(ctx.entry, data_off, size);
    let t = Track::new(ctx.entry, off, "MSS", channels, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, (data_off + size).min(ctx.size()))])
}
