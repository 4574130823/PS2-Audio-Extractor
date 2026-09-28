//! PS-ADPCM ("VAG"): Sony's SPU format, used by nearly all PS2 audio. 16-byte frames: a
//! predictor/shift byte, a flag byte and 28 4-bit samples. Follows vgmstream's
//! coding/psx_decoder.c (float math, same rounding).

use std::io;

use super::{FrameDecoder, Sink, Stream, clamp16, run_interleaved};
use crate::track::Track;

#[derive(Debug, Clone, Default)]
pub struct Params {
    /// Bytes per channel block (ignored for mono).
    pub interleave: u64,
    /// Bytes skipped at the start of each channel's first block (e.g. stereo VAGs
    /// repeating the header there).
    pub first_skip: u64,
    /// Some games put garbage in the flag byte; decode those frames regardless.
    pub badflags: bool,
    /// Konami's VIG encryption: (xor for each frame's byte 0, add for its byte 2), like
    /// vgmstream's meta/vig_kces_streamfile.h.
    pub vig_key: Option<(u8, u8)>,
}

impl Params {
    pub fn interleaved(interleave: u64) -> Params {
        Params { interleave, ..Default::default() }
    }
}

const COEFS: [[f32; 2]; 5] = [[0.0, 0.0], [0.9375, 0.0], [1.796875, -0.8125], [1.53125, -0.859375], [1.90625, -0.9375]];

#[derive(Default)]
pub struct Psx {
    hist: (i32, i32),
    badflags: bool,
    key: Option<(u8, u8)>,
}

impl FrameDecoder for Psx {
    fn frame_bytes(&self) -> usize {
        16
    }
    fn frame_samples(&self) -> usize {
        28
    }
    fn decode(&mut self, frame: &[u8], out: &mut [i16]) {
        match self.key {
            Some((x, a)) => {
                let mut f = [0u8; 16];
                f.copy_from_slice(&frame[..16]);
                f[0] ^= x;
                f[2] = f[2].wrapping_add(a);
                frame_into(&f, &mut self.hist, self.badflags, out);
            }
            None => frame_into(frame, &mut self.hist, self.badflags, out),
        }
    }
    fn reset(&mut self) {
        self.hist = (0, 0);
    }
}

/// Decodes one 16-byte frame into 28 samples. `hist` is (previous, one before that).
pub fn frame_into(frame: &[u8], hist: &mut (i32, i32), badflags: bool, out: &mut [i16]) {
    let mut coef = (frame[0] >> 4) as usize;
    let mut shift = (frame[0] & 0x0f) as i32;
    let flag = if badflags { 0 } else { frame[1] };
    if coef > 4 {
        coef = 0;
    }
    if shift > 12 {
        shift = 9;
    }
    let (c1, c2) = (COEFS[coef][0], COEFS[coef][1]);
    for (i, o) in out.iter_mut().take(28).enumerate() {
        let mut s = 0i32;
        if flag < 0x07 {
            // 0x07: "discard" frame, decodes to silence
            let b = frame[2 + i / 2];
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
    let decoders = (0..track.channels.max(1)).map(|_| Psx { hist: (0, 0), badflags: p.badflags, key: p.vig_key }).collect();
    run_interleaved(s, decoders, p.interleave, p.first_skip, track.samples, sink)
}

/// Samples in `bytes` of PS-ADPCM spread over `channels`.
pub fn bytes_to_samples(bytes: u64, channels: u16) -> u64 {
    bytes / channels.max(1) as u64 / 16 * 28
}

/// Whether bytes look like PS-ADPCM: every non-empty frame has a valid predictor, shift
/// and flag. Used to reject false signature matches.
pub fn plausible(data: &[u8]) -> bool {
    let mut real = 0;
    for f in data.chunks_exact(16) {
        if f.iter().all(|&b| b == 0) {
            continue;
        }
        if f[0] >> 4 > 4 || f[0] & 0x0f > 12 || f[1] > 7 {
            return false;
        }
        real += 1;
    }
    real > 0 || data.iter().all(|&b| b == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_and_discard() {
        let mut out = [1i16; 28];
        let mut h = (0, 0);
        frame_into(&[0u8; 16], &mut h, false, &mut out);
        assert!(out.iter().all(|&s| s == 0));
        let mut f = [0x77u8; 16];
        f[0] = 0x0c;
        f[1] = 0x07;
        frame_into(&f, &mut h, false, &mut out);
        assert!(out.iter().all(|&s| s == 0));
    }

    #[test]
    fn nibble_order() {
        let mut f = [0u8; 16];
        f[0] = 0x0c;
        f[2] = 0x21;
        let (mut out, mut h) = ([0i16; 28], (0, 0));
        frame_into(&f, &mut h, false, &mut out);
        assert_eq!(&out[..3], &[1, 2, 0]);
    }

    #[test]
    fn plausibility() {
        let mut ok = vec![0u8; 64];
        ok[16] = 0x2a;
        ok[17] = 0x02;
        assert!(plausible(&ok));
        let mut bad = ok.clone();
        bad[32] = 0x9f;
        assert!(!plausible(&bad));
    }
}
