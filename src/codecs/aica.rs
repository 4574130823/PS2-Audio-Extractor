//! Yamaha AICA ADPCM (Dreamcast-style 4-bit ADPCM, some PS2 ports). Follows vgmstream's
//! coding/yamaha_decoder.c (`decode_aica`): headerless nibbles, stereo data puts both
//! channels in each byte, mono data has consecutive nibbles; either nibble may come first.

use std::io;

use super::{FrameDecoder, Sink, Stream, clamp16, run_interleaved};
use crate::track::Track;

#[derive(Debug, Clone, Default)]
pub struct Params {
    /// High nibble first (left channel in the high nibble for stereo).
    pub high_first: bool,
    /// Bytes per channel block (vgmstream's AICA_int); 0 = stereo bytes / plain mono.
    pub interleave: u64,
}

const SCALE: [i32; 16] = [230, 230, 230, 230, 307, 409, 512, 614, 230, 230, 230, 230, 307, 409, 512, 614];

#[derive(Clone)]
struct State {
    hist: i16,
    step: i32,
}

impl Default for State {
    fn default() -> Self {
        State { hist: 0, step: 0x7f }
    }
}

impl State {
    fn expand(&mut self, code: u8) -> i16 {
        let code = (code & 0x0f) as i32;
        self.hist = (self.hist as i32 * 254 / 256) as i16;
        let mut delta = (((code & 7) * 2 + 1) * self.step) >> 3;
        if delta > 32767 {
            delta = 32767;
        }
        if code & 8 != 0 {
            delta = -delta;
        }
        let sample = clamp16(self.hist as i32 + delta);
        self.step = ((self.step * SCALE[code as usize]) >> 8).clamp(0x7f, 0x6000);
        self.hist = sample;
        sample
    }
}

/// One channel's nibbles, two per byte.
struct Mono {
    st: State,
    high_first: bool,
}

impl FrameDecoder for Mono {
    fn frame_bytes(&self) -> usize {
        1
    }
    fn frame_samples(&self) -> usize {
        2
    }
    fn decode(&mut self, f: &[u8], out: &mut [i16]) {
        let (a, b) = if self.high_first { (f[0] >> 4, f[0] & 0x0f) } else { (f[0] & 0x0f, f[0] >> 4) };
        out[0] = self.st.expand(a);
        out[1] = self.st.expand(b);
    }
}

pub fn decode(track: &Track, s: &mut Stream, p: &Params, sink: Sink) -> io::Result<()> {
    let ch = track.channels.max(1) as usize;
    if ch == 1 || p.interleave > 0 {
        let decoders = (0..ch).map(|_| Mono { st: State::default(), high_first: p.high_first }).collect();
        return run_interleaved(s, decoders, p.interleave, 0, track.samples, sink);
    }
    // Stereo: one byte per sample, both channels (further channels repeat the pair).
    let mut states = vec![State::default(); ch];
    let mut left = track.samples;
    let mut pos = 0u64;
    while left > 0 && pos < s.len() {
        let n = (0x10000u64).min(left) as usize;
        let buf = s.bytes(pos, n)?;
        let mut out = Vec::with_capacity(n * ch);
        for &b in &buf {
            for (c, st) in states.iter_mut().enumerate() {
                let first = c & 1 == 0;
                let code = if first == p.high_first { b >> 4 } else { b & 0x0f };
                out.push(st.expand(code));
            }
        }
        left -= n as u64;
        pos += n as u64;
        if !sink(&out)? {
            return Ok(());
        }
    }
    Ok(())
}

/// Samples in AICA data (2 nibbles per byte, shared by the channels).
pub fn bytes_to_samples(bytes: u64, channels: u16) -> u64 {
    bytes * 2 / channels.max(1) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion() {
        let mut st = State::default();
        // step 0x7f, code 7: (15 * 127) >> 3 = 238
        assert_eq!(st.expand(7), 238);
        assert_eq!(st.step, (0x7f * 614) >> 8);
        // hist decays before adding: 238 * 254 / 256 = 236; code 8 = -(step >> 3)
        let step = st.step;
        assert_eq!(st.expand(8), 236 - (step >> 3) as i16);
        assert_eq!(st.step, (step * 230) >> 8);
        for _ in 0..8 {
            st.expand(0);
        }
        assert_eq!(st.step, 0x7f); // clamped at the minimum
    }
}
