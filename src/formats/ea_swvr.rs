//! EA SWVR streams (vgmstream meta/ea_swvr.c + layout/blocked_ea_swvr.c), demuxed from
//! .av/.trk/.mis files [Future Cop L.A.P.D. (PS1/PC), Freekstyle (PS2), Rumble Racing (PS2),
//! NASCAR Rumble (PS1)]: "RVWS" then blocks ("VAGM" stereo / "VAGB" mono PS-ADPCM, "MSIC"/
//! "SHOC" PC PCM, "FILL" padding). Freekstyle's raw movie audio starts right at a "VAGM"
//! block and can hold several subsongs. The GameCube versions (DSP) aren't handled.

use std::io;
use std::sync::Arc;

use super::ea_schl::Rd;
use super::{Ctx, Found, Parser};
use crate::codecs::{Codec, ea_xa, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "EA SWVR",
    // "MGAV" is a little-endian "VAGM" block: Freekstyle (PS2) raw movie audio
    magics: &[b"RVWS", b"MGAV"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

const VAGM: u32 = 0x5641474D;
const VAGB: u32 = 0x56414742;
const MSIC: u32 = 0x4D534943;
const SHOC: u32 = 0x53484F43;
const FILL: u32 = 0x46494C4C;
const PADD: u32 = 0x50414444;
const SDAT: u32 = 0x53444154;
const SHDR: u32 = 0x53484452;

struct Blk {
    offset: u64,
    header: u64,
    channel_size: u64,
    /// subsong this block belongs to (1-based), 0 = any
    subsong: u32,
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let entry = ctx.entry;
    let mut r = Rd(&mut ctx.r);
    let size = r.size();
    if off + 0x40 > size {
        return Ok(vec![]);
    }
    let magic = r.id(off)?;
    let (mut start, loop_block) = if magic == u32::from_be_bytes(*b"RVWS") {
        (r.u32(off + 4, false)? as u64, r.u32(off + 0x0c, false)? as u64)
    } else {
        (0, 0)
    };
    if magic == u32::from_be_bytes(*b"RVWS") && (!(0x10..=0x10000).contains(&start) || r.u32(off + 8, false)? != 0) {
        return Ok(vec![]);
    }
    if r.u32(off + start, false)? == PADD {
        start += r.u32(off + start + 4, false)? as u64;
    }
    if r.u32(off + start, false)? == FILL {
        start += r.u32(off + start + 4, false)? as u64;
    }
    if off + start + 0x40 > size {
        return Ok(vec![]);
    }
    let first = off + start;
    let block_id = r.u32(first, false)?;
    let (kind, rate, channels, mut subsongs) = match block_id {
        VAGM => {
            if r.u16(first + 0x1a, false)? == 0x0024 {
                (ea_xa::Kind::Psx, 22050, 2u16, r.u32(first + 0x0c, false)? as u64 + 1)
            } else {
                (ea_xa::Kind::Psx, (44100.0 * 1324.0 / 4096.0) as u32, 2, 1)
            }
        }
        VAGB => {
            let rate = if r.u16(first + 0x1a, false)? == 0x6400 { 22050 } else { (44100.0 * 1080.0 / 4096.0) as u32 };
            (ea_xa::Kind::Psx, rate, 1, 1)
        }
        MSIC => (ea_xa::Kind::Pcm8UInt, 14291, 2, 1),
        SHOC => {
            if r.u32(first + 0x10, false)? != SHDR || r.u32(first + 0x18, false)? != u32::from_be_bytes(*b"snds") {
                return Ok(vec![]);
            }
            (ea_xa::Kind::Pcm8UInt, 22050, 1, 1)
        }
        _ => return Ok(vec![]),
    };
    if subsongs == 0 || subsongs > 64 {
        return Ok(vec![]);
    }

    // Walk the blocks (block_update_ea_swvr) to the end of the file, or (inside a bigger
    // file) to the first block that can't be one.
    let mut blocks = Vec::new();
    let mut pos = first;
    let mut end = first;
    let mut known = 0;
    while pos < size {
        let id = r.u32(pos, false)?;
        let mut bsize = r.u32(pos + 4, false)? as u64;
        let rel = pos - off;
        let (mut header, mut channel_size, mut subsong) = (0u64, 0u64, 0u32);
        match id {
            VAGM => {
                if r.u16(pos + 0x1a, false)? == 0x0024 {
                    header = 0x40;
                    subsong = r.u32(pos + 0x0c, false)?.wrapping_add(1);
                } else {
                    header = 0x1c;
                }
                channel_size = bsize.saturating_sub(header) / channels as u64;
            }
            VAGB => {
                header = if r.u16(pos + 0x1a, false)? == 0x6400 { 0x40 } else { 0x18 };
                channel_size = bsize.saturating_sub(header) / channels as u64;
            }
            MSIC => {
                header = 0x1c;
                channel_size = bsize.saturating_sub(header) / channels as u64;
            }
            SHOC => {
                if r.u32(pos + 0x10, false)? == SDAT {
                    header = 0x14;
                    channel_size = bsize.saturating_sub(header) / channels as u64;
                }
            }
            FILL => {
                if (rel + 4) % 0x6000 == 0 || (rel + 4) % 0x10000 == 0 || bsize > 0x100000 {
                    bsize = 4;
                }
            }
            0xFFFFFFFF => {
                end = pos;
                break;
            }
            _ => {}
        }
        // (vgmstream would take any size here; a bad one means we've left the stream)
        if bsize < 4 || bsize > 0x100000 || pos + bsize > size + 0x800 {
            end = pos;
            break;
        }
        if matches!(id, VAGM | VAGB | MSIC | SHOC | FILL | PADD) {
            known += 1;
        }
        if channel_size > 0 && (id == VAGM || id == VAGB || id == MSIC || id == SHOC) {
            blocks.push(Blk { offset: pos, header, channel_size, subsong });
        } else {
            blocks.push(Blk { offset: pos, header, channel_size: 0, subsong: 0 });
        }
        pos += bsize;
        end = pos.min(size);
    }
    // (no header to check: a real stream is a chain of known blocks with sane audio)
    if known < 2 || blocks.iter().all(|b| b.channel_size == 0) {
        return Ok(vec![]);
    }
    if kind == ea_xa::Kind::Psx {
        let b = blocks.iter().find(|b| b.channel_size > 0).unwrap();
        if b.channel_size < 0x40 {
            return Ok(vec![]);
        }
        for c in 0..channels as u64 {
            let probe = r.b(b.offset + b.header + b.channel_size * c, 0x80.min(b.channel_size as usize))?;
            if !psx::plausible(&probe) {
                return Ok(vec![]);
            }
        }
    }
    if block_id != VAGM || r.u16(first + 0x1a, false)? != 0x0024 {
        subsongs = 1;
    }

    let mut found = Vec::new();
    for target in 1..=subsongs as u32 {
        let mut list = Vec::new();
        let (mut total, mut audio_count) = (0u64, 0u64);
        let mut loop_start = None;
        for b in &blocks {
            let mut cs = b.channel_size;
            if b.subsong != 0 && b.subsong != target {
                cs = 0;
            }
            if loop_block > 0 && audio_count == loop_block {
                loop_start = Some(total);
            }
            let samples = match kind {
                ea_xa::Kind::Psx => cs / 16 * 28,
                _ => cs,
            };
            if samples > 0 {
                let il = if kind == ea_xa::Kind::Pcm8UInt { 1 } else { cs };
                let starts = (0..channels as u64).map(|i| b.offset + b.header + il * i - off).collect();
                list.push(ea_xa::Block { samples: samples as u32, starts, reset: false });
                audio_count += 1;
            }
            total += samples;
        }
        if total == 0 {
            continue;
        }
        let codec = Codec::EaXa(ea_xa::Params { kind, blocks: Arc::new(list) });
        let mut t = Track::new(entry, off, "EA SWVR", channels, rate, total, Data::at(entry, off, end - off), codec);
        if loop_block > 0 {
            if let Some((a, b)) = super::ea_schl::vgm_loop(loop_start.unwrap_or(0) as i64, total as i64, total) {
                t = t.looped(a, b);
            }
        }
        found.push(Found::new(t, end));
    }
    Ok(found)
}
