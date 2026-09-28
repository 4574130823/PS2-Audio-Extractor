//! Ubisoft ADPCM (vgmstream coding/ubi_adpcm_decoder.c): 4-bit (music) and 6-bit (voices,
//! sfx) modes, mono or joint stereo, in big frames with the decoder state stored per frame.
//! The track's data starts at the 0x30 codec header.

use std::io;

use super::{Sink, Stream, clamp16};
use crate::track::Track;

#[derive(Debug, Clone, Default)]
pub struct Params {}

const CODES_MAX: usize = 1536;
const FRAME_MAX: usize = 0x34 * 2 + (CODES_MAX * 6 / 8 + 1) * 2;

/// The 0x30 codec header.
#[derive(Debug, Clone, Copy)]
pub struct Header {
    pub sample_count: u32,
    pub subframe_count: u32,
    pub codes_per_subframe_last: u32,
    pub codes_per_subframe: u32,
    pub subframes_per_frame: u32,
    pub bits_per_sample: u32,
    pub channels: u32,
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// Parses and checks the header (parse_header); `size` is the stream's size.
pub fn header(b: &[u8], size: u64) -> Option<Header> {
    if b.len() < 0x30 || le32(b, 0) != 0x08 {
        return None;
    }
    let mut h = Header {
        sample_count: le32(b, 0x04),
        subframe_count: le32(b, 0x08),
        codes_per_subframe_last: le32(b, 0x0c),
        codes_per_subframe: le32(b, 0x10),
        subframes_per_frame: le32(b, 0x14),
        bits_per_sample: le32(b, 0x24),
        channels: le32(b, 0x2c),
    };
    if h.codes_per_subframe_last as usize > CODES_MAX
        || h.codes_per_subframe as usize > CODES_MAX
        || (h.codes_per_subframe_last == 0 && h.codes_per_subframe == 0)
        || h.subframes_per_frame != 2
        || (h.bits_per_sample != 4 && h.bits_per_sample != 6)
        || !(1..=2).contains(&h.channels)
    {
        return None;
    }
    if h.sample_count == 0x77E7A374u32.wrapping_mul(h.channels) {
        fix_samples(&mut h, size);
    }
    Some(h)
}

impl Header {
    /// Samples per channel (ubi_adpcm_get_samples).
    pub fn samples(&self) -> u64 {
        (self.sample_count / self.channels) as u64
    }
}

fn fix_samples(h: &mut Header, size: u64) {
    if size == 0 {
        return;
    }
    let size = (size as u32).wrapping_sub(0x30);
    let setup = 0x34 * h.channels;
    let subframe = h.codes_per_subframe * h.bits_per_sample / 8 + 1;
    let frame = setup + subframe * h.subframes_per_frame;
    let base_frames = size.wrapping_sub(1) / frame;
    let last = size.wrapping_sub(base_frames * frame);
    let mut subframes = base_frames * h.subframes_per_frame;
    let mut samples = base_frames * (h.codes_per_subframe * h.subframes_per_frame);
    if last > setup + subframe {
        samples += h.codes_per_subframe * (h.subframes_per_frame - 1);
        subframes += h.subframes_per_frame - 1;
    }
    samples += h.codes_per_subframe_last / 2;
    subframes += 1;
    h.sample_count = samples;
    h.subframe_count = subframes;
}

const T6_1: [i32; 64] = [
    -100000000, -369, -245, -133, -33, 56, 135, 207, 275, 338, 395, 448, 499, 548, 593, 635, 676, 717, 755, 791, 825, 858, 889, 919, 948, 975, 1003,
    1029, 1054, 1078, 1103, 1132, 1800, 1800, 1800, 2048, 3072, 4096, 5000, 5056, 5184, 5240, 6144, 6880, 9624, 12880, 14952, 18040, 20480, 22920,
    25600, 28040, 32560, 35840, 40960, 45832, 51200, 56320, 63488, 67704, 75776, 89088, 102400, 0,
];
const T6_2: [i32; 64] = [
    1800, 1800, 1800, 2048, 3072, 4096, 5000, 5056, 5184, 5240, 6144, 6880, 9624, 12880, 14952, 18040, 20480, 22920, 25600, 28040, 32560, 35840,
    40960, 45832, 51200, 56320, 63488, 67704, 75776, 89088, 102400, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 3, 3, 4, 4, 5,
    5, 5, 6, 6, 6, 7,
];
const T4_1: [i32; 16] = [
    -100000000, 8, 269, 425, 545, 645, 745, 850, -1082465976, 1058977874, 1068540887, 1072986849, 1075167887, 1076761723, 1078439444, 1203982336,
];
const T4_2: [i32; 16] = [-1536, 2314, 5243, 8192, 14336, 25354, 45445, 143626, 0, 0, 0, 1, 1, 1, 3, 7];
const DELTA: [i32; 66] = [
    1024, 1031, 1053, 1076, 1099, 1123, 1148, 1172, 1198, 1224, 1251, 1278, 1306, 1334, 1363, 1393, 1423, 1454, 1485, 1518, 1551, 1584, 1619, 1654,
    1690, 1726, 1764, 1802, 1841, 1881, 1922, 1964, 2007, -1024, -1031, -1053, -1076, -1099, -1123, -1148, -1172, -1198, -1224, -1251, -1278,
    -1306, -1334, -1363, -1393, -1423, -1454, -1485, -1518, -1551, -1584, -1619, -1654, -1690, -1726, -1764, -1802, -1841, -1881, -1922, -1964,
    -2007,
];

#[derive(Default, Clone, Copy)]
struct Ch {
    step1: i32,
    next1: i32,
    next2: i32,
    coef1: i16,
    coef2: i16,
    mod1: i16,
    mod2: i16,
    mod3: i16,
    mod4: i16,
    hist1: i16,
    hist2: i16,
    delta1: i16,
    delta2: i16,
    delta3: i16,
    delta4: i16,
    delta5: i16,
}

fn read_state(b: &[u8]) -> Ch {
    let s16 = |at: usize| i16::from_le_bytes([b[at], b[at + 1]]);
    let s32 = |at: usize| i32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    Ch {
        step1: s32(0x04),
        next1: s32(0x08),
        next2: s32(0x0c),
        coef1: s16(0x10),
        coef2: s16(0x12),
        mod1: s16(0x18),
        mod2: s16(0x1a),
        mod3: s16(0x1c),
        mod4: s16(0x1e),
        hist1: s16(0x20),
        hist2: s16(0x22),
        delta1: s16(0x28),
        delta2: s16(0x2a),
        delta3: s16(0x2c),
        delta4: s16(0x2e),
        delta5: s16(0x30),
    }
}

fn clamp_val(v: i32, lo: i32, hi: i32) -> i32 {
    v.clamp(lo, hi)
}

fn sign(v: i32) -> i32 {
    if v < 0 { -1 } else { 1 }
}

fn absmax16(v: i16, absmax: i16) -> i16 {
    if v < 0 {
        if (v as i32) < -(absmax as i32) {
            return absmax.wrapping_neg();
        }
    } else if v > absmax {
        return absmax;
    }
    v
}

/// delta0 of both modes.
fn delta0(step0_next: i32, code_signed: i32) -> i32 {
    if (((step0_next as u32) & 0xFFFFFF00).wrapping_sub(1)) & 0x8000_0000 == 0 {
        let index = ((step0_next >> 3) & 0x1f) as usize + if code_signed < 0 { 33 } else { 0 };
        let shift = clamp_val((step0_next >> 8) & 0xff, 0, 31) as u32;
        DELTA[index].wrapping_shl(shift) >> 10
    } else {
        0
    }
}

fn expand6(code: u8, s: &mut Ch) -> i16 {
    let code_signed = code as i32 - 31;
    let idx = code_signed.unsigned_abs() as usize;
    let step0_next = T6_1[idx].wrapping_add(s.step1);
    let mut step0 = (s.step1 & 0xffff).wrapping_mul(246);
    step0 = step0.wrapping_add(T6_2[idx]) >> 8;
    step0 = clamp_val(step0, 271, 2560);
    let d0 = delta0(step0_next, code_signed);
    let sample = (d0.wrapping_add(s.delta1 as i32).wrapping_add(s.hist1 as i32)) as i16;
    s.hist1 = sample;
    s.step1 = step0;
    s.delta1 = d0 as i16;
    sample
}

fn expand4(code: u8, s: &mut Ch) -> i16 {
    let code_signed = code as i32 - 7;
    let idx = code_signed.unsigned_abs() as usize;
    let step0_next = T4_1[idx].wrapping_add(s.step1);
    let mut step0 = (s.step1 & 0xffff).wrapping_mul(246);
    step0 = step0.wrapping_add(T4_2[idx]) >> 8;
    step0 = clamp_val(step0, 271, 2560);
    let mut d0 = delta0(step0_next, code_signed);

    let next0 = ((s.mod1 as i32 * s.delta1 as i32)
        .wrapping_add(s.mod2 as i32 * s.delta2 as i32)
        .wrapping_add(s.mod3 as i32 * s.delta3 as i32)
        .wrapping_add(s.mod4 as i32 * s.delta4 as i32)
        >> 10) as i16 as i32;
    let mut sample = (s.coef1 as i32 * s.hist1 as i32).wrapping_add(s.coef2 as i32 * s.hist2 as i32) >> 10;
    sample = d0.wrapping_add(next0).wrapping_add(sample) as i16 as i32;

    let mut coef1_next = s.coef1 as i32 * 255;
    let mut coef2_next = s.coef2 as i32 * 254;
    d0 = d0 as i16 as i32;
    if d0 + next0 != 0 {
        let sign1 = sign(d0 + next0) * sign(s.delta1 as i32 + s.next1);
        let sign2 = sign(d0 + next0) * sign(s.delta2 as i32 + s.next2);
        let mut coef_delta = ((((sign1 * 3072) + coef1_next) >> 6) & !0x3) as i16 as i32;
        coef_delta = clamp16(clamp16(coef_delta + 30719) as i32 - 30719) as i32;
        coef_delta = clamp16(clamp16(coef_delta - 30720) as i32 + 30720) as i32;
        coef_delta = (((sign2 * 1024) as i16 as i32) - ((sign1 * coef_delta) as i16 as i32)) * 2;
        coef1_next += sign1 * 3072;
        coef2_next += coef_delta;
    }
    s.hist2 = s.hist1;
    s.hist1 = sample as i16;
    s.coef2 = absmax16((coef2_next >> 8) as i16, 768);
    s.coef1 = absmax16((coef1_next >> 8) as i16, (960 - s.coef2 as i32) as i16);
    s.next2 = s.next1;
    s.next1 = next0;
    s.step1 = step0;
    s.delta5 = s.delta4;
    s.delta4 = s.delta3;
    s.delta3 = s.delta2;
    s.delta2 = s.delta1;
    s.delta1 = d0 as i16;
    let sg = |v: i16| if v < 0 { -1 } else { 1 };
    s.mod4 = (clamp16(s.mod4 as i32 * 255 + 2048 * sg(s.delta1) * sg(s.delta5)) as i32 >> 8) as i16;
    s.mod3 = (clamp16(s.mod3 as i32 * 255 + 2048 * sg(s.delta1) * sg(s.delta4)) as i32 >> 8) as i16;
    s.mod2 = (clamp16(s.mod2 as i32 * 255 + 2048 * sg(s.delta1) * sg(s.delta3)) as i32 >> 8) as i16;
    s.mod1 = (clamp16(s.mod1 as i32 * 255 + 2048 * sg(s.delta1) * sg(s.delta2)) as i32 >> 8) as i16;
    sample as i16
}

fn unpack(data: &[u8], codes: &mut [u8], count: usize, bps: u32) {
    let mut pos = 0;
    let mut bits = 0u32;
    let mut input = 0u64;
    let mask = if bps == 6 { 0x3f } else { 0x0f };
    for c in codes.iter_mut().take(count) {
        if bits < bps {
            input = (input << 32) | le32(data, pos) as u64;
            pos += 4;
            bits += 32;
        }
        bits -= bps;
        *c = ((input >> bits) & mask) as u8;
    }
}

/// Decoder state kept between frames, like vgmstream's (buffers are reused as-is).
struct Dec {
    h: Header,
    ch: [Ch; 2],
    frame: Vec<u8>,
    codes: Vec<u8>,
    samples: Vec<i16>,
}

impl Dec {
    fn expand(&self, code: u8, s: &mut Ch) -> i16 {
        if self.h.bits_per_sample == 6 { expand6(code, s) } else { expand4(code, s) }
    }

    fn subframe(&mut self, data_at: usize, out_at: usize, count: usize) {
        let bps = self.h.bits_per_sample;
        let mut codes = std::mem::take(&mut self.codes);
        unpack(&self.frame[data_at..], &mut codes, count, bps);
        if self.h.channels == 1 {
            let mut st = self.ch[0];
            for i in 0..count {
                self.samples[out_at + i] = self.expand(codes[i], &mut st);
            }
            self.ch[0] = st;
        } else {
            let (mut a, mut b) = (self.ch[0], self.ch[1]);
            let mut i = 0;
            while i < count {
                let o = out_at + i;
                for k in 0..4 {
                    self.samples[o + k] = self.expand(codes[i + 2 * k], &mut a);
                }
                for k in 0..4 {
                    self.samples[o + 4 + k] = self.expand(codes[i + 2 * k + 1], &mut b);
                }
                i += 8;
            }
            self.ch = [a, b];
            let mut i = 0;
            while i < count {
                let o = out_at + i;
                let old: [i16; 8] = self.samples[o..o + 8].try_into().unwrap();
                for k in 0..4 {
                    self.samples[o + 2 * k] = clamp16(old[k] as i32 + old[4 + k] as i32);
                    self.samples[o + 2 * k + 1] = clamp16(old[k] as i32 - old[4 + k] as i32);
                }
                i += 8;
            }
        }
        self.codes = codes;
    }
}

pub fn decode(track: &Track, s: &mut Stream, _p: &Params, sink: Sink) -> io::Result<()> {
    let len = s.len();
    let hb = s.bytes(0, 0x30)?;
    let Some(h) = header(&hb, len) else { return Err(io::Error::other("bad Ubi ADPCM header")) };
    let channels = h.channels as usize;
    let mut d = Dec {
        h,
        ch: [Ch::default(); 2],
        frame: vec![0u8; FRAME_MAX + 16],
        codes: vec![0u8; CODES_MAX + 16],
        samples: vec![0i16; CODES_MAX * 2 + 16],
    };
    let mut offset = 0x30u64;
    let mut subframe_number = 0u32;
    let mut left = track.samples;
    let bps = h.bits_per_sample;
    while left > 0 {
        let (ca, cb) = if subframe_number + 1 == h.subframe_count {
            (h.codes_per_subframe_last, 0)
        } else if subframe_number + 2 == h.subframe_count {
            (h.codes_per_subframe, h.codes_per_subframe_last)
        } else {
            (h.codes_per_subframe, h.codes_per_subframe)
        };
        let size_a = if bps * ca / 8 > 0 { bps * ca / 8 + 1 } else { 0 } as usize;
        let size_b = if bps * cb / 8 > 0 { bps * cb / 8 + 1 } else { 0 } as usize;
        let frame_size = 0x34 * channels + size_a + size_b;
        // Only the bytes that exist are replaced (the rest keeps the previous frame's).
        let avail = (len.saturating_sub(offset) as usize).min(frame_size);
        if avail > 0 {
            let b = s.bytes(offset, avail)?;
            d.frame[..avail].copy_from_slice(&b);
        }
        d.ch[0] = read_state(&d.frame[0..0x34]);
        let data_at = 0x34 * channels;
        if channels == 2 {
            d.ch[1] = read_state(&d.frame[0x34..0x68]);
        }
        d.subframe(data_at, 0, ca as usize);
        d.subframe(data_at + size_a, ca as usize, cb as usize);
        offset += frame_size as u64;
        subframe_number += 2;
        let filled = (ca + cb) as usize / channels;
        if filled == 0 {
            break;
        }
        let n = (filled as u64).min(left) as usize;
        if !sink(&d.samples[..n * channels])? {
            return Ok(());
        }
        left -= n as u64;
    }
    if left > 0 {
        let z = vec![0i16; left as usize * channels];
        sink(&z)?;
    }
    Ok(())
}
