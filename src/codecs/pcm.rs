//! Plain PCM: 16-bit (little/big endian) or 8-bit (signed/unsigned).

use std::io;

use super::{FrameDecoder, Sink, Stream, run_interleaved};
use crate::track::Track;

#[derive(Debug, Clone)]
pub struct Params {
    pub bits: u8,
    pub big_endian: bool,
    /// 8-bit only: signed samples (otherwise unsigned, centered on 128).
    pub signed: bool,
    /// Bytes per channel block; 0 = samples interleaved one by one.
    pub interleave: u64,
    /// XOR applied to every data byte first (JSTM scrambles its PCM with 0x5A).
    pub xor: u8,
    /// 16-bit LE samples stored rotated right by one bit (Enthusia "LP"), per vgmstream's
    /// meta/lp_ap_lep_streamfile.h.
    pub rol1: bool,
}

impl Default for Params {
    fn default() -> Self {
        Params { bits: 16, big_endian: false, signed: true, interleave: 0, xor: 0, rol1: false }
    }
}

impl Params {
    pub fn le16(interleave: u64) -> Params {
        Params { interleave, ..Default::default() }
    }
    pub fn be16(interleave: u64) -> Params {
        Params { big_endian: true, interleave, ..Default::default() }
    }
    pub fn u8(interleave: u64) -> Params {
        Params { bits: 8, signed: false, interleave, ..Default::default() }
    }
    pub fn s8(interleave: u64) -> Params {
        Params { bits: 8, signed: true, interleave, ..Default::default() }
    }
}

struct Pcm {
    p: Params,
}

impl FrameDecoder for Pcm {
    fn frame_bytes(&self) -> usize {
        (self.p.bits / 8) as usize
    }
    fn frame_samples(&self) -> usize {
        1
    }
    fn decode(&mut self, f: &[u8], out: &mut [i16]) {
        let x = self.p.xor;
        let (b0, b1) = (f[0] ^ x, f.get(1).map_or(0, |b| b ^ x));
        out[0] = match (self.p.bits, self.p.big_endian, self.p.signed) {
            (16, false, _) if self.p.rol1 => u16::from_le_bytes([b0, b1]).rotate_left(1) as i16,
            (8, _, true) => (b0 as i8 as i16) << 8,
            (8, _, false) => ((b0 as i16) - 128) << 8,
            (_, true, _) => i16::from_be_bytes([b0, b1]),
            _ => i16::from_le_bytes([b0, b1]),
        };
    }
}

pub fn decode(track: &Track, s: &mut Stream, p: &Params, sink: Sink) -> io::Result<()> {
    let decoders = (0..track.channels.max(1)).map(|_| Pcm { p: p.clone() }).collect();
    let il = if p.interleave == 0 { (p.bits / 8) as u64 } else { p.interleave };
    run_interleaved(s, decoders, il, 0, track.samples, sink)
}

pub fn bytes_to_samples(bytes: u64, channels: u16, bits: u8) -> u64 {
    bytes / channels.max(1) as u64 / (bits as u64 / 8).max(1)
}
