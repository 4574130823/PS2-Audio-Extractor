//! EA-XA ADPCM (Electronic Arts' CD-XA descendant, vgmstream coding/ea_xa_decoder.c), and
//! a small block-layout engine for EA's (and similar) containers.
//!
//! EA streams keep their audio in blocks, each giving its sample count and where every
//! channel's data starts inside it (not a fixed interleave), so a track here is a list of
//! `Block`s over the track's data (`Stream` positions). Each channel decodes the block's
//! samples from its own start, carrying its ADPCM history from block to block, like
//! vgmstream's blocked layouts. Banks with no blocks are a single block.
//!
//! The engine also runs the other codecs those containers use (PS-ADPCM, PCM), so they
//! share the same layouts.

use std::io;
use std::sync::Arc;

use super::{Sink, Stream, clamp16};
use crate::track::Track;

/// Which decoder, named like vgmstream's coding types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    /// coding_EA_XA: v1, mono, or stereo sharing one 0x1e frame (high nibbles left).
    #[default]
    EaXa,
    /// coding_EA_XA_int: v1, one 0x0f frame stream per channel.
    EaXaInt,
    /// coding_EA_XA_V2: 0x0f ADPCM frames or 0x3d PCM frames (0xEE marker).
    EaXaV2,
    /// PS-ADPCM, 0x10 frames per channel.
    Psx,
    /// 16-bit PCM per channel (samples contiguous).
    Pcm16 { big_endian: bool },
    /// 16-bit PCM with samples interleaved one by one (coding_PCM16_int).
    Pcm16Int { big_endian: bool },
    /// 16-bit PCM interleaved one by one within groups of `group` channels (layered streams).
    Pcm16Group { big_endian: bool, group: u16 },
    /// Signed 8-bit PCM per channel.
    Pcm8,
    /// Signed 8-bit PCM interleaved one by one (coding_PCM8_int).
    Pcm8Int,
    /// Unsigned 8-bit PCM interleaved one by one (coding_PCM8_U_int).
    Pcm8UInt,
}

/// A piece of the layout: `samples` per channel, decoded from each channel's start.
#[derive(Debug, Clone)]
pub struct Block {
    pub samples: u32,
    /// Where each channel's data starts, as a position in the track's data (none: silence).
    pub starts: Vec<u64>,
    /// Clear the decoders' history first (a new, independent segment starts here).
    pub reset: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Params {
    pub kind: Kind,
    pub blocks: Arc<Vec<Block>>,
}

const TABLE: [i32; 20] = [0, 240, 460, 392, 0, 0, -208, -220, 0, 1, 3, 4, 7, 8, 10, 11, 0, -1, -3, -4];

/// Bytes read around a channel's position (one per channel, so they don't thrash).
struct Cache {
    start: u64,
    data: Vec<u8>,
}

impl Cache {
    fn get(&mut self, s: &mut Stream, pos: u64, len: usize) -> io::Result<&[u8]> {
        if pos < self.start || pos + len as u64 > self.start + self.data.len() as u64 {
            self.start = pos;
            self.data = s.bytes(pos, len.max(0x8000))?;
        }
        let at = (pos - self.start) as usize;
        Ok(&self.data[at..at + len])
    }
}

#[derive(Default, Clone, Copy)]
struct Hist {
    h1: i32,
    h2: i32,
}

/// EA-XA v1 samples `first..first+n` of a frame (decode_ea_xa). `f` is the frame (0x0f or
/// 0x1e bytes); `stereo_ch` is Some(channel) for the shared stereo frame.
fn xa_v1(f: &[u8], stereo_ch: Option<usize>, h: &mut Hist, first: usize, n: usize, out: &mut Vec<i16>) {
    let (coef1, coef2, shift) = match stereo_ch {
        Some(ch) => {
            let hn = ch == 0;
            let c = if hn { f[0] >> 4 } else { f[0] & 0x0f } as usize;
            let s = if hn { f[1] >> 4 } else { f[1] & 0x0f } as i32;
            (TABLE[c], TABLE[c + 4], s + 8)
        }
        None => {
            let c = (f[0] >> 4) as usize;
            (TABLE[c], TABLE[c + 4], (f[0] & 0x0f) as i32 + 8)
        }
    };
    for i in first..first + n {
        let nibble = match stereo_ch {
            Some(ch) => {
                let b = f[2 + i];
                if ch == 0 { b >> 4 } else { b & 0x0f }
            }
            None => {
                let b = f[1 + i / 2];
                if i & 1 == 0 { b >> 4 } else { b & 0x0f }
            }
        };
        let mut s = ((nibble as i32) << 28) >> shift;
        s = (s + coef1 * h.h1 + coef2 * h.h2 + 128) >> 8;
        let s = clamp16(s) as i32;
        out.push(s as i16);
        h.h2 = h.h1;
        h.h1 = s;
    }
}

/// EA-XA v2 (decode_ea_xa_v2): ADPCM frame or 0xEE PCM frame. Returns the frame size.
fn xa_v2(f: &[u8], h: &mut Hist, first: usize, n: usize, out: &mut Vec<i16>) -> usize {
    if f[0] == 0xEE {
        h.h1 = i16::from_be_bytes([f[1], f[2]]) as i32;
        h.h2 = i16::from_be_bytes([f[3], f[4]]) as i32;
        for i in first..first + n {
            out.push(i16::from_be_bytes([f[5 + i * 2], f[6 + i * 2]]));
        }
        0x3d
    } else {
        let c = (f[0] >> 4) as usize;
        let (coef1, coef2, shift) = (TABLE[c], TABLE[c + 4], (f[0] & 0x0f) as i32 + 8);
        for i in first..first + n {
            let b = f[1 + i / 2];
            let nibble = if i & 1 == 0 { b >> 4 } else { b & 0x0f };
            let mut s = ((nibble as i32) << 28) >> shift;
            s = (s + coef1 * h.h1 + coef2 * h.h2) >> 8;
            let s = clamp16(s) as i32;
            out.push(s as i16);
            h.h2 = h.h1;
            h.h1 = s;
        }
        0x0f
    }
}

/// Decodes `n` samples of one channel starting at `pos`, returning them.
fn channel(kind: Kind, cache: &mut Cache, s: &mut Stream, mut pos: u64, ch: usize, channels: usize, h: &mut Hist, n: usize) -> io::Result<Vec<i16>> {
    let mut out = Vec::with_capacity(n);
    match kind {
        Kind::EaXa | Kind::EaXaInt | Kind::Psx => {
            let stereo = kind == Kind::EaXa && channels > 1;
            let (fb, fs) = match kind {
                Kind::Psx => (16usize, 28usize),
                _ if stereo => (0x1e, 28),
                _ => (0x0f, 28),
            };
            let mut done = 0;
            while done < n {
                let take = (n - done).min(fs);
                let f = cache.get(s, pos, fb)?;
                if kind == Kind::Psx {
                    let mut buf = [0i16; 28];
                    let mut hist = (h.h1, h.h2);
                    psx_partial(f, &mut hist, take, &mut buf);
                    h.h1 = hist.0;
                    h.h2 = hist.1;
                    out.extend_from_slice(&buf[..take]);
                } else {
                    xa_v1(f, stereo.then_some(ch), h, 0, take, &mut out);
                }
                if take == fs {
                    pos += fb as u64;
                }
                done += take;
            }
        }
        Kind::EaXaV2 => {
            let mut done = 0;
            while done < n {
                let take = (n - done).min(28);
                let f = cache.get(s, pos, 0x3d)?;
                let size = xa_v2(f, h, 0, take, &mut out);
                if take == 28 {
                    pos += size as u64;
                }
                done += take;
            }
        }
        Kind::Pcm16 { big_endian } | Kind::Pcm16Int { big_endian } | Kind::Pcm16Group { big_endian, .. } => {
            let stride = match kind {
                Kind::Pcm16Int { .. } => 2 * channels,
                Kind::Pcm16Group { group, .. } => 2 * group as usize,
                _ => 2,
            };
            let len = (n.max(1) - 1) * stride + 2;
            let b = cache.get(s, pos, len)?;
            for i in 0..n {
                let at = i * stride;
                out.push(if big_endian { i16::from_be_bytes([b[at], b[at + 1]]) } else { i16::from_le_bytes([b[at], b[at + 1]]) });
            }
        }
        Kind::Pcm8 | Kind::Pcm8Int | Kind::Pcm8UInt => {
            let stride = if kind == Kind::Pcm8 { 1 } else { channels };
            let len = (n.max(1) - 1) * stride + 1;
            let b = cache.get(s, pos, len)?;
            for i in 0..n {
                let v = b[i * stride];
                out.push(if kind == Kind::Pcm8UInt { ((v as i16) << 8).wrapping_sub(i16::MIN) } else { (v as i8 as i16) << 8 });
            }
        }
    }
    Ok(out)
}

/// PS-ADPCM (same math as `psx::frame_into`), only the first `n` samples of a frame.
fn psx_partial(f: &[u8], hist: &mut (i32, i32), n: usize, out: &mut [i16; 28]) {
    const COEFS: [[f32; 2]; 5] = [[0.0, 0.0], [0.9375, 0.0], [1.796875, -0.8125], [1.53125, -0.859375], [1.90625, -0.9375]];
    let mut coef = (f[0] >> 4) as usize;
    let mut shift = (f[0] & 0x0f) as i32;
    if coef > 4 {
        coef = 0;
    }
    if shift > 12 {
        shift = 9;
    }
    let (c1, c2) = (COEFS[coef][0], COEFS[coef][1]);
    for (i, o) in out.iter_mut().take(n).enumerate() {
        let mut s = 0i32;
        if f[1] < 0x07 {
            let b = f[2 + i / 2];
            let nibble = if i & 1 == 1 { (b as i8) >> 4 } else { ((b << 4) as i8) >> 4 };
            s = (nibble as i32) << (20 - shift);
            s += ((c1 * hist.0 as f32 + c2 * hist.1 as f32) * 256.0) as i32;
            s >>= 8;
        }
        *o = clamp16(s);
        hist.1 = hist.0;
        hist.0 = s;
    }
}

pub fn decode(track: &Track, s: &mut Stream, p: &Params, sink: Sink) -> io::Result<()> {
    let ch = track.channels.max(1) as usize;
    let mut hist = vec![Hist::default(); ch];
    let mut caches: Vec<Cache> = (0..ch).map(|_| Cache { start: 0, data: vec![] }).collect();
    let mut left = track.samples;
    for b in p.blocks.iter() {
        if left == 0 {
            break;
        }
        if b.reset {
            hist.iter_mut().for_each(|h| *h = Hist::default());
        }
        let n = (b.samples as u64).min(left) as usize;
        if n == 0 {
            continue;
        }
        if b.starts.is_empty() {
            // a silent part (segmented layouts)
            left -= n as u64;
            if !sink(&vec![0i16; n * ch])? {
                return Ok(());
            }
            continue;
        }
        let mut per = Vec::with_capacity(ch);
        for c in 0..ch {
            let start = b.starts.get(c).copied().unwrap_or(0);
            per.push(channel(p.kind, &mut caches[c], s, start, c, ch, &mut hist[c], n)?);
        }
        let mut out = vec![0i16; n * ch];
        for (c, v) in per.iter().enumerate() {
            for (i, smp) in v.iter().enumerate() {
                out[i * ch + c] = *smp;
            }
        }
        left -= n as u64;
        if !sink(&out)? {
            return Ok(());
        }
    }
    // Past the last block vgmstream has nothing more: silence.
    while left > 0 {
        let n = left.min(0x1000) as usize;
        if !sink(&vec![0i16; n * ch])? {
            return Ok(());
        }
        left -= n as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_mono_frame() {
        // coef index 0 (no prediction), shift 0x0c: nibble 1 -> (1<<28)>>20 = 256, +128 >> 8 = 1
        let mut f = [0u8; 0x0f];
        f[0] = 0x0c;
        f[1] = 0x1f;
        let mut h = Hist::default();
        let mut out = vec![];
        xa_v1(&f, None, &mut h, 0, 2, &mut out);
        assert_eq!(out, vec![1, -1]);
    }

    #[test]
    fn v2_pcm_frame() {
        let mut f = [0u8; 0x3d];
        f[0] = 0xEE;
        f[5] = 0x12;
        f[6] = 0x34;
        let mut h = Hist::default();
        let mut out = vec![];
        assert_eq!(xa_v2(&f, &mut h, 0, 1, &mut out), 0x3d);
        assert_eq!(out, vec![0x1234]);
    }
}
