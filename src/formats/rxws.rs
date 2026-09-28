//! RXWS - from Sony SCEI games [Okage: Shadow King (PS2), Genji (PS2), Bokura no Kazoku
//! (PS2)] (vgmstream meta/rxws.c). A bank: .xws holds header and data (BODY chunk), .xwh
//! is the header of a separate .xwb. PS-ADPCM or PCM16; ATRAC3 streams are listed with a
//! note (they need FFmpeg in vgmstream).

use std::io;

use super::ps2p::{vgm_interleaved, vgm_loop};
use super::{Ctx, Found, Parser, label, le16, le32, sane_rate};
use crate::codecs::{Codec, pcm, psx};
use crate::track::Track;

pub const PARSER: Parser = Parser {
    name: "RXWS",
    magics: &[b"RXWS"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h0 = ctx.bytes(off, 0x24)?;
    if &h0[0..4] != b"RXWS" || &h0[0x10..0x14] != b"FORM" {
        return Ok(vec![]);
    }
    let head_size = le32(&h0, 4) as u64 + 0x10;
    if off + head_size > ctx.size() || head_size > 0x4000_0000 {
        return Ok(vec![]);
    }
    let h = ctx.bytes(off, head_size as usize)?;
    let chunk_size = le32(&h, 0x14) as u64;
    let total = le32(&h, 0x20) as i32;
    if total < 1 || 0x24 + total as u64 * 0x1c > head_size {
        return Ok(vec![]);
    }
    let total = total as usize;

    // BODY chunk (.xws), or the data is in the .xwb.
    let mut body_offset = 0u64;
    let mut cur = 0x10u64;
    while cur < head_size {
        if cur + 8 > head_size {
            break;
        }
        if &h[cur as usize..cur as usize + 4] == b"BODY" {
            body_offset = cur + 0x10;
            break;
        }
        cur += 0x10 + le32(&h, cur as usize + 4) as u64;
    }
    let (body, body_base, body_size) = if body_offset != 0 {
        (ctx.entry, off, head_size)
    } else {
        if off != 0 || ctx.size() != head_size {
            return Ok(vec![]);
        }
        match ctx.sibling("xwb") {
            Some((e, r)) => (e, 0, r.size),
            None => return Ok(vec![]),
        }
    };

    // Names (FTXT, right after FORM).
    let ftxt = 0x20 + chunk_size;
    let names = (ftxt + 0x18 <= head_size && &h[ftxt as usize..ftxt as usize + 4] == b"FTXT" && le32(&h, ftxt as usize + 0x10) as usize == total).then_some(ftxt + 0x10);

    let mut found = Vec::new();
    for i in 0..total {
        let e = 0x24 + 0x1c * i;
        let kind = h[e];
        let channels = h[e + 0x09] as u64;
        let rate = le16(&h, e + 0x0a) as u32;
        let stream_offset = le32(&h, e + 0x10) as u64;
        let num = le32(&h, e + 0x14) as i32;
        let loop_start = le32(&h, e + 0x18) as i32;
        let next = if i + 1 == total { body_size - body_offset } else { le32(&h, e + 0x1c + 0x10) as u64 };
        let stream_size = next.wrapping_sub(stream_offset);
        let start = body_offset + stream_offset;
        if !(1..=8).contains(&channels) || !sane_rate(rate) || num <= 0 || start >= body_size || stream_size > body_size {
            continue;
        }
        let ch = channels as u16;
        let name = match names {
            Some(n) if n + 8 + 4 * i as u64 <= head_size => {
                let at = n + le32(&h, (n + 4 + 4 * i as u64) as usize) as u64;
                if at < head_size { label(&h[at as usize..(at as usize + 0x100).min(h.len())]) } else { None }
            }
            _ => None,
        };
        let s32 = |v: u64| v as u32 as i32 as i64;
        let t = match kind {
            0 => {
                let samples = psx::bytes_to_samples(num as u64, ch);
                let data = vgm_interleaved(body, body_base + start, stream_size, channels, 0x10, body_base + body_size);
                let t = Track::new(ctx.entry, off, "RXWS", ch, rate, samples, data, Codec::Psx(psx::Params::interleaved(0x10)));
                if loop_start >= 0 { vgm_loop(t, s32(psx::bytes_to_samples(loop_start as u64, ch)), samples as i64) } else { t }
            }
            1 => {
                let samples = pcm::bytes_to_samples(num as u64, ch, 16);
                let data = vgm_interleaved(body, body_base + start, stream_size, channels, 2, body_base + body_size);
                let t = Track::new(ctx.entry, off, "RXWS", ch, rate, samples, data, Codec::Pcm(pcm::Params::le16(0)));
                if loop_start >= 0 { vgm_loop(t, s32(pcm::bytes_to_samples(loop_start as u64, ch, 16)), samples as i64) } else { t }
            }
            2 => {
                let samples = (num as i64 - (1024 + 69 * 2)).max(0) as u64;
                let data = vgm_interleaved(body, body_base + start, stream_size, 1, 0, body_base + body_size);
                let mut t = Track::new(ctx.entry, off, "RXWS", ch, rate, samples, data, Codec::None);
                t.note = Some("ATRAC3 audio (not supported)".into());
                t
            }
            _ => continue,
        };
        if t.samples > 0 {
            found.push(Found::new(t, off + head_size).label(name));
        }
    }
    Ok(found)
}
