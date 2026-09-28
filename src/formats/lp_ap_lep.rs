//! LP/AP/LEP - from Konami (KCES)'s Enthusia: Professional Racing (PS2) (vgmstream
//! meta/lp_ap_lep.c). AP and LEP are stereo PS-ADPCM; LP is stereo PCM16 stored rotated
//! by one bit, which the PCM decoder undoes.

use std::io;

use super::ps2p::{vgm_interleaved, vgm_loop};
use super::{Ctx, Found, Parser, le16, le32, sane_rate};
use crate::codecs::{Codec, pcm, psx};
use crate::track::Track;

pub const PARSER: Parser = Parser {
    name: "LP/AP/LEP",
    magics: &[b"LP  ", b"AP  ", b"LEP "],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

/// vgmstream keeps sample counts in 32-bit ints.
fn i32s(samples: u64) -> i64 {
    samples as u32 as i32 as i64
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x60)?;
    let size = ctx.size();
    let channels = 2u64;
    match &h[0..4] {
        b"AP  " | b"LP  " => {
            let is_ap = &h[0..4] == b"AP  ";
            let end = le32(&h, 0x04).wrapping_add(0x20);
            let rate = le32(&h, 0x08);
            let interleave = le32(&h, 0x0c) as u64;
            let loop_start = le32(&h, 0x14);
            let loop_end = le32(&h, 0x18).wrapping_add(0x20);
            let start = le32(&h, 0x1c).wrapping_add(0x20);
            let data_size = end.wrapping_sub(start) as u64;
            let loop_end = loop_end.wrapping_sub(start) as u64;
            let loop_start = loop_start.wrapping_sub(start) as u64;
            let (start, end) = (start as u64, end as u64);
            if !sane_rate(rate) || interleave == 0 || interleave > 0x10000 || start > end || data_size == 0 || off + end > size {
                return Ok(vec![]);
            }
            if is_ap && (interleave % 0x10 != 0 || !psx::plausible(&ctx.bytes(off + start, 0x100.min(data_size as usize))?)) {
                return Ok(vec![]);
            }
            if !is_ap && interleave % 2 != 0 {
                return Ok(vec![]);
            }
            let data = vgm_interleaved(ctx.entry, off + start, data_size, channels, interleave, size);
            let t = if is_ap {
                let samples = psx::bytes_to_samples(data_size, 2);
                let t = Track::new(ctx.entry, off, "AP", 2, rate, samples, data, Codec::Psx(psx::Params::interleaved(interleave)));
                if loop_start != 0 { vgm_loop(t, i32s(psx::bytes_to_samples(loop_start, 2)), i32s(psx::bytes_to_samples(loop_end, 2))) } else { t }
            } else {
                let samples = pcm::bytes_to_samples(data_size, 2, 16);
                // PCM16 stored rotated by one bit (vgmstream's meta/lp_ap_lep_streamfile.h).
                let codec = Codec::Pcm(pcm::Params { rol1: true, ..pcm::Params::le16(interleave) });
                let t = Track::new(ctx.entry, off, "LP", 2, rate, samples, data, codec);
                if loop_start != 0 { vgm_loop(t, i32s(pcm::bytes_to_samples(loop_start, 2, 16)), i32s(pcm::bytes_to_samples(loop_end, 2, 16))) } else { t }
            };
            Ok(vec![Found::new(t, off + end)])
        }
        b"LEP " => {
            let data_size = le32(&h, 0x08) as u64;
            let rate = le16(&h, 0x12) as u32;
            let loop_start = le32(&h, 0x58) as u64;
            let start = 0x800u64;
            if !sane_rate(rate) || data_size == 0 || off + start + data_size > size {
                return Ok(vec![]);
            }
            if !psx::plausible(&ctx.bytes(off + start, 0x100.min(data_size as usize))?) {
                return Ok(vec![]);
            }
            let samples = psx::bytes_to_samples(data_size, 2);
            let data = vgm_interleaved(ctx.entry, off + start, data_size, channels, 0x10, size);
            let mut t = Track::new(ctx.entry, off, "LEP", 2, rate, samples, data, Codec::Psx(psx::Params::interleaved(0x10)));
            if loop_start != 0 {
                t = vgm_loop(t, i32s(psx::bytes_to_samples(loop_start, 2)), i32s(samples));
            }
            Ok(vec![Found::new(t, off + start + data_size)])
        }
        _ => Ok(vec![]),
    }
}
