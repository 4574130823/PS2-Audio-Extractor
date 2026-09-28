//! SShd/SSbd (.ADS/.SS2): Sony's stream format, for music and voices (vgmstream
//! meta/sshd.c, including its "ADSC" and "cavia stream" containers, which are just an
//! SShd at 0x08 / 0x7d8).

use std::io;

use super::vag::{Cache, vgm_loop};
use super::{Ctx, Found, Parser, le32, sane_rate};
use crate::codecs::{Codec, pcm, psx, ima};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "ADS",
    magics: &[b"SShd"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

#[derive(PartialEq)]
enum Coding {
    Pcm16,
    /// Codec 0: "PCM16 big endian" per Sony's docs. vgmstream rejects it ("probably never
    /// used"), but PS2 video (.PSS) audio headers list it as a type, so it's read here.
    Pcm16Be,
    Psx,
    Ima,
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x30)?;
    if &h[0..4] != b"SShd" || &h[0x20..0x24] != b"SSbd" {
        return Ok(vec![]);
    }
    // vgmstream's file size is the (sub)file the header starts: known when it's at the start
    // of a file, or inside an ADSC / cavia container at the start of one.
    let known = off == 0
        || (off == 0x08 && ctx.is(0, b"ADSC")? && ctx.u32le(4)? == 1)
        || (off == 0x7d8 && ctx.is(0, b"cavia stream")?);
    let avail = ctx.size() - off;
    let file_size = if known { Some(avail) } else { None };

    let header_size = le32(&h, 0x04) as u64;
    if header_size != 0x18 && header_size != 0x20 && Some(header_size) != file_size.map(|f| f.wrapping_sub(8)) {
        return Ok(vec![]);
    }
    let codec = le32(&h, 0x08);
    let mut rate = le32(&h, 0x0c);
    let channels = le32(&h, 0x10);
    let mut interleave = le32(&h, 0x14);
    let mut coding = match codec {
        0x01 | 0x8000_0001 => Coding::Pcm16,
        0x00 => Coding::Pcm16Be,
        0x10 | 0x02 => Coding::Psx,
        _ => return Ok(vec![]),
    };
    // Angel Studios/Rockstar San Diego videos hijack the PCM codec for IMA
    if coding == Coding::Pcm16 && rate == 12000 && interleave == 0x200 {
        rate = 48000;
        interleave = 0x40;
        coding = Coding::Ima;
    }
    if !sane_rate(rate) || !(1..=8).contains(&channels) || (channels > 1 && interleave == 0) || interleave > 0x10000 {
        return Ok(vec![]);
    }
    let ch = channels;

    // sizes
    let mut body = le32(&h, 0x24);
    let limit = file_size.unwrap_or(avail);
    if body as u64 + 0x28 > limit {
        body = limit.saturating_sub(0x28) as u32;
    }
    if let Some(fs) = file_size {
        // True Fortune: odd stream size
        if body as u64 * 2 == fs.wrapping_sub(0x18) {
            body = body * 2 - 0x10;
        }
    }
    let mut stream_size = body;

    // start: sector padding after the header [Evergrace II, Armored Core 3]
    let mut start = 0x28u32;
    let padded = match file_size {
        Some(fs) => fs - body as u64 >= 0x800,
        // inside other files: the padding is empty (or "PAD!" then empty)
        None => {
            let pad = ctx.bytes(off + 0x28, 0x800 - 0x28)?;
            let skip = if &pad[0..4] == b"PAD!" { 4 } else { 0 };
            avail >= 0x800 + body as u64 && pad[skip..].iter().all(|&b| b == 0)
        }
    };
    if padded {
        start = 0x800;
    }
    // "ADSC" alignment
    if coding == Coding::Psx && le32(&h, 0x28) == 0x1000 && le32(&h, 0x2c) == 0 && ctx.u32le(off + 0x1008)? != 0 {
        let pad = ctx.bytes(off + 0x2c, 0xFDC)?;
        if pad.iter().all(|&b| b == 0) {
            start = 0x1000 - 0x08;
        }
    }

    // loops
    let loop_start = le32(&h, 0x18);
    let loop_end = le32(&h, 0x1c);
    let mut loop_flag = false;
    let mut is_samples = false;
    let (mut ls_sample, mut le_sample, mut ls_off, mut le_off) = (0u32, 0u32, 0u32, 0u32);
    let (mut silent_cavia, mut silent_capcom) = (false, false);
    if loop_start != 0xFFFF_FFFF && loop_end == 0xFFFF_FFFF {
        if codec == 0x02 {
            // Capcom: address * 0x10
            loop_flag = loop_start.wrapping_mul(0x10).wrapping_add(0x200) < body;
            ls_off = loop_start.wrapping_mul(0x10);
            silent_capcom = true;
        } else if &h[0x28..0x2c] == b"PAD!" {
            // Super Galdelic Hour: PCM bytes
            loop_flag = true;
            ls_sample = loop_start / 2 / ch;
            is_samples = true;
        } else if loop_start % 0x800 == 0 && loop_start > 0 {
            // cavia: offset from the container
            loop_flag = true;
            ls_off = loop_start - 0x800;
            silent_cavia = true;
        } else {
            // Katakamuna: address * 0x10
            loop_flag = true;
            ls_off = loop_start.wrapping_mul(0x10);
        }
    } else if loop_start != 0xFFFF_FFFF && loop_end != 0xFFFF_FFFF && loop_end > 0 {
        let pcm = matches!(coding, Coding::Pcm16 | Coding::Pcm16Be);
        let psx = coding == Coding::Psx;
        if loop_end <= body / 0x200 && pcm {
            loop_flag = true;
            ls_off = loop_start.wrapping_mul(0x200);
            le_off = loop_end.wrapping_mul(0x200);
        } else if loop_end <= body / 0x70 && pcm {
            loop_flag = true;
            ls_off = loop_start.wrapping_mul(0x70);
            le_off = loop_end.wrapping_mul(0x70);
        } else if loop_end <= body / 0x20 && pcm {
            loop_flag = true;
            ls_off = loop_start.wrapping_mul(0x20);
            le_off = loop_end.wrapping_mul(0x20);
        } else if loop_end <= body / 0x20 && psx {
            loop_flag = true;
            ls_off = loop_start.wrapping_mul(0x20);
            le_off = loop_end.wrapping_mul(0x20);
        } else if loop_end <= body / 0x10
            && psx
            && (ctx.u32be(off + 0x28 + loop_end as u64 * 0x10 + 0x10)? == 0x0007_7777 || ctx.u32be(off + 0x28 + loop_end as u64 * 0x10 + 0x20)? == 0x0007_7777)
        {
            // not-quite-looping sfx [Kono Aozora ni Yakusoku, Chanter]
        } else if (loop_end > body / 0x20 && psx) || (loop_end > body / 0x70 && pcm) {
            // loops in samples [Eve of Extinction, Culdcept, WWE Smackdown! 3]
            loop_flag = true;
            ls_sample = loop_start;
            le_sample = loop_end;
            is_samples = true;
        }
    }

    // Empty frames in the last interleave block are skipped for smooth looping.
    if coding == Coding::Psx {
        let mut c = Cache::new(ctx);
        let mut o = off + start as u64 + stream_size as u64;
        let min = o.saturating_sub(interleave as u64);
        loop {
            o -= 0x10;
            let f = c.get(o, 0x10)?.to_vec();
            let w: Vec<u32> = f.chunks(4).map(|b| u32::from_be_bytes(b.try_into().unwrap())).collect();
            let rest0 = w[1] == 0 && w[2] == 0 && w[3] == 0;
            let trim = f[1] == 0x07
                || (w[0] == 0 && rest0)
                || (w[0] == 0x0000_7777 && w[1] == 0x7777_7777 && w[2] == 0x7777_7777 && w[3] == 0x7777_7777)
                || (w[0] == 0x0C02_0000 && rest0 && silent_cavia)
                || (w[0] == 0x0C01_0000 && rest0 && silent_capcom);
            if !trim {
                break;
            }
            stream_size = stream_size.wrapping_sub(0x10 * ch);
            if o <= min {
                break;
            }
        }
    }

    let samples = match coding {
        Coding::Pcm16 | Coding::Pcm16Be => stream_size / 2 / ch,
        Coding::Psx => stream_size / ch / 0x10 * 28,
        Coding::Ima => stream_size / ch * 2,
    } as u64;
    if samples == 0 || stream_size > body {
        return Ok(vec![]);
    }
    let data_off = off + start as u64;
    if data_off >= ctx.size() {
        return Ok(vec![]);
    }
    let il = interleave as u64;
    // whole rows, as vgmstream's interleave layout reads them
    let row = il * ch as u64;
    let data_size = if ch > 1 { (stream_size as u64).div_ceil(row) * row } else { stream_size as u64 };
    let data = Data::at(ctx.entry, data_off, data_size);
    let codec = match coding {
        Coding::Psx => {
            let probe = ctx.bytes(data_off, 0x100.min(body as usize))?;
            if !psx::plausible(&probe) {
                return Ok(vec![]);
            }
            Codec::Psx(psx::Params::interleaved(il))
        }
        Coding::Pcm16 => Codec::Pcm(pcm::Params::le16(il)),
        Coding::Pcm16Be => Codec::Pcm(pcm::Params::be16(il)),
        // vgmstream's coding_DVI_IMA_mono with the interleave
        Coding::Ima => Codec::Ima(ima::Params::new(ima::Kind::Dvi, il)),
    };
    let mut t = Track::new(ctx.entry, off, "ADS", ch as u16, rate, samples, data, codec);
    if loop_flag {
        let (a, mut b) = if is_samples {
            (ls_sample as i64, le_sample as i64)
        } else {
            match coding {
                Coding::Pcm16 | Coding::Pcm16Be => ((ls_off / 2 / ch) as i64, (le_off / 2 / ch) as i64),
                _ => ((ls_off / ch / 0x10 * 28) as i64, (le_off / ch / 0x10 * 28) as i64),
            }
        };
        if coding == Coding::Ima && !is_samples {
            return Ok(vec![]); // vgmstream fails on these (never seen)
        }
        if b == 0 {
            b = samples as i64;
        }
        if b > samples as i64 {
            b = samples as i64;
        }
        t = vgm_loop(t, a, b);
    }
    let end = (off + start as u64 + body as u64).min(ctx.size());
    Ok(vec![Found::new(t, end)])
}
