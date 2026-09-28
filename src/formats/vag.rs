//! VAGp and its many variants: Sony's standard sample format (vgmstream meta/vag.c:
//! `init_vgmstream_vag`, `_vag_aaap`, `_vag_footer`, `_vag_evolution_games`).
//!
//! Also home to PS-ADPCM helpers other ports share, ported exactly from vgmstream's
//! coding/psx_decoder.c (`ps_find_loop_offsets*`, `ps_find_padding`) and its loop sanity
//! check (`prepare_vgmstream`).
//!
//! Standard mono VAGs are marked for vgmstream's "dual file stereo" (two files named
//! *L/*R joined into one stereo track; done in `scan::pair_dual`). Not ported: the 2 MiB
//! "bad rip" size rule (it would reject VAGs at the start of bigger files).

use std::io;

use super::{Ctx, Found, Parser, be32, label, le32, sane_rate};
use crate::codecs::{Codec, psx};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser {
    name: "VAG",
    magics: &[b"VAGp", b"VAGi", b"pGAV", b"VAG1", b"VAG2", b"AAAp"],
    magic_at: 0,
    // Evolution Games' VAGs replace the signature with spaces: found by extension only.
    exts: &["vag"],
    locate: None,
    parse,
};

// ------------------------------------------------------------------------ shared helpers

/// Sets loop points the way vgmstream accepts them: loops that don't fit the track
/// (end past the last sample, start not before end, negative start) are dropped.
pub(crate) fn vgm_loop(mut t: Track, start: i64, end: i64) -> Track {
    if start >= 0 && end > start && end as u64 <= t.samples {
        t.loop_start = Some(start as u64);
        t.loop_end = Some(end as u64);
    } else {
        t.loop_start = None;
        t.loop_end = None;
    }
    t
}

/// Reads file bytes through a small window cache (the loop/padding scanners jump around).
pub(crate) struct Cache<'c, 'a> {
    ctx: &'c mut Ctx<'a>,
    base: u64,
    buf: Vec<u8>,
}

impl<'c, 'a> Cache<'c, 'a> {
    pub(crate) fn new(ctx: &'c mut Ctx<'a>) -> Self {
        Cache { ctx, base: u64::MAX, buf: Vec::new() }
    }
    fn fill(&mut self, off: u64, len: usize) -> io::Result<()> {
        if self.base == u64::MAX || off < self.base || off + len as u64 > self.base + self.buf.len() as u64 {
            self.base = off;
            self.buf = self.ctx.bytes(off, 0x10000.max(len))?;
        }
        Ok(())
    }
    pub(crate) fn get(&mut self, off: u64, len: usize) -> io::Result<&[u8]> {
        self.fill(off, len)?;
        let s = (off - self.base) as usize;
        Ok(&self.buf[s..s + len])
    }
    pub(crate) fn u8(&mut self, off: u64) -> io::Result<u8> {
        Ok(self.get(off, 1)?[0])
    }
}

/// vgmstream's `ps_find_loop_offsets` (`full`: `ps_find_loop_offsets_full`): loop points
/// from PS-ADPCM loop flags, reading channel 0's frames from `start` (absolute).
pub(crate) fn ps_find_loop(ctx: &mut Ctx, start: u64, data_size: u64, channels: u64, interleave: u64, full: bool) -> io::Result<Option<(i64, i64)>> {
    if data_size == 0 || channels == 0 || (channels > 1 && interleave == 0) {
        return Ok(None);
    }
    // past the end of the file there are no flags to find
    let data_size = data_size.min(ctx.size().saturating_sub(start));
    let mut c = Cache::new(ctx);
    let (mut num_samples, mut loop_start, mut loop_end) = (0i64, 0i64, 0i64);
    let (mut start_found, mut end_found) = (false, false);
    let mut offset = start;
    let max_offset = start + data_size;
    let mut consumed = 0u64;
    while offset < max_offset {
        let header = u16::from_be_bytes(c.get(offset, 2)?.try_into().unwrap());
        let flag = (header & 0x0f) as u8;
        if flag == 0x06 && !start_found {
            loop_start = num_samples;
            start_found = true;
        }
        if flag == 0x03 && loop_end == 0 {
            loop_end = num_samples + 28;
            end_found = true;
            // Commandos (PS2): many loop starts and ends
            if channels == 1 && offset + 0x10 < max_offset && c.u8(offset + 0x11)? & 0x0f == 0x06 {
                loop_end = 0;
                end_found = false;
            }
            if start_found && end_found {
                break;
            }
        }
        if flag == 0x01 && full {
            let hdr = (header >> 8) as u8;
            let buf = c.get(offset + 0x10, 0x10)?.to_vec();
            if buf[0] != 0x00 && buf[0] != 0x0c && buf[0] != 0x3c && buf[0] != 0x1c && hdr == buf[0] && buf[1] == 0x07 && buf[2..].iter().all(|&b| b == 0) {
                loop_start = 28;
                loop_end = num_samples + 28;
                start_found = true;
                end_found = true;
                break;
            }
        }
        num_samples += 28;
        offset += 0x10;
        consumed += 0x10;
        if consumed == interleave {
            consumed = 0;
            offset += interleave * (channels - 1);
        }
    }
    Ok((start_found && end_found).then_some((loop_start, loop_end)))
}

/// vgmstream's `ps_find_padding`: bytes of empty frames at the end of the data.
pub(crate) fn ps_find_padding(ctx: &mut Ctx, start: u64, data_size: u64, channels: u64, interleave: u64, discard_empty: bool) -> io::Result<u64> {
    if data_size == 0 || channels == 0 || (channels > 1 && interleave == 0) {
        return Ok(0);
    }
    let mut c = Cache::new(ctx);
    let mut offset = (start + data_size).saturating_sub(interleave * (channels - 1));
    let mut padding = 0u64;
    let mut consumed = 0u64;
    while offset > start && offset >= 0x10 {
        offset -= 0x10;
        let f = c.get(offset, 0x10)?.to_vec();
        let (f1, f2, f3, f4) = (be32(&f, 0), be32(&f, 4), be32(&f, 8), be32(&f, 12));
        let flag = ((f1 >> 16) & 0xff) as u8;
        let mut empty = f1 == 0 && f2 == 0 && f3 == 0 && f4 == 0;
        if !empty && discard_empty {
            empty = flag == 0x07
                || flag == 0x77
                || ((f1 & 0xFF00FFFF) == 0 && f2 == 0 && f3 == 0 && f4 == 0)
                || ((f1 & 0xFF00FFFF) == 0x0C000000 && f2 == 0 && f3 == 0 && f4 == 0)
                || ((f1 & 0xFFFF) == 0x7777 && f2 == 0x77777777 && f3 == 0x77777777 && f4 == 0x77777777);
        }
        if !empty {
            break;
        }
        padding += 0x10 * channels;
        consumed += 0x10;
        if consumed == interleave {
            consumed = 0;
            offset = offset.saturating_sub(interleave * (channels - 1));
        }
    }
    Ok(padding)
}

/// vgmstream's `ps_check_format`: every frame in the range has a sane predictor and flag.
fn ps_check_format(ctx: &mut Ctx, offset: u64, max: u64) -> io::Result<bool> {
    let end = (offset + max).min(ctx.size());
    if end <= offset {
        return Ok(true);
    }
    let b = ctx.bytes(offset, (end - offset) as usize)?;
    Ok(b.chunks(16).all(|f| f[0] >> 4 <= 5 && f.get(1).copied().unwrap_or(0) <= 7))
}

// ------------------------------------------------------------------------ the parser

/// How the channels are laid out, in vgmstream's terms.
struct Layout {
    start: u64,
    channels: u64,
    interleave: u64,
    /// Stereo files that repeat the header in each channel's first block.
    first_skip: u64,
    /// The last block of each channel is `channel_size % interleave` long.
    interleave_last: bool,
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let id = ctx.bytes(off, 4)?;
    match &id[..] {
        b"AAAp" => return aaap(ctx, off),
        b"   \0" if off == 0 => return evolution(ctx),
        b"VAGp" => {
            // The Sims 2 spinoffs: the header is a footer, after the data.
            if let Some(f) = footer(ctx, off)? {
                return Ok(vec![f]);
            }
        }
        _ => {}
    }
    if &id[0..3] != b"VAG" && &id[1..4] != b"GAV" {
        return Ok(vec![]);
    }
    let h = ctx.bytes(off, 0x40)?;
    // What vgmstream calls the file size: everything from the header on.
    let file_size = ctx.size() - off;
    let ext = if off == 0 { ctx.ext() } else { String::new() };
    let version = be32(&h, 0x04);
    let reserved = be32(&h, 0x08);
    let mut channel_size = be32(&h, 0x0c) as u64;
    let mut rate = be32(&h, 0x10);
    let mut name_size = 0x10;
    let mut hevag = false;
    let mut loops: Option<(i64, i64)> = None;
    // Standard mono VAGs may pair with a file named like them into stereo (vgmstream's
    // allow_dual_stereo; see scan::pair_dual).
    let mut dual = false;
    // (loop flags are scanned once the header checks out)
    let mut scan: Option<(u64, u64, u64, u64, bool)> = None;

    let (l, format) = match &h[0..4] {
        b"VAG1" => {
            // Metal Gear Solid 3, Cabela's African Safari, Shamu's Deep Sea Adventures
            let mut ch = h[0x1e] as u64;
            if ch == 0 {
                ch = 1;
            } else if channel_size == file_size.wrapping_sub(0x40) {
                channel_size /= ch;
            }
            (Layout { start: 0x40, channels: ch, interleave: 0x10, first_skip: 0, interleave_last: false }, "VAG")
        }
        b"VAG2" => (Layout { start: 0x40, channels: 2, interleave: 0x800, first_skip: 0, interleave_last: false }, "VAG"),
        b"VAGi" => (Layout { start: 0x800, channels: 2, interleave: le32(&h, 0x08) as u64, first_skip: 0, interleave_last: false }, "VAG"),
        b"pGAV" => {
            if version == 0x2000_0000 && le32(&h, 0x0c) as u64 + 0x30 == file_size {
                // Army Men RTS
                channel_size = le32(&h, 0x0c) as u64;
                rate = le32(&h, 0x10);
                let mut l = Layout { start: 0x30, channels: 1, interleave: 0, first_skip: 0, interleave_last: true };
                if ctx.u32be(off + 0x8030)? == 0 && off + 0x8034 <= ctx.size() {
                    l.channels = 2;
                    l.interleave = 0x8000;
                    channel_size /= 2;
                }
                (l, "VAG")
            } else {
                // Jak II, Jak 3, Jak X: stereo repeats the header in each channel's block
                let mut il = 0;
                if paired(ctx, off, &h, 0x2000)? {
                    il = 0x2000;
                } else if paired(ctx, off, &h, 0x1000)? {
                    il = 0x1000;
                }
                let ch = if il > 0 { 2 } else { 1 };
                channel_size = le32(&h, 0x0c) as u64 / ch;
                rate = le32(&h, 0x10);
                (Layout { start: 0x30, channels: ch, interleave: il, first_skip: if il > 0 { 0x30 } else { 0 }, interleave_last: false }, "VAG")
            }
        }
        b"VAGp" => {
            let std = Layout { start: 0x30, channels: 1, interleave: 0, first_skip: 0, interleave_last: false };
            if ext == "vig" {
                // MX vs. ATV Untamed
                (Layout { start: 0x7e0, channels: 2, interleave: 0x10, ..std }, "VAG")
            } else if ext == "swag" {
                // Frantix (PSP)
                channel_size = le32(&h, 0x0c) as u64;
                rate = le32(&h, 0x10);
                let il = file_size / 2;
                scan = Some((off + 0x40, channel_size * 2, 2, il, false));
                (Layout { start: 0x40, channels: 2, interleave: il, ..std }, "VAG")
            } else if paired(ctx, off, &h, 0x6000)? {
                // The Simpsons Wrestling (PS1)
                (Layout { channels: 2, interleave: 0x6000, first_skip: 0x30, ..std }, "VAG")
            } else if paired(ctx, off, &h, 0x1000)? {
                // Shikigami no Shiro
                scan = Some((off + 0x30, channel_size * 2, 2, 0x1000, false));
                (Layout { channels: 2, interleave: 0x1000, first_skip: 0x30, ..std }, "VAG")
            } else if version == 0x20 && paired(ctx, off, &h, 0x800)? {
                // ModernGroove: Ministry of Sound Edition
                (Layout { channels: 2, interleave: 0x800, first_skip: 0x30, ..std }, "VAG")
            } else if version == 0x0200_0000 || version == 0x4000_0000 {
                // Edge of Reality engine (0x02), Killzone (0x40): little endian
                channel_size = le32(&h, 0x0c) as u64;
                rate = le32(&h, 0x10);
                if version == 0x0200_0000 {
                    if (0x20..=0x7e).contains(&h[0x30]) {
                        name_size = 0x20;
                    }
                    scan = Some((off + 0x40, channel_size, 1, 0, false));
                }
                (Layout { start: 0x40, ..std }, "VAG")
            } else if version == 0x0002_0001 || version == 0x0003_0000 {
                // vagconv2 (PS Vita/PS4): HEVAG
                hevag = true;
                let mut ch = 1;
                if be32(&h, 0x18) == 0 && be32(&h, 0x1c) & 0xFFFF_00FF == 0 && h[0x1e] < 16 {
                    ch = (h[0x1e] as u64).max(1);
                }
                channel_size /= ch;
                (Layout { channels: ch, interleave: 0x10, ..std }, "VAG")
            } else if version == 4 && channel_size == file_size.wrapping_sub(0x60) && be32(&h, 0x1c) != 0 {
                // Kingdom Hearts II
                let (ls, le) = (be32(&h, 0x14) as i32 as i64, be32(&h, 0x18) as i32 as i64);
                if le > 0 {
                    loops = Some((ls, le));
                }
                let ch = h[0x1e] as u64;
                if ch == 0 {
                    return Ok(vec![]);
                }
                channel_size /= ch;
                (Layout { start: 0x60, channels: ch, interleave: 0x10, ..std }, "VAG")
            } else if version == 0x20 && &h[0x30..0x3c] == b"STEREOVAG2K\0" {
                // The Simpsons Skateboarding
                (Layout { start: 0x800, channels: 2, interleave: 0x800, ..std }, "VAG")
            } else if version == 2 && &h[0x24..0x28] == b"VAGx" {
                // Need for Speed: Hot Pursuit 2
                let ch = be32(&h, 0x2c) as u64;
                if ch == 0 || ch > 8 {
                    return Ok(vec![]);
                }
                let total = channel_size;
                channel_size /= ch;
                // Standalone files end with the data; inside archives, assume so.
                let end = if off == 0 { file_size } else { (0x30 + total).next_multiple_of(0x10) };
                if end % 0x10 != 0 {
                    return Ok(vec![]);
                }
                let mut il = 0;
                if ch > 1 {
                    // interleave from the distance between the last two end flags
                    let mut c = Cache::new(ctx);
                    let (mut o, mut end_off) = (end, 0u64);
                    while o > 0x30 {
                        o -= 0x10;
                        if c.u8(off + o + 1)? == 0x01 {
                            if end_off == 0 {
                                end_off = o;
                            } else {
                                il = end_off - o;
                                break;
                            }
                        }
                    }
                    if il == 0 {
                        return Ok(vec![]);
                    }
                }
                (Layout { channels: ch, interleave: il, ..std }, "VAG")
            } else if version == 0x20 && channel_size == file_size.wrapping_sub(0x800) && reserved == 1 {
                // Garfield: Saving Arlene
                channel_size -= ps_find_padding(ctx, off + 0x800, channel_size, 2, 0x400, false)?;
                channel_size /= 2;
                (Layout { start: 0x800, channels: 2, interleave: 0x400, ..std }, "VAG")
            } else if version == 0x20 && reserved == 0x0101_0101 {
                // Eko Software: stereo, interleave found from channel 2's empty first frame
                let il = if ctx.u32be(off + 0x800 + 0x400)? == 0 {
                    0x400
                } else if ctx.u32be(off + 0x800 + 0x4000)? == 0 {
                    0x4000
                } else if ctx.u32be(off + 0x800 + 0x2000)? == 0 {
                    0x2000
                } else {
                    return Ok(vec![]);
                };
                channel_size /= 2;
                loops = Some((0, psx::bytes_to_samples(channel_size, 1) as i64));
                (Layout { start: 0x800, channels: 2, interleave: il, first_skip: 0, interleave_last: true }, "VAG")
            } else if version == 0x20 && channel_size == file_size + 0x10 {
                // THQ Australia (Jimmy Neutron, SpongeBob)
                channel_size -= 0x40;
                scan = Some((off + 0x30, channel_size, 1, 0, false));
                (std, "VAG")
            } else if version == 0x20
                && &h[0x20..0x28] == b"KAudioDL"
                && ((channel_size + 0x30) * 2 == file_size
                    || (channel_size + 0x30).next_multiple_of(0x800) * 2 == file_size
                    || (channel_size + 0x30).next_multiple_of(0x400) * 2 == file_size)
            {
                // NBA 06 .SKX stereo
                let il = file_size / 2;
                scan = Some((off + 0x30, channel_size, 2, il, false));
                (Layout { channels: 2, interleave: il, ..std }, "VAG")
            } else {
                // standard PS1/PS2/PS3 .vag
                let full = version == 0x20 && psx::bytes_to_samples(channel_size, 1) > 20 * rate as u64;
                scan = Some((off + 0x30, channel_size, 1, 0, full));
                dual = true;
                (std, "VAG")
            }
        }
        _ => return Ok(vec![]),
    };

    let ch = l.channels;
    if ch == 0 || ch > 16 || !sane_rate(rate) || channel_size == 0 || channel_size > file_size / ch {
        return Ok(vec![]);
    }
    if ch > 1 && (l.interleave == 0 || l.interleave > 0x10_0000 || l.interleave % 0x10 != 0) {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(channel_size, 1);
    // stereo with repeated headers: the codec skips them in each channel's first block
    let data_off = off + l.start - l.first_skip;
    if data_off >= ctx.size() {
        return Ok(vec![]);
    }
    // Exactly the bytes vgmstream's layout reads (past the end of the file reads zeros).
    let size = if ch == 1 {
        channel_size
    } else if l.first_skip > 0 {
        let rest = channel_size.saturating_sub(l.interleave - l.first_skip);
        (1 + rest.div_ceil(l.interleave)) * l.interleave * ch
    } else if l.interleave_last {
        (channel_size / l.interleave * l.interleave + channel_size % l.interleave) * ch
    } else {
        channel_size.div_ceil(l.interleave) * l.interleave * ch
    };
    let probe = ctx.bytes(data_off + l.first_skip, 0x100.min(size as usize))?;
    if !psx::plausible(&probe) {
        return Ok(vec![]);
    }
    if let Some((a, b, c, d, e)) = scan {
        loops = ps_find_loop(ctx, a, b, c, d, e)?;
    }
    let codec = Codec::Psx(psx::Params { interleave: if ch > 1 { l.interleave } else { 0 }, first_skip: l.first_skip, ..Default::default() });
    let mut t = Track::new(ctx.entry, off, format, ch as u16, rate, samples, Data::at(ctx.entry, data_off, size), codec);
    t.dual_ok = dual && off == 0 && ch == 1;
    if hevag {
        t.codec = Codec::None;
        t.note = Some("HEVAG (PS Vita/PS4 VAG) isn't supported".into());
    }
    if let Some((a, b)) = loops {
        t = vgm_loop(t, a, b);
    }
    let end = (data_off + size).min(ctx.size());
    Ok(vec![Found::new(t, end).label(label(&h[0x20..0x20 + name_size]))])
}

/// Stereo VAGs that repeat the header one interleave block in: "VAGp" (or "pGAV") there,
/// with the same size and rate (so two unrelated mono sounds aren't taken for one).
fn paired(ctx: &mut Ctx, off: u64, h: &[u8], il: u64) -> io::Result<bool> {
    if off + il + 0x14 > ctx.size() {
        return Ok(false);
    }
    let b = ctx.bytes(off + il, 0x14)?;
    Ok(b[0..4] == h[0..4] && b[0x0c..0x14] == h[0x0c..0x14])
}

/// "AAAp": Acclaim Austin Audio [The Red Star]: a VAGp header per channel.
fn aaap(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(off, 8)?;
    let interleave = u16::from_le_bytes([h[4], h[5]]) as u64;
    let channels = u16::from_le_bytes([h[6], h[7]]) as u64;
    if channels == 0 || channels > 8 || interleave == 0 || interleave % 0x10 != 0 {
        return Ok(vec![]);
    }
    for i in 0..channels {
        if !ctx.is(off + 8 + i * 0x30, b"VAGp")? {
            return Ok(vec![]);
        }
    }
    if ctx.u32be(off + 8 + 4)? != 0x20 {
        return Ok(vec![]);
    }
    let channel_size = ctx.u32be(off + 8 + 0x0c)? as u64;
    let rate = ctx.u32be(off + 8 + 0x10)?;
    let start = off + 8 + channels * 0x30;
    if !sane_rate(rate) || channel_size == 0 || start >= ctx.size() {
        return Ok(vec![]);
    }
    let size = channel_size.div_ceil(interleave) * interleave * channels;
    if !psx::plausible(&ctx.bytes(start, 0x100.min(size as usize))?) {
        return Ok(vec![]);
    }
    let samples = psx::bytes_to_samples(channel_size, 1);
    let t = Track::new(ctx.entry, off, "VAG", channels as u16, rate, samples, Data::at(ctx.entry, start, size), Codec::Psx(psx::Params::interleaved(interleave)));
    Ok(vec![Found::new(t, (start + size).min(ctx.size()))])
}

/// VAGp footer [The Sims 2: Pets, The Sims 2: Castaway]: PS-ADPCM first, a little endian
/// header in the last 0x40 bytes (the data is aligned to 0x40 before it).
fn footer(ctx: &mut Ctx, hoff: u64) -> io::Result<Option<Found>> {
    let h = ctx.bytes(hoff, 0x40)?;
    if le32(&h, 0x04) != 2 {
        return Ok(None);
    }
    let stream_size = le32(&h, 0x0c) as u64;
    let rate = le32(&h, 0x10);
    let file_size = (stream_size + 0x40).next_multiple_of(0x40);
    if stream_size == 0 || !sane_rate(rate) || hoff + 0x40 < file_size {
        return Ok(None);
    }
    let start = hoff + 0x40 - file_size;
    // A header at the very start is an Edge of Reality VAG (same version bytes); one at the
    // very end of a file is a footer; otherwise go by where the audio is.
    let at_end = hoff + 0x40 == ctx.size();
    if hoff == 0 || (!at_end && psx::plausible(&ctx.bytes(hoff + 0x40, 0x100)?)) {
        return Ok(None);
    }
    if !ps_check_format(ctx, start, 0x40)? || !psx::plausible(&ctx.bytes(start, 0x100.min(stream_size as usize))?) {
        return Ok(None);
    }
    let loops = ps_find_loop(ctx, start, stream_size, 1, 0, false)?;
    let samples = psx::bytes_to_samples(stream_size, 1);
    let mut t = Track::new(ctx.entry, start, "VAG", 1, rate, samples, Data::at(ctx.entry, start, stream_size), Codec::Psx(psx::Params::default()));
    if let Some((a, b)) = loops {
        t = vgm_loop(t, a, b);
    }
    Ok(Some(Found::new(t, hoff + 0x40).label(label(&h[0x20..0x30]))))
}

/// Evolution Games [Nickelodeon Rocket Power: Beach Bandits]: "VAGp" replaced by spaces.
fn evolution(ctx: &mut Ctx) -> io::Result<Vec<Found>> {
    let h = ctx.bytes(0, 0x30)?;
    if &h[0..4] != b"   \0" || le32(&h, 4) != 0 || &h[8..12] != b"   \0" {
        return Ok(vec![]);
    }
    let size = ctx.size();
    let mut stream_size = le32(&h, 0x0c) as u64;
    if stream_size + 0x30 != size && (stream_size + 0x30).next_multiple_of(0x80) != size {
        return Ok(vec![]);
    }
    // the last frames are garbage
    stream_size = stream_size.saturating_sub(0x20);
    let rate = if &h[0x10..0x14] == b"tpad" { 44100 } else { le32(&h, 0x10) };
    if !sane_rate(rate) || stream_size == 0 {
        return Ok(vec![]);
    }
    let t = Track::new(ctx.entry, 0, "VAG", 1, rate, psx::bytes_to_samples(stream_size, 1), Data::at(ctx.entry, 0x30, stream_size), Codec::Psx(psx::Params::default()));
    Ok(vec![Found::new(t, size).label(label(&h[0x20..0x30]))])
}
