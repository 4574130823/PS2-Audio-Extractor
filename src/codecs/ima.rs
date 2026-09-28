//! IMA/DVI ADPCM variants: 4-bit samples expanded with IMA's step table. Follows
//! vgmstream's coding/ima_decoder.c (same expansions, headers and nibble layouts):
//!
//! * `Ima` / `Dvi`: headerless nibbles (low / high nibble first), mono per channel with
//!   an interleave (vgmstream's IMA_mono / DVI_IMA_mono).
//! * `Blitz`: Blitz Games' headerless IMA (custom expansion, low nibble first).
//! * `Xbox`: Microsoft's Xbox IMA, 0x24-byte frames (hist + step header, 64 samples);
//!   stereo data mixes both channels in 0x48-byte frames. With an interleave, channel pairs
//!   take turns in blocks (vgmstream's "stereo codec" interleave).
//! * `XboxMono`: Xbox IMA frames, one channel per interleave block.
//! * `XboxMch`: multichannel Xbox IMA, all channels in 0x24*channels frames.
//! * `Rad`: Radical's IMA, 0x14*channels frames with a per-channel header.
//! * `Cd`: Crystal Dynamics' IMA, 0x24-byte mono frames (different expansion).

use std::io;

use super::{FrameDecoder, Sink, Stream, clamp16, run_interleaved};
use crate::track::Track;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Ima,
    Dvi,
    Blitz,
    Xbox,
    XboxMono,
    XboxMch,
    Rad,
    Cd,
}

#[derive(Debug, Clone, Default)]
pub struct Params {
    pub kind: Kind,
    /// Bytes per channel block (0 = none: mono, or channels mixed inside each frame).
    /// For `Xbox`, half of a stereo pair's block.
    pub interleave: u64,
}

impl Params {
    pub fn new(kind: Kind, interleave: u64) -> Params {
        Params { kind, interleave }
    }
}

const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143,
    157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411,
    1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

const INDEX: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

/// Crystal Dynamics: the step table pre-scaled (clamped to 0x1fff, times 4).
const CD_STEPS: [i32; 89] = [
    28, 32, 36, 40, 44, 48, 52, 56, 64, 68, 76, 84, 92, 100, 112, 124, 136, 148, 164, 180, 200, 220, 240, 264, 292, 320, 352, 388,
    428, 472, 520, 572, 628, 692, 760, 836, 920, 1012, 1116, 1228, 1348, 1484, 1632, 1796, 1976, 2176, 2392, 2632, 2896, 3184,
    3504, 3852, 4240, 4664, 5128, 5644, 6208, 6828, 7512, 8264, 9088, 9996, 10996, 12096, 13308, 14640, 16104, 17712, 19484,
    21432, 23576, 25936, 28528, 31380, 32764, 32764, 32764, 32764, 32764, 32764, 32764, 32764, 32764, 32764, 32764, 32764,
    32764, 32764, 32764,
];
const CD_DELTAS: [i32; 16] = [
    0x0800, 0x1800, 0x2800, 0x3800, 0x4800, 0x5800, 0x6800, 0x7800, -0x0800, -0x1800, -0x2800, -0x3800, -0x4800, -0x5800, -0x6800,
    -0x7800,
];

fn clamp_index(i: i32) -> i32 {
    i.clamp(0, 88)
}

/// Standard IMA expansion (shift+add style).
fn expand(code: u8, hist: &mut i32, index: &mut i32) {
    let code = (code & 0x0f) as i32;
    let step = STEPS[*index as usize];
    let mut delta = step >> 3;
    if code & 1 != 0 {
        delta += step >> 2;
    }
    if code & 2 != 0 {
        delta += step >> 1;
    }
    if code & 4 != 0 {
        delta += step;
    }
    if code & 8 != 0 {
        delta = -delta;
    }
    *hist = clamp16(*hist + delta) as i32;
    *index = clamp_index(*index + INDEX[code as usize]);
}

fn expand_blitz(code: u8, hist: &mut i32, index: &mut i32) {
    let code = (code & 0x0f) as i32;
    let mut step = STEPS[*index as usize];
    if step == 22385 {
        step = 22358;
    } else if step == 24623 {
        step = 24633;
    }
    let mut delta = code & 7;
    if code & 8 != 0 {
        delta = -delta;
    }
    delta = (step >> 1) + delta * step;
    *hist = hist.wrapping_add(delta); // not clamped (the game doesn't either)
    *index = clamp_index(*index + INDEX[code as usize]);
}

fn expand_cd(code: u8, hist: &mut i32, index: &mut i32) {
    let code = (code & 0x0f) as usize;
    let step = CD_STEPS[*index as usize];
    let delta = ((step * CD_DELTAS[code]) >> 16) as i16 as i32;
    *hist = clamp16(*hist + delta) as i32;
    *index = clamp_index(*index + INDEX[code]);
}

/// Headerless IMA: one byte = two samples of one channel.
struct Plain {
    kind: Kind,
    hist: i32,
    index: i32,
}

impl FrameDecoder for Plain {
    fn frame_bytes(&self) -> usize {
        1
    }
    fn frame_samples(&self) -> usize {
        2
    }
    fn decode(&mut self, frame: &[u8], out: &mut [i16]) {
        let b = frame[0];
        let (first, second) = if self.kind == Kind::Dvi { (b >> 4, b & 0x0f) } else { (b & 0x0f, b >> 4) };
        for (o, code) in out.iter_mut().zip([first, second]) {
            if self.kind == Kind::Blitz {
                expand_blitz(code, &mut self.hist, &mut self.index);
                *o = clamp16(self.hist);
            } else {
                expand(code, &mut self.hist, &mut self.index);
                *o = self.hist as i16;
            }
        }
    }
}

/// One channel of a 0x24-byte Xbox IMA frame: `hdr` is its 4-byte header, `nibble(i)`
/// the byte holding nibble i (0..62; the 64th nibble is never used).
fn xbox_channel(hdr: &[u8], byte_of: impl Fn(usize) -> u8, out: &mut [i16]) {
    let mut hist = i16::from_le_bytes([hdr[0], hdr[1]]) as i32;
    let mut index = clamp_index(hdr[2] as i8 as i32);
    out[0] = hist as i16;
    for (i, o) in out.iter_mut().enumerate().take(64).skip(1) {
        let n = i - 1;
        let b = byte_of(n);
        let code = if n & 1 == 0 { b & 0x0f } else { b >> 4 };
        expand(code, &mut hist, &mut index);
        *o = hist as i16;
    }
}

/// Xbox IMA mono frames (one channel per interleave block).
struct XboxMono;

impl FrameDecoder for XboxMono {
    fn frame_bytes(&self) -> usize {
        0x24
    }
    fn frame_samples(&self) -> usize {
        64
    }
    fn decode(&mut self, f: &[u8], out: &mut [i16]) {
        xbox_channel(&f[0..4], |n| f[4 + n / 2], out);
    }
}

/// Crystal Dynamics IMA mono frames (the first nibble is skipped: the header sample
/// stands for it).
struct Cd;

impl FrameDecoder for Cd {
    fn frame_bytes(&self) -> usize {
        0x24
    }
    fn frame_samples(&self) -> usize {
        64
    }
    fn decode(&mut self, f: &[u8], out: &mut [i16]) {
        let mut hist = i16::from_le_bytes([f[0], f[1]]) as i32;
        let mut index = clamp_index(f[2] as i32);
        out[0] = hist as i16;
        for (i, o) in out.iter_mut().enumerate().take(64).skip(1) {
            let b = f[4 + i / 2];
            let code = if i & 1 != 0 { b >> 4 } else { b & 0x0f };
            expand_cd(code, &mut hist, &mut index);
            *o = hist as i16;
        }
    }
}

/// Decodes stateless frames of `frame` bytes with `samples` samples per channel each,
/// until `total` samples per channel. `f(frame, out)` fills channel-interleaved samples.
fn run_frames(
    s: &mut Stream,
    frame: usize,
    samples: usize,
    ch: usize,
    total: u64,
    sink: Sink,
    mut f: impl FnMut(&[u8], &mut [i16]),
) -> io::Result<()> {
    let per_read = (0x40000 / frame).max(1);
    let mut left = total;
    let mut pos = 0u64;
    let mut out_frame = vec![0i16; samples * ch];
    while left > 0 && pos < s.len() {
        let buf = s.bytes(pos, per_read * frame)?;
        let mut out = Vec::with_capacity(per_read * samples * ch);
        for fr in buf.chunks_exact(frame) {
            if left == 0 {
                break;
            }
            f(fr, &mut out_frame);
            let n = (samples as u64).min(left) as usize;
            out.extend_from_slice(&out_frame[..n * ch]);
            left -= n as u64;
        }
        if !sink(&out)? {
            return Ok(());
        }
        pos += (per_read * frame) as u64;
    }
    Ok(())
}

pub fn decode(track: &Track, s: &mut Stream, p: &Params, sink: Sink) -> io::Result<()> {
    let ch = track.channels.max(1) as usize;
    let total = track.samples;
    match p.kind {
        Kind::Ima | Kind::Dvi | Kind::Blitz => {
            let decoders = (0..ch).map(|_| Plain { kind: p.kind, hist: 0, index: 0 }).collect();
            run_interleaved(s, decoders, p.interleave, 0, total, sink)
        }
        Kind::XboxMono => run_interleaved(s, (0..ch).map(|_| XboxMono).collect(), p.interleave, 0, total, sink),
        Kind::Cd => run_interleaved(s, (0..ch).map(|_| Cd).collect(), p.interleave, 0, total, sink),
        Kind::Xbox if ch == 1 => run_frames(s, 0x24, 64, 1, total, sink, |f, out| xbox_channel(&f[0..4], |n| f[4 + n / 2], out)),
        Kind::Xbox if p.interleave == 0 => {
            // Stereo frames; any further channels repeat the first two (like vgmstream).
            let mut tmp = [0i16; 64];
            run_frames(s, 0x48, 64, ch, total, sink, |f, out| {
                for c in 0..ch {
                    let side = c % 2;
                    xbox_channel(&f[4 * side..4 * side + 4], |n| f[8 + 4 * side + 8 * (n / 8) + (n % 8) / 2], &mut tmp);
                    for (i, v) in tmp.iter().enumerate() {
                        out[i * ch + c] = *v;
                    }
                }
            })
        }
        Kind::Xbox => {
            // Channel pairs take turns in blocks of 2*interleave bytes of stereo frames.
            let il = p.interleave as usize;
            let frames = il / 0x24;
            let row = il * ch;
            let samples = frames * 64;
            let mut tmp = [0i16; 64];
            run_frames(s, row, samples, ch, total, sink, |r, out| {
                for c in 0..ch {
                    let pair = c & !1;
                    let side = c % 2;
                    for k in 0..frames {
                        let at = il * pair + 0x48 * k;
                        let f = &r[at.min(row)..(at + 0x48).min(row)];
                        if f.len() < 0x48 {
                            tmp.fill(0);
                        } else {
                            xbox_channel(&f[4 * side..4 * side + 4], |n| f[8 + 4 * side + 8 * (n / 8) + (n % 8) / 2], &mut tmp);
                        }
                        for (i, v) in tmp.iter().enumerate() {
                            out[(k * 64 + i) * ch + c] = *v;
                        }
                    }
                }
            })
        }
        Kind::XboxMch => {
            let mut tmp = [0i16; 64];
            run_frames(s, 0x24 * ch, 64, ch, total, sink, |f, out| {
                for c in 0..ch {
                    xbox_channel(&f[4 * c..4 * c + 4], |n| f[4 * ch + 4 * c + 4 * ch * (n / 8) + (n % 8) / 2], &mut tmp);
                    for (i, v) in tmp.iter().enumerate() {
                        out[i * ch + c] = *v;
                    }
                }
            })
        }
        Kind::Rad => run_frames(s, 0x14 * ch, 32, ch, total, sink, |f, out| {
            for c in 0..ch {
                let mut index = clamp_index(i16::from_le_bytes([f[4 * c], f[4 * c + 1]]) as i32);
                let mut hist = i16::from_le_bytes([f[4 * c + 2], f[4 * c + 3]]) as i32;
                for i in 0..32 {
                    let b = f[4 * ch + c + i / 2 * ch];
                    let code = if i & 1 != 0 { b >> 4 } else { b & 0x0f };
                    expand(code, &mut hist, &mut index);
                    out[i * ch + c] = hist as i16;
                }
            }
        }),
    }
}

/// Samples in headerless IMA data (2 per byte).
pub fn bytes_to_samples(bytes: u64, channels: u16) -> u64 {
    bytes * 2 / channels.max(1) as u64
}

/// Samples in Xbox IMA data (0x24-byte frames per channel, 64 samples each).
pub fn xbox_bytes_to_samples(bytes: u64, channels: u16) -> u64 {
    let ch = channels.max(1) as u64;
    let align = 0x24 * ch;
    let m = bytes % align;
    (bytes / align) * (align - 4 * ch) * 2 / ch + if m > 4 * ch { (m - 4 * ch) * 2 / ch } else { 0 }
}

/// Whether bytes look like Xbox IMA frames (every header's step index is valid).
pub fn xbox_plausible(data: &[u8], channels: u16) -> bool {
    let ch = channels.max(1) as usize;
    data.chunks_exact(0x24 * ch).all(|f| (0..ch).all(|c| u16::from_le_bytes([f[4 * c + 2], f[4 * c + 3]]) <= 88))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_expansion() {
        let (mut h, mut i) = (0, 0);
        expand(0x7, &mut h, &mut i);
        assert_eq!((h, i), (11, 8)); // step 7: 7>>3 + 7>>2 + 7>>1 + 7
        expand(0xf, &mut h, &mut i);
        assert_eq!((h, i), (-19, 16)); // step 16: -(2 + 4 + 8 + 16)
        let (mut h, mut i) = (0, 88);
        expand_cd(0x7, &mut h, &mut i);
        assert_eq!(h, (32764 * 0x7800) >> 16);
    }

    #[test]
    fn xbox_frame() {
        let mut f = [0u8; 0x24];
        f[0..2].copy_from_slice(&100i16.to_le_bytes());
        f[2] = 0;
        f[4] = 0x70; // first nibble 0 (+0), second 7
        let mut out = [0i16; 64];
        xbox_channel(&f[0..4], |n| f[4 + n / 2], &mut out);
        assert_eq!(out[0], 100);
        assert_eq!(out[1], 100); // code 0: +step>>3 = 0
        assert_eq!(out[2], 111); // code 7 at step 7: 0+1+3+7
    }

    #[test]
    fn sample_counts() {
        assert_eq!(xbox_bytes_to_samples(0x48, 2), 64);
        assert_eq!(xbox_bytes_to_samples(0x24 * 3, 1), 192);
        assert_eq!(bytes_to_samples(0x100, 2), 0x100);
    }
}
