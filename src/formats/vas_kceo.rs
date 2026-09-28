//! .VAS - from Konami Computer Entertainment Osaka games [Jikkyou Powerful Pro Yakyuu 8
//! (PS2), TMNT 2: Battle Nexus (multi)] (vgmstream meta/vas_kceo.c): single streams
//! (PS2 PS-ADPCM, PC PCM16; Xbox IMA and GameCube DSP get a note) and the containers
//! holding them. Found by extension, and PS2 containers also by their fixed first word.

use std::io;

use super::ps2p::{vgm_interleaved, vgm_loop};
use super::{Ctx, Found, Parser, be32, le32};
use crate::codecs::{Codec, pcm, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "VAS",
    magics: &[b"\xab\x8a\x5a\x00"],
    magic_at: 0,
    exts: &["vas", "dsp"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let size = ctx.size();
    let is_ext = off == 0 && matches!(ctx.ext().as_str(), "vas" | "dsp");
    if is_ext {
        if let Some(t) = single(ctx, 0, size)? {
            return Ok(vec![Found::new(t, size)]);
        }
        if ctx.ext() != "vas" {
            return Ok(vec![]);
        }
    }
    let first = ctx.bytes(off, 0x100)?;
    let mut subs: Vec<(u64, u64)> = Vec::new();
    let end;
    if be32(&first, 0) == 0xab8a_5a00 {
        // PS2
        let total_size = le32(&first, 4) as u64 * 0x800 + 0x800;
        let total = le32(&first, 8) as i32;
        if off + total_size > size || (is_ext && total_size != size) || !(1..=0x100).contains(&total) {
            return Ok(vec![]);
        }
        end = off + total_size;
        if le32(&first, 0x94) != 0 {
            let table = ctx.bytes(off + 0x800, total as usize * 0x10)?;
            for i in 0..total as usize {
                subs.push((le32(&table, i * 0x10) as u64 * 0x800, le32(&table, i * 0x10 + 8) as u64 + 0x800));
            }
        } else {
            let mut o = 0x800u64;
            for _ in 0..total {
                let s = ctx.u32le(off + o)? as u64 + 0x800;
                subs.push((o, s));
                o += s;
            }
        }
    } else if !is_ext {
        return Ok(vec![]);
    } else if le32(&first, 0) == 0x800 {
        // Xbox/PC
        let total = le32(&first, 4) as i32;
        if !(1..=0x100).contains(&total) || le32(&first, 8) != 0x800 {
            return Ok(vec![]);
        }
        let table = ctx.bytes(8, total as usize * 8 + 8)?;
        for i in 0..total as usize {
            let o = le32(&table, i * 8) as u64;
            let next = if i + 1 == total as usize { size } else { le32(&table, i * 8 + 8) as u64 };
            subs.push((o, next.wrapping_sub(o) & 0xffff_ffff));
        }
        end = size;
    } else {
        // Files pasted together.
        if le32(&first, 0) as u64 + 0x800 >= size {
            return Ok(vec![]);
        }
        let mut o = 0u64;
        while o < size {
            let s = ctx.u32le(o)? as u64 + 0x800;
            if s > 0x800 {
                subs.push((o, s));
            }
            o += s;
        }
        if o > size {
            return Ok(vec![]);
        }
        end = size;
    }
    let mut found = Vec::new();
    for (o, s) in subs {
        let at = off + o;
        if at + s > end || s == 0 {
            continue;
        }
        if let Some(t) = single(ctx, at, s)? {
            found.push(Found::new(t, end));
        }
    }
    Ok(found)
}

/// One stream: the sub-file [base, base + size).
fn single(ctx: &mut Ctx, base: u64, size: u64) -> io::Result<Option<Track>> {
    if size <= 0x800 {
        return Ok(None);
    }
    let h = ctx.bytes(base, 0x40)?;
    enum Kind {
        Psx,
        Pcm,
        Ima,
        Dsp,
    }
    let (channels, rate, loop_start, loop_end, volume, dummy, loop_flag, data_size, kind);
    if le32(&h, 0) == 1 || le32(&h, 0) == 0x69 {
        channels = le32(&h, 4) as u64;
        let block_size = le32(&h, 8) as u64;
        rate = le32(&h, 0x0c) as i32;
        loop_start = le32(&h, 0x10) as u64;
        loop_end = le32(&h, 0x14) as u64;
        volume = le32(&h, 0x18);
        loop_flag = le32(&h, 0x1c) != 0;
        data_size = le32(&h, 0x24) as u64;
        dummy = le32(&h, 0x30);
        let pc = le32(&h, 0) == 1;
        if block_size != channels * if pc { 2 } else { 0x24 } {
            return Ok(None);
        }
        kind = if pc { Kind::Pcm } else { Kind::Ima };
    } else if le32(&h, 0) as u64 + 0x800 == size {
        data_size = le32(&h, 0) as u64;
        rate = le32(&h, 4) as i32;
        volume = le32(&h, 8);
        dummy = le32(&h, 0x0c);
        loop_flag = le32(&h, 0x10) != 0;
        loop_start = le32(&h, 0x14) as u64;
        channels = 2;
        loop_end = data_size;
        // ps_check_format
        let probe = ctx.bytes(base + 0x800, 0x1000.min(size - 0x800) as usize)?;
        if probe.chunks(16).any(|f| f[0] >> 4 > 5 || f.get(1).copied().unwrap_or(0) > 7) {
            return Ok(None);
        }
        kind = Kind::Psx;
    } else if be32(&h, 0) as u64 + 0x800 == size {
        data_size = be32(&h, 0) as u64;
        rate = be32(&h, 4) as i32;
        volume = be32(&h, 8);
        dummy = be32(&h, 0x0c);
        loop_flag = be32(&h, 0x10) != 0;
        loop_start = be32(&h, 0x14) as u64;
        channels = 2;
        loop_end = data_size;
        if ctx.bytes(base + 0x8c, 4)? != [0, 0, 0, 2] {
            return Ok(None);
        }
        kind = Kind::Dsp;
    } else {
        return Ok(None);
    }
    if channels != 2 || !(8000..=48000).contains(&rate) || volume == 0 || volume > 0xff || dummy != 0 || data_size == 0 {
        return Ok(None);
    }
    let start = base + 0x800;
    let s32 = |v: u64| v as u32 as i32 as i64;
    let rate = rate as u32;
    let file_end = base + size;
    let t = match kind {
        Kind::Psx => {
            let samples = psx::bytes_to_samples(data_size, 2);
            let data = vgm_interleaved(ctx.entry, start, data_size, 2, 0x200, file_end);
            let t = Track::new(ctx.entry, base, "VAS", 2, rate, samples, data, Codec::Psx(psx::Params::interleaved(0x200)));
            if loop_flag { vgm_loop(t, s32(psx::bytes_to_samples(loop_start, 2)), s32(psx::bytes_to_samples(loop_end, 2))) } else { t }
        }
        Kind::Pcm => {
            let samples = pcm::bytes_to_samples(data_size, 2, 16);
            let data = Data::at(ctx.entry, start, data_size);
            let t = Track::new(ctx.entry, base, "VAS", 2, rate, samples, data, Codec::Pcm(pcm::Params::le16(0)));
            if loop_flag { vgm_loop(t, s32(pcm::bytes_to_samples(loop_start, 2, 16)), s32(pcm::bytes_to_samples(loop_end, 2, 16))) } else { t }
        }
        Kind::Ima => {
            let adj = |v: u64| v - v / 0x20000 * 0x20;
            let samples = adj(data_size) / 0x48 * 64;
            let mut t = Track::new(ctx.entry, base, "VAS", 2, rate, samples, Data::at(ctx.entry, start, data_size), Codec::None);
            t.note = Some("Xbox IMA ADPCM audio (not supported)".into());
            t
        }
        Kind::Dsp => {
            let samples = data_size / 2 / 8 * 14;
            let mut t = Track::new(ctx.entry, base, "VAS", 2, rate, samples, Data::at(ctx.entry, start, data_size), Codec::None);
            t.note = Some("NGC DSP audio (not supported)".into());
            t
        }
    };
    Ok((t.samples > 0).then_some(t))
}
