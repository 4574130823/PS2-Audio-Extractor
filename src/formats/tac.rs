//! tri-Ace codec files [Star Ocean 3, Valkyrie Profile 2, Radiata Stories] (vgmstream
//! meta/tac.c). There's no signature (the games keep them in unnamed bigfile entries), so
//! they're tried at the start of extensionless and .aac files; the header, the codebook and
//! the first frame's CRC must all check out.

use std::io;

use super::vag::vgm_loop;
use super::{Ctx, Found, Parser};
use crate::codecs::{Codec, tac};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "TAC",
    magics: &[],
    magic_at: 0,
    exts: &["", "aac", "laac"],
    locate: None,
    parse,
};

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let file_size = ctx.size() - off;
    if file_size < tac::BLOCK_SIZE as u64 {
        return Ok(vec![]); // the decoder needs the whole first block
    }
    let Some(h) = tac::header(&ctx.bytes(off, 0x20)?) else { return Ok(vec![]) };
    let stream_size = h.file_size as u64;
    if stream_size == 0 || file_size > stream_size || file_size < stream_size - tac::BLOCK_SIZE as u64 {
        return Ok(vec![]);
    }
    // the first frame must decode (codebook, id and CRC)
    let block = ctx.bytes(off, tac::BLOCK_SIZE)?;
    let Some(mut dec) = tac::Tac::new(&block) else { return Ok(vec![]) };
    if h.frame_count == 0 || dec.decode_frame(&block) != tac::Step::Ok {
        return Ok(vec![]);
    }
    let samples = (h.frame_count as i64 - 1) * 1024 + h.frame_last as i64 + 1;
    if samples <= 0 {
        return Ok(vec![]);
    }
    let mut t = Track::new(ctx.entry, off, "TAC", 2, 48000, samples as u64, Data::at(ctx.entry, off, file_size), Codec::Tac(tac::Params {}));
    if h.loop_offset as u64 != stream_size {
        t = vgm_loop(t, (h.loop_frame as i64 - 1) * 1024 + h.loop_discard as i64, samples);
    }
    Ok(vec![Found::new(t, off + file_size)])
}
