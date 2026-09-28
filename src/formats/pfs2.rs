//! 2PFS - from Konami games [Mahoromatic: Moetto-KiraKira Maid-San (PS2), GANTZ The Game
//! (PS2)] (vgmstream meta/2pfs.c): music (.sap, one interleaved stream) and banks of mono
//! sounds (.iap).

use std::io;

use super::ps2p::{vgm_interleaved, vgm_loop};
use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::Track;

pub const PARSER: Parser = Parser {
    name: "2PFS",
    magics: &[b"2PFS"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

const INTERLEAVE: u64 = 0x1000;

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x60)?;
    let version = le16(&h, 0x04);
    if &h[0..4] != b"2PFS" || (version != 1 && version != 2) {
        return Ok(vec![]);
    }
    let size = ctx.size();
    match h[0x10] {
        0x01 => {
            let stream_size = le32(&h, 0x34) as u64;
            let start = le32(&h, 0x38) as u64 + 0x40;
            if le32(&h, 0x3c) != 1 {
                return Ok(vec![]);
            }
            let channels = h[0x40] as u64;
            let loop_flag = h[0x41] != 0;
            let (ls_adjust, rate, ls_block, le_block) = if version == 1 {
                (le16(&h, 0x42) as u64, le32(&h, 0x44), le32(&h, 0x48) as u64, le32(&h, 0x4c) as u64)
            } else {
                (le32(&h, 0x44) as u64, le32(&h, 0x48), le32(&h, 0x50) as u64, le32(&h, 0x54) as u64)
            };
            let end = off + start + stream_size;
            let ch = channels as u16;
            let samples = psx::bytes_to_samples(stream_size, ch);
            if !(1..=8).contains(&channels) || !sane_rate(rate) || samples == 0 || end > size {
                return Ok(vec![]);
            }
            if !psx::plausible(&ctx.bytes(off + start, 0x100.min(stream_size as usize))?) {
                return Ok(vec![]);
            }
            let data = vgm_interleaved(ctx.entry, off + start, stream_size, channels, INTERLEAVE, size);
            let mut t = Track::new(ctx.entry, off, "2PFS", ch, rate, samples, data, Codec::Psx(psx::Params::interleaved(INTERLEAVE)));
            if loop_flag {
                let s32 = |v: u64| v as u32 as i32 as i64;
                let ls = s32(psx::bytes_to_samples(ls_block * channels * INTERLEAVE, ch)) + s32(psx::bytes_to_samples(ls_adjust * channels, ch));
                let le = s32(psx::bytes_to_samples(le_block * channels * INTERLEAVE, ch)) + s32(psx::bytes_to_samples(INTERLEAVE * channels, ch));
                t = vgm_loop(t, ls, le);
            }
            Ok(vec![Found::new(t, end)])
        }
        0x02 | 0x03 => {
            let base = le32(&h, 0x48) as u64 + 0x50;
            let total = le32(&h, 0x54) as i32;
            if !(1..=4096).contains(&total) || off + 0x60 + total as u64 * 0x20 > size {
                return Ok(vec![]);
            }
            let table = ctx.bytes(off + 0x60, total as usize * 0x20)?;
            let mut infos = Vec::new();
            let mut end = off + 0x60 + total as u64 * 0x20;
            for i in 0..total as usize {
                let e = &table[i * 0x20..i * 0x20 + 0x20];
                let channels = e[0];
                let rate = (48000 * le32(e, 0x08) as i32 as i64 / 4096) as i32;
                let start = off + base + le32(e, 0x0c) as u64;
                let stream_size = le32(e, 0x10) as u64;
                // A bad entry fails only its own subsong in vgmstream, but here it means
                // this isn't a bank.
                if channels != 1 || rate <= 0 || !sane_rate(rate as u32) || start + stream_size > size {
                    return Ok(vec![]);
                }
                end = end.max(start + stream_size);
                infos.push((rate as u32, start, stream_size));
            }
            let mut found = Vec::new();
            for (rate, start, stream_size) in infos {
                let samples = psx::bytes_to_samples(stream_size, 1);
                if samples == 0 {
                    continue;
                }
                if !psx::plausible(&ctx.bytes(start, 0x100.min(stream_size as usize))?) {
                    return Ok(vec![]);
                }
                let data = vgm_interleaved(ctx.entry, start, stream_size, 1, INTERLEAVE, size);
                let t = Track::new(ctx.entry, off, "2PFS", 1, rate, samples, data, Codec::Psx(psx::Params::default()));
                found.push(Found::new(t, end));
            }
            Ok(found)
        }
        _ => Ok(vec![]),
    }
}
