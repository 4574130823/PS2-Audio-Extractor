//! .ADM - from Dragon Quest V (PS2) (vgmstream meta/ps2_adm.c, layout/blocked_adm.c).
//! Stereo PS-ADPCM in 0x800 blocks (0x400 per channel), some ending early (their unused
//! lines are zero). No header: found by extension. Loop points are in the game's
//! executable (SLPM_655.55), read when it's next to the file.

use std::io;

use super::ps2p::{merge, vgm_loop};
use super::{Ctx, Found, Parser, le32};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "ADM",
    magics: &[],
    magic_at: 0,
    exts: &["adm"],
    locate: None,
    parse,
};

const BLOCK: u64 = 0x800;
const HALF: u64 = 0x400;

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let size = ctx.size();
    if off != 0 || size < 0x1000 * 9 + 2 {
        return Ok(vec![]);
    }
    for i in 0..10u64 {
        let at = 0x1000 * i + 1;
        if at >= size || ctx.u8(at)? != 0x06 {
            return Ok(vec![]);
        }
    }
    let loop_offset = loop_info(ctx)?;

    // Walk the blocks: each channel plays `used` bytes of its half.
    let mut pieces = Vec::new();
    let mut samples = 0u64;
    let mut loop_start = None;
    let mut b = 0u64;
    loop {
        let blk = ctx.bytes(b, BLOCK as usize)?;
        let real = (size - b).min(BLOCK) as usize;
        let mut used = HALF;
        if blk[1] != 0x06 {
            let mut line = BLOCK as usize;
            while line > 0 {
                line -= 0x10;
                // (past the end of the file vgmstream reads -1: not an unused line)
                if line + 4 <= real && le32(&blk, line) == 0 {
                    match used.checked_sub(0x10) {
                        Some(u) => used = u,
                        None => return Ok(vec![]), // vgmstream would underflow
                    }
                } else {
                    break;
                }
            }
        }
        if loop_offset == Some(b) {
            loop_start = Some(samples);
        }
        let frames = used / 0x10;
        for f in 0..frames {
            pieces.push((b + f * 0x10, 0x10));
            pieces.push((b + HALF + f * 0x10, 0x10));
        }
        samples += psx::bytes_to_samples(used, 1);
        b += BLOCK;
        if b >= size {
            break;
        }
    }
    if samples == 0 || !psx::plausible(&ctx.bytes(0, 0x400)?) {
        return Ok(vec![]);
    }
    let data = Data::blocks(ctx.entry, merge(pieces));
    let mut t = Track::new(ctx.entry, 0, "ADM", 2, 44100, samples, data, Codec::Psx(psx::Params::interleaved(0x10)));
    if loop_offset.is_some() {
        t = vgm_loop(t, loop_start.unwrap_or(0) as i64, samples as i64);
    }
    Ok(vec![Found::new(t, size)])
}

/// Loop start offset, from the table in Dragon Quest V's executable.
fn loop_info(ctx: &mut Ctx) -> io::Result<Option<u64>> {
    let Some((_, mut exe)) = ctx.sibling_named("SLPM_655.55") else { return Ok(None) };
    let path = ctx.path().to_string();
    let name = path.rsplit('/').next().unwrap_or(&path);
    let names = exe.bytes(0x23b3c0, 51 * 0x20)?;
    let mut index = None;
    for i in 0..51 {
        let raw = &names[i * 0x20..i * 0x20 + 0x20];
        let s: Vec<u8> = raw.iter().copied().take_while(|&c| c != 0).collect();
        if s == name.as_bytes() {
            index = Some(i);
            break;
        }
    }
    let Some(i) = index else { return Ok(None) };
    let info = exe.bytes(0x23baf0 + 0x1c * i as u64, 0x1c)?;
    // 0x0c: 0 = loops
    Ok((le32(&info, 0x0c) == 0).then(|| le32(&info, 0) as i32 as i64 as u64))
}
