//! CRI ADX: 18-byte frames (a 2-byte scale and 32 4-bit samples) with a predictor
//! derived from the stream's high-pass cutoff. Follows vgmstream's coding/adx_decoder.c,
//! including the XOR-scrambled scales of encrypted ADX (types 8 and 9).

use std::io;

use super::{FrameDecoder, Sink, Stream, clamp16, run_interleaved};
use crate::track::Track;

#[derive(Debug, Clone, Default)]
pub struct Params {
    /// Early (version 3) libraries round slightly differently.
    pub v3: bool,
    /// Encoding type 4: exponential scales.
    pub exponential: bool,
    pub coef: (i32, i32),
    /// Starting sample history per channel (from the header).
    pub hist: Vec<(i32, i32)>,
    /// Encrypted ADX: (xor start, multiplier, increment) of the key sequence.
    pub key: Option<(u16, u16, u16)>,
    /// Bytes per channel block: normally one frame (0x12).
    pub interleave: u64,
}

/// ADX predictor coefficients from the high-pass cutoff, computed the way CRI's library
/// (and vgmstream) do, in single precision.
pub fn coefs(cutoff: u16, sample_rate: u32) -> (i32, i32) {
    let (x, y) = (cutoff as f32, sample_rate as f32);
    let z = ((2.0 * std::f64::consts::PI * x as f64 / y as f64) as f32).cos();
    let a = (std::f64::consts::SQRT_2 - z as f64) as f32;
    let b = (std::f64::consts::SQRT_2 - 1.0) as f32;
    let c = (a - ((a + b) * (a - b)).sqrt()) / b;
    ((c * 8192.0) as i16 as i32, (c * c * -4096.0) as i16 as i32)
}

pub struct Adx {
    v3: bool,
    exponential: bool,
    coef: (i32, i32),
    hist: (i32, i32),
    /// Encrypted: current xor, multiplier, increment, and how many steps per frame.
    key: Option<(u16, u16, u16, usize)>,
}

impl Adx {
    fn next_key(&mut self) {
        if let Some((x, m, a, steps)) = self.key.as_mut() {
            for _ in 0..*steps {
                *x = ((*x as u32 * *m as u32 + *a as u32) & 0x7fff) as u16;
            }
        }
    }
}

impl FrameDecoder for Adx {
    fn frame_bytes(&self) -> usize {
        18
    }
    fn frame_samples(&self) -> usize {
        32
    }
    fn decode(&mut self, frame: &[u8], out: &mut [i16]) {
        let raw = i16::from_be_bytes([frame[0], frame[1]]) as i32;
        let scale = if let Some((x, ..)) = self.key {
            ((raw ^ x as i32) & 0x1fff) + 1
        } else if self.exponential {
            1i32.checked_shl((12 - raw).clamp(0, 31) as u32).unwrap_or(0)
        } else if frame[0] == 0x80 && frame[1] == 0x01 {
            0 // end-of-stream marker
        } else {
            raw + 1
        };
        let (c1, c2) = self.coef;
        for (i, o) in out.iter_mut().take(32).enumerate() {
            let b = frame[2 + i / 2];
            let nibble = if i & 1 == 0 { (b as i8) >> 4 } else { ((b << 4) as i8) >> 4 } as i32;
            let s = if self.v3 {
                nibble * scale + ((c1 * self.hist.0) >> 12) + ((c2 * self.hist.1) >> 12)
            } else {
                nibble * scale + ((c1 * self.hist.0 + c2 * self.hist.1) >> 12)
            };
            let s = clamp16(s);
            *o = s;
            self.hist.1 = self.hist.0;
            self.hist.0 = s as i32;
        }
        self.next_key();
    }
}

pub fn decode(track: &Track, s: &mut Stream, p: &Params, sink: Sink) -> io::Result<()> {
    let ch = track.channels.max(1) as usize;
    let decoders = (0..ch)
        .map(|c| {
            let mut d = Adx {
                v3: p.v3,
                exponential: p.exponential,
                coef: p.coef,
                hist: p.hist.get(c).copied().unwrap_or_default(),
                key: p.key.map(|(x, m, a)| (x, m, a, 1)),
            };
            // Channel c's key starts c steps in, then moves `ch` steps per frame.
            for _ in 0..c {
                d.next_key();
            }
            if let Some(k) = d.key.as_mut() {
                k.3 = ch;
            }
            d
        })
        .collect();
    run_interleaved(s, decoders, p.interleave.max(18), 0, track.samples, sink)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_coefs() {
        // the usual 500 Hz cutoff (checked against vgmstream's output)
        assert_eq!(coefs(500, 44100), (7334, -3283));
        assert_eq!(coefs(500, 48000), (7400, -3342));
    }
}
