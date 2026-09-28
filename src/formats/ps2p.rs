//! PS2P - THQ Australia (Studio Oz) sound bank of VAGs [Jimmy Neutron: Attack of the
//! Twonkies (PS2), SpongeBob: Lights, Camera, Pants! (PS2)] (vgmstream meta/ps2p.c).
//!
//! Also holds helpers other ports use: vgmstream's PS-ADPCM utilities (loop flag search,
//! end padding search) and its VAGp sub-file parser (meta/vag.c), for banks of VAGs.

use std::io;

use super::{Ctx, Found, Parser, label, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::disc::Reader;
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "PS2P",
    magics: &[b"ps2p"],
    magic_at: 0,
    exts: &[],
    locate: None,
    parse,
};

const TABLE: u64 = 0x20;
const ENTRY: u64 = 0x0c;
const AUX: u64 = 0x1c;

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 0x20)?;
    if &h[0..4] != b"ps2p" {
        return Ok(vec![]);
    }
    let alignment = le32(&h, 0x0c) as u64;
    let file_count = le32(&h, 0x14) as u64;
    let aux_count = le32(&h, 0x18) as u64;
    let avail = ctx.size() - off;
    if file_count < 1 || file_count > 0x4000 || aux_count > file_count || alignment == 0 || alignment > avail {
        return Ok(vec![]);
    }
    let table2 = TABLE + file_count * ENTRY;
    let tables_end = table2 + aux_count * AUX;
    if tables_end > avail || tables_end > alignment {
        return Ok(vec![]);
    }
    let t = ctx.bytes(off, tables_end as usize)?;
    // (offset, size) of each file, relative to the bank.
    let info = |id: u64| -> Option<(u64, u64)> {
        if id >= file_count {
            return None;
        }
        let (o, s) = if id == 0 {
            (alignment, le32(&t, TABLE as usize) as u64)
        } else {
            (le32(&t, (TABLE + (id - 1) * ENTRY + 8) as usize) as u64, le32(&t, (TABLE + id * ENTRY) as usize) as u64)
        };
        (o != 0 && s != 0).then_some((o, s))
    };
    // The whole bank must be here: every file inside it.
    let mut end = tables_end;
    for id in 0..file_count {
        match info(id) {
            Some((o, s)) if o >= tables_end && o + s <= avail => end = end.max(o + s),
            _ => return Ok(vec![]),
        }
    }
    let mapped = |id: u64| (0..aux_count).any(|i| le32(&t, (table2 + i * AUX) as usize) as u64 == id);

    let total = if aux_count > 0 { aux_count } else { file_count };
    let mut found = Vec::new();
    for sub in 0..total {
        let (id_l, id_r) = if aux_count > 0 {
            let l = le32(&t, (table2 + sub * AUX) as usize) as u64;
            let r = (l + 1 < file_count && !mapped(l + 1)).then_some(l + 1);
            (l, r)
        } else {
            (sub, None)
        };
        let Some((off_l, size_l)) = info(id_l) else { continue };
        let Some(vl) = vag_subfile(&mut ctx.r, off + off_l, size_l)? else { continue };
        let name = if aux_count > 0 {
            let strings = table2 + aux_count * AUX - 4;
            let rel = if sub > 0 { le32(&t, (table2 + (sub - 1) * AUX + 0x18) as usize) as u64 } else { 0 };
            if strings + rel < avail { label(&ctx.bytes(off + strings + rel, 0x100)?) } else { None }
        } else {
            vl.name.clone()
        };
        let track = match id_r {
            None => vl.track(ctx.entry, off, "PS2P"),
            Some(r) => {
                let Some((off_r, size_r)) = info(r) else { continue };
                let Some(vr) = vag_subfile(&mut ctx.r, off + off_r, size_r)? else { continue };
                if vl.channels != 1 || vr.channels != 1 {
                    continue;
                }
                // Two mono VAGs played as layers: one frame of each in turn.
                let frames = vl.samples.div_ceil(28);
                let mut pieces = Vec::new();
                let mut t = Track::new(ctx.entry, off, "PS2P", 2, vl.rate, vl.samples, Data::default(), Codec::Psx(psx::Params::interleaved(0x10)));
                if frames * 16 > vr.avail {
                    t.note = Some("Right channel is shorter than the left".into());
                } else {
                    for f in 0..frames {
                        pieces.push((vl.starts[0] + f * 16, 16));
                        pieces.push((vr.starts[0] + f * 16, 16));
                    }
                    t.data = Data::blocks(ctx.entry, merge(pieces));
                }
                t
            }
        };
        found.push(Found::new(track, off + end).label(name));
    }
    Ok(found)
}

/// Joins pieces that follow each other in the file.
pub(crate) fn merge(pieces: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(pieces.len());
    for (o, s) in pieces {
        match out.last_mut() {
            Some(l) if l.0 + l.1 == o => l.1 += s,
            _ => out.push((o, s)),
        }
    }
    out
}

/// An offset past the end of any file: a data piece there reads as zeros.
pub(crate) const ZERO: u64 = 1 << 62;

/// Data for `size` bytes of `channels`-interleaved audio at `start`, read the way
/// vgmstream's interleave layout reads it: a short last row is read whole (each channel
/// from its own block), from the bytes that follow, which are zeros past `file_end`.
pub(crate) fn vgm_interleaved(entry: usize, start: u64, size: u64, channels: u64, interleave: u64, file_end: u64) -> Data {
    if channels <= 1 || interleave == 0 {
        return Data::at(entry, start, size);
    }
    let row = interleave * channels;
    let len = size.div_ceil(row) * row;
    let real = len.min(file_end.saturating_sub(start));
    let mut pieces = vec![(start, real)];
    if real < len {
        pieces.push((ZERO, len - real));
    }
    Data::blocks(entry, pieces)
}

/// Sets loop points the way vgmstream accepts them (bad ones are dropped, not clamped).
pub(crate) fn vgm_loop(t: Track, start: i64, end: i64) -> Track {
    if start >= 0 && end > start && end as u64 <= t.samples { t.looped(start as u64, end as u64) } else { t }
}

/// A sub-file [base, base + limit) of a file, read like a vgmstream STREAMFILE: bytes
/// past its end (or the file's) don't exist.
pub(crate) struct Sub<'a> {
    pub r: &'a mut Reader,
    pub base: u64,
    pub limit: u64,
    cache_off: u64,
    cache: Vec<u8>,
}

impl<'a> Sub<'a> {
    pub fn new(r: &'a mut Reader, base: u64, limit: u64) -> Sub<'a> {
        let limit = limit.min(r.size.saturating_sub(base));
        Sub { r, base, limit, cache_off: 0, cache: Vec::new() }
    }

    /// Reads up to `buf.len()` bytes at `off`; returns how many exist.
    pub fn read(&mut self, off: u64, buf: &mut [u8]) -> io::Result<usize> {
        buf.fill(0);
        if off >= self.limit {
            return Ok(0);
        }
        let n = ((self.limit - off) as usize).min(buf.len());
        if off >= self.cache_off && off + n as u64 <= self.cache_off + self.cache.len() as u64 {
            let s = (off - self.cache_off) as usize;
            buf[..n].copy_from_slice(&self.cache[s..s + n]);
            return Ok(n);
        }
        if n <= 0x1000 {
            let c = ((self.limit - off) as usize).min(0x10000);
            self.cache.resize(c, 0);
            self.cache_off = off;
            self.r.read_at(self.base + off, &mut self.cache)?;
            buf[..n].copy_from_slice(&self.cache[..n]);
            return Ok(n);
        }
        self.r.read_at(self.base + off, &mut buf[..n])?;
        Ok(n)
    }

    pub fn u8(&mut self, off: u64) -> io::Result<Option<u8>> {
        let mut b = [0u8; 1];
        Ok((self.read(off, &mut b)? == 1).then_some(b[0]))
    }

    pub fn u16be(&mut self, off: u64) -> io::Result<Option<u16>> {
        let mut b = [0u8; 2];
        Ok((self.read(off, &mut b)? == 2).then(|| u16::from_be_bytes(b)))
    }

    pub fn u32be(&mut self, off: u64) -> io::Result<Option<u32>> {
        let mut b = [0u8; 4];
        Ok((self.read(off, &mut b)? == 4).then(|| u32::from_be_bytes(b)))
    }
}

/// vgmstream's `ps_find_loop_offsets` (`full`: `ps_find_loop_offsets_full`): loop points
/// from the PS-ADPCM loop flags, as (start, end) samples, offsets relative to `s`.
pub(crate) fn ps_find_loop(s: &mut Sub, start: u64, size: u64, channels: u64, interleave: u64, full: bool) -> io::Result<Option<(i64, i64)>> {
    if size == 0 || channels == 0 || (channels > 1 && interleave == 0) {
        return Ok(None);
    }
    let (mut num, mut ls, mut le) = (0i64, 0i64, 0i64);
    let (mut ls_found, mut le_found) = (false, false);
    let mut offset = start;
    let max = start + size;
    let mut consumed = 0u64;
    while offset < max {
        let header = s.u16be(offset)?.unwrap_or(0xffff);
        let flag = header & 0x0f;
        if flag == 0x06 && !ls_found {
            ls = num;
            ls_found = true;
        }
        if flag == 0x03 && le == 0 {
            le = num + 28;
            le_found = true;
            if channels == 1 && offset + 0x10 < max && s.u8(offset + 0x11)?.unwrap_or(0xff) & 0x0f == 0x06 {
                le = 0;
                le_found = false;
            }
            if ls_found && le_found {
                break;
            }
        }
        if flag == 0x01 && full {
            let mut buf = [0u8; 16];
            let read = s.read(offset + 0x10, &mut buf)?;
            let hdr = (header >> 8) as u8;
            if read > 0 && !matches!(buf[0], 0x00 | 0x0c | 0x3c | 0x1c) && hdr == buf[0] && buf[1] == 0x07 && buf[2..].iter().all(|&b| b == 0) {
                ls = 28;
                le = num + 28;
                ls_found = true;
                le_found = true;
                break;
            }
        }
        num += 28;
        offset += 0x10;
        consumed += 0x10;
        if consumed == interleave {
            consumed = 0;
            offset += interleave * (channels - 1);
        }
    }
    Ok((ls_found && le_found).then_some((ls, le)))
}

/// vgmstream's `ps_find_padding`: bytes of silent/empty frames at the end of the data
/// (with its buffering quirks, which decide which frames it actually looks at).
pub(crate) fn ps_find_padding(s: &mut Sub, start: u64, size: u64, channels: u64, interleave: u64, discard_empty: bool) -> io::Result<u64> {
    if size == 0 || channels == 0 || (channels > 1 && interleave == 0) {
        return Ok(0);
    }
    let skip = (interleave * (channels - 1)) as i64;
    let mut offset = (start + size) as i64 - skip;
    let min = start as i64;
    let mut read_offset = 0i64;
    let mut buf = vec![0u8; 0x8000];
    let mut buf_pos = 0i64;
    let mut padding = 0u64;
    let mut consumed = 0u64;
    while offset > min {
        if offset < read_offset || buf_pos <= 0 {
            read_offset = (offset - 0x8000).max(0);
            let bytes = s.read(read_offset as u64, &mut buf)?;
            if bytes < 16 {
                break;
            }
            buf_pos = (bytes / 16 * 16) as i64;
        }
        buf_pos -= 16;
        offset -= 16;
        if buf_pos < 0 {
            break;
        }
        let f = &buf[buf_pos as usize..buf_pos as usize + 16];
        let w = |i: usize| u32::from_be_bytes(f[i..i + 4].try_into().unwrap());
        let (f1, f2, f3, f4) = (w(0), w(4), w(8), w(12));
        let flag = ((f1 >> 16) & 0xff) as u8;
        let mut empty = f1 == 0 && f2 == 0 && f3 == 0 && f4 == 0;
        if !empty && discard_empty {
            empty = flag == 0x07
                || flag == 0x77
                || ((f1 & 0xff00ffff) == 0 && f2 == 0 && f3 == 0 && f4 == 0)
                || ((f1 & 0xff00ffff) == 0x0c000000 && f2 == 0 && f3 == 0 && f4 == 0)
                || ((f1 & 0x0000ffff) == 0x00007777 && f2 == 0x77777777 && f3 == 0x77777777 && f4 == 0x77777777);
        }
        if !empty {
            break;
        }
        padding += 16 * channels;
        consumed += 16;
        if consumed == interleave {
            consumed = 0;
            offset -= skip;
            buf_pos -= skip;
        }
    }
    Ok(padding)
}

/// A VAG sub-file as vgmstream's meta/vag.c reads it (the variants found in banks).
pub(crate) struct Vag {
    pub channels: u16,
    pub rate: u32,
    pub samples: u64,
    /// Start of each channel's data in the file.
    pub starts: Vec<u64>,
    /// Bytes of channel 0's data available in the sub-file.
    pub avail: u64,
    pub loops: Option<(i64, i64)>,
    pub name: Option<String>,
}

impl Vag {
    pub fn track(&self, entry: usize, off: u64, format: &'static str) -> Track {
        let frames = self.samples.div_ceil(28);
        let (data, il) = if self.channels == 1 {
            (Data::at(entry, self.starts[0], frames * 16), 0)
        } else {
            let pieces = self.starts.iter().map(|&s| (s, frames * 16)).collect();
            (Data::blocks(entry, pieces), frames * 16)
        };
        let t = Track::new(entry, off, format, self.channels, self.rate, self.samples, data, Codec::Psx(psx::Params::interleaved(il)));
        match self.loops {
            Some((a, b)) => vgm_loop(t, a, b),
            None => t,
        }
    }
}

/// Parses a VAGp sub-file at `base` of `size` bytes like vgmstream: standard mono VAGs,
/// THQ Australia's (size off by 0x10) and .SKX "KAudioDL" stereo ones. Other variants
/// aren't expected inside banks and return None, as do invalid ones.
pub(crate) fn vag_subfile(r: &mut Reader, base: u64, size: u64) -> io::Result<Option<Vag>> {
    let mut s = Sub::new(r, base, size);
    let file_size = s.limit;
    if file_size < 0x30 {
        return Ok(None);
    }
    let mut h = [0u8; 0x30];
    s.read(0, &mut h)?;
    if &h[0..4] != b"VAGp" {
        return Ok(None);
    }
    let be = |i: usize| u32::from_be_bytes(h[i..i + 4].try_into().unwrap());
    let version = be(0x04);
    let mut channel_size = be(0x0c) as u64;
    let rate = be(0x10);
    let start = 0x30u64;
    let (channels, interleave, loops);
    let is_vagp = |v: Option<u32>| v == Some(0x5641_4770);
    let other = is_vagp(s.u32be(0x6000)?)
        || is_vagp(s.u32be(0x1000)?)
        || (version == 0x20 && is_vagp(s.u32be(0x800)?))
        || matches!(version, 0x0200_0000 | 0x4000_0000 | 0x0002_0001 | 0x0003_0000)
        || (version == 0x04 && channel_size == file_size.wrapping_sub(0x60) && be(0x1c) != 0)
        || (version == 0x20 && s.u32be(0x30)? == Some(0x5354_4552) && s.u32be(0x34)? == Some(0x454f_5641) && s.u32be(0x38)? == Some(0x4732_4b00))
        || (version == 0x02 && be(0x24) == 0x5641_4778)
        || (version == 0x20 && channel_size == file_size.wrapping_sub(0x800) && be(0x08) == 1)
        || (version == 0x20 && be(0x08) == 0x0101_0101);
    if other {
        return Ok(None); // variants not expected inside banks
    }
    if version == 0x20 && channel_size == file_size + 0x10 {
        // THQ Australia
        channels = 1;
        interleave = 0;
        channel_size -= 0x40;
        loops = ps_find_loop(&mut s, start, channel_size, 1, 0, false)?;
    } else if version == 0x20
        && &h[0x20..0x28] == b"KAudioDL"
        && ((channel_size + 0x30) * 2 == file_size
            || (channel_size + 0x30).next_multiple_of(0x800) * 2 == file_size
            || (channel_size + 0x30).next_multiple_of(0x400) * 2 == file_size)
    {
        // .SKX stereo
        channels = 2;
        interleave = file_size / 2;
        loops = ps_find_loop(&mut s, start, channel_size, 2, interleave, false)?;
    } else {
        channels = 1;
        interleave = 0;
        let full = version == 0x20 && psx::bytes_to_samples(channel_size, 1) > 20 * rate as u64;
        loops = ps_find_loop(&mut s, start, channel_size, 1, 0, full)?;
    }
    let bad_size = channel_size > file_size / channels
        || (file_size > 0x200000 && channel_size + interleave + start < (file_size - 0x200000) / channels);
    let samples = psx::bytes_to_samples(channel_size, 1);
    if bad_size || !sane_rate(rate) || samples == 0 {
        return Ok(None);
    }
    let starts: Vec<u64> = (0..channels).map(|c| base + start + interleave * c).collect();
    let avail = file_size.saturating_sub(start + interleave * (channels - 1));
    let mut probe = vec![0u8; 0x100.min(avail as usize)];
    s.read(start, &mut probe)?;
    if !psx::plausible(&probe) {
        return Ok(None);
    }
    Ok(Some(Vag { channels: channels as u16, rate, samples, starts, avail, loops, name: label(&h[0x20..0x30]) }))
}
