//! Ongakukan ADPCM (Train Simulator). Follows vgmstream's coding/libs/ongakukan_adp_lib.c:
//! mono, one byte = two samples (high nibble first), a scale that grows or shrinks with
//! each code, and 16-bit history that wraps.

use std::io;

use super::{FrameDecoder, Sink, Stream, run_interleaved};
use crate::track::Track;

#[derive(Debug, Clone, Default)]
pub struct Params {}

const FILTER: [i32; 16] = [233, 549, 453, 375, 310, 233, 233, 233, 233, 233, 233, 233, 310, 375, 453, 549];

struct Ongakukan {
    scale: i32,
    hist: i16,
}

impl Default for Ongakukan {
    fn default() -> Self {
        Ongakukan { scale: 0x10, hist: 0 }
    }
}

impl FrameDecoder for Ongakukan {
    fn frame_bytes(&self) -> usize {
        1
    }
    fn frame_samples(&self) -> usize {
        2
    }
    fn decode(&mut self, f: &[u8], out: &mut [i16]) {
        let lo = (f[0] & 0x0f) as i32;
        let hi = (f[0] >> 4) as i32;
        let s0 = (self.hist as i32).wrapping_add((hi - 8).wrapping_mul(self.scale)) as i16;
        self.scale = self.scale.wrapping_mul(FILTER[hi as usize]) >> 8;
        let s1 = (s0 as i32).wrapping_add((lo - 8).wrapping_mul(self.scale)) as i16;
        self.scale = self.scale.wrapping_mul(FILTER[lo as usize]) >> 8;
        self.hist = s1;
        out[0] = s0;
        out[1] = s1;
    }
}

pub fn decode(track: &Track, s: &mut Stream, _p: &Params, sink: Sink) -> io::Result<()> {
    let decoders = (0..track.channels.max(1)).map(|_| Ongakukan::default()).collect();
    run_interleaved(s, decoders, 1, 0, track.samples, sink)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs() {
        let mut d = Ongakukan::default();
        let mut out = [0i16; 2];
        d.decode(&[0x88], &mut out); // both codes 8: +0, scale * 233 >> 8
        assert_eq!(out, [0, 0]);
        assert_eq!(d.scale, ((0x10 * 233) >> 8) * 233 >> 8);
        let mut d = Ongakukan::default();
        d.decode(&[0xf0], &mut out); // high 15: +7*16 = 112, scale 16*549>>8 = 34; low 0: -8*34
        assert_eq!(out, [112, 112 - 8 * 34]);
    }
}
