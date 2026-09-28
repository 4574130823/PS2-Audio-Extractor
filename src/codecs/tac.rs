//! tri-Ace's PS2 codec (Star Ocean 3, Valkyrie Profile 2, Radiata Stories): range-coded
//! spectral codes, an IDCT-like transform and MPEG-style synthesis, originally run on the
//! PS2's VU1. A port of vgmstream's coding/tac_decoder.c + coding/libs/tac_lib.c (itself
//! from Nisto's pk3dec), with the same single-precision float operations in the same order,
//! and vgmstream's float to 16-bit conversion (truncation, then clamping).
//!
//! The VU1 ops are kept as in vgmstream (lane by lane, through pointers, so an op whose
//! output register is also an input behaves exactly the same).

#![allow(clippy::too_many_arguments)]

use std::io;
use std::ptr::addr_of_mut;

use super::{Sink, Stream};
use crate::track::Track;

#[derive(Debug, Clone, Default)]
pub struct Params {}

pub const BLOCK_SIZE: usize = 0x4E000;
pub const FRAME_SAMPLES: usize = 1024;
const CHANNELS: usize = 2;
const CODED_BANDS: usize = 27;
const CODED_COEFS: usize = 32;
const TOTAL_POINTS: usize = 32;
const SCALE_TABLE_MAX_INDEX: i16 = 511;
const RANGE_FREQUENCY_SIZE: usize = 1 << 14;
const MAX_CODES: usize = 892;

/// A VU1 register: four floats (x, y, z, w), sometimes holding ints as raw bits.
#[derive(Clone, Copy, Default)]
struct V([f32; 4]);

const X: u8 = 0x8;
const Y: u8 = 0x4;
const Z: u8 = 0x2;
const W: u8 = 0x1;
const XYZW: u8 = 0xF;
const XZ: u8 = 0xA;
const YW: u8 = 0x5;

const fn v(bits: [u32; 4]) -> V {
    V([f32::from_bits(bits[0]), f32::from_bits(bits[1]), f32::from_bits(bits[2]), f32::from_bits(bits[3])])
}

const VECTOR_VOLUME: V = V([16256.0; 4]);
const VECTOR_ZERO: V = V([0.0; 4]);
const VECTOR_ONE: V = V([1.0; 4]);
const VECTOR_ROUND: V = V([0.5; 4]);

// ---------------------------------------------------------------------- VU1 ops
// (vgmstream's tac_ops.h; lanes in x, y, z, w order, each read when it's computed)

#[inline(always)]
unsafe fn lane(p: *const V, i: usize) -> f32 {
    unsafe { (*p).0[i] }
}
#[inline(always)]
unsafe fn set(p: *mut V, i: usize, x: f32) {
    unsafe { (*p).0[i] = x }
}
#[inline(always)]
fn on(m: u8, i: usize) -> bool {
    m & (8 >> i) != 0
}

/// fd = fs + ft (bc: which lane of ft is broadcast, or None for lane by lane)
#[inline(always)]
unsafe fn add(m: u8, fd: *mut V, fs: *const V, ft: *const V, bc: Option<usize>) {
    for i in 0..4 {
        if on(m, i) {
            unsafe { set(fd, i, lane(fs, i) + lane(ft, bc.unwrap_or(i))) };
        }
    }
}
#[inline(always)]
unsafe fn sub(m: u8, fd: *mut V, fs: *const V, ft: *const V, bc: Option<usize>) {
    for i in 0..4 {
        if on(m, i) {
            unsafe { set(fd, i, lane(fs, i) - lane(ft, bc.unwrap_or(i))) };
        }
    }
}
#[inline(always)]
unsafe fn mul(m: u8, fd: *mut V, fs: *const V, ft: *const V, bc: Option<usize>) {
    for i in 0..4 {
        if on(m, i) {
            unsafe { set(fd, i, lane(fs, i) * lane(ft, bc.unwrap_or(i))) };
        }
    }
}
#[inline(always)]
unsafe fn madd(m: u8, fd: *mut V, fs: *const V, ft: *const V, bc: Option<usize>) {
    for i in 0..4 {
        if on(m, i) {
            unsafe { set(fd, i, lane(fd, i) + lane(fs, i) * lane(ft, bc.unwrap_or(i))) };
        }
    }
}
#[inline(always)]
unsafe fn msub(m: u8, fd: *mut V, fs: *const V, ft: *const V, bc: Option<usize>) {
    for i in 0..4 {
        if on(m, i) {
            unsafe { set(fd, i, lane(fd, i) - lane(fs, i) * lane(ft, bc.unwrap_or(i))) };
        }
    }
}
#[inline(always)]
unsafe fn div(m: u8, fd: *mut V, fs: *const V, ft: *const V) {
    for i in 0..4 {
        if on(m, i) {
            unsafe { set(fd, i, lane(fs, i) / lane(ft, i)) };
        }
    }
}
/// fd = fs * f
#[inline(always)]
unsafe fn fmul(m: u8, fd: *mut V, fs: *const V, f: f32) {
    for i in 0..4 {
        if on(m, i) {
            unsafe { set(fd, i, lane(fs, i) * f) };
        }
    }
}
#[inline(always)]
unsafe fn movex(m: u8, fd: *mut V, fs: *const V) {
    for i in 0..4 {
        if on(m, i) {
            unsafe { set(fd, i, lane(fs, 0)) };
        }
    }
}
#[inline(always)]
unsafe fn mr32(m: u8, ft: *mut V, fs: *const V) {
    let x = unsafe { lane(fs, 0) };
    for i in 0..4 {
        if on(m, i) {
            unsafe { set(ft, i, if i < 3 { lane(fs, i + 1) } else { x }) };
        }
    }
}
/// negates fd's lanes where fs is negative
#[inline(always)]
unsafe fn sign(m: u8, fd: *mut V, fs: *const V) {
    for i in 0..4 {
        if on(m, i) && unsafe { lane(fs, i) } < 0.0 {
            unsafe { set(fd, i, -lane(fd, i)) };
        }
    }
}

macro_rules! p {
    ($e:expr) => {
        addr_of_mut!($e)
    };
}

// ---------------------------------------------------------------------- decoding

pub struct Tac {
    // header
    frame_count: u16,
    joint_stereo: u32,
    data_start: usize,
    frame_offset: usize,
    frame_number: u32,
    // range coding
    symbol_frequency: [u16; 257],
    code_history: [[i16; 32]; CHANNELS],
    cumulative_frequency: [u16; 258],
    symbol_lookup: Vec<u8>,
    codes: [[i16; FRAME_SAMPLES]; CHANNELS],
    // vectors
    spectrum: [[V; FRAME_SAMPLES / 4]; CHANNELS],
    wave: [[V; FRAME_SAMPLES / 4]; CHANNELS],
    hist: [[V; FRAME_SAMPLES / 4]; CHANNELS],
}

/// What decoding a frame did.
#[derive(PartialEq, Debug)]
pub enum Step {
    Ok,
    NextBlock,
    Done,
    Error,
}

/// The fixed 0x20 header fields vgmstream's meta reads (and the lib validates).
pub struct Header {
    pub range_offset: u32,
    pub loop_frame: u16,
    pub loop_discard: u16,
    pub frame_count: u16,
    pub frame_last: u16,
    pub loop_offset: u32,
    pub file_size: u32,
    pub joint_stereo: u32,
}

/// `init_header`: None if the lib would refuse it.
pub fn header(b: &[u8]) -> Option<Header> {
    let u32le = |a: usize| u32::from_le_bytes(b[a..a + 4].try_into().unwrap());
    let u16le = |a: usize| u16::from_le_bytes([b[a], b[a + 1]]);
    let h = Header {
        range_offset: u32le(0),
        loop_frame: u16le(8),
        loop_discard: u16le(0x0a),
        frame_count: u16le(0x0c),
        frame_last: u16le(0x0e),
        loop_offset: u32le(0x10),
        file_size: u32le(0x14),
        joint_stereo: u32le(0x18),
    };
    let empty = u32le(0x1c);
    if h.range_offset < 0x20 || h.range_offset as usize > BLOCK_SIZE {
        return None;
    }
    if h.file_size as usize % BLOCK_SIZE != 0 {
        return None;
    }
    if h.loop_discard as usize > FRAME_SAMPLES || h.frame_last as usize + 1 > FRAME_SAMPLES {
        return None;
    }
    if h.loop_frame > h.frame_count || h.loop_offset > h.file_size {
        return None;
    }
    if (h.joint_stereo != 0 && h.joint_stereo != 1) || empty != 0 {
        return None;
    }
    Some(h)
}

impl Tac {
    /// `tac_init` from the first block.
    pub fn new(block: &[u8]) -> Option<Box<Tac>> {
        if block.len() < BLOCK_SIZE {
            return None;
        }
        let h = header(block)?;
        let mut t = Box::new(Tac {
            frame_count: h.frame_count,
            joint_stereo: h.joint_stereo,
            data_start: 0,
            frame_offset: 0,
            frame_number: 1,
            symbol_frequency: [0; 257],
            code_history: [[0; 32]; CHANNELS],
            cumulative_frequency: [0; 258],
            symbol_lookup: vec![0; RANGE_FREQUENCY_SIZE - 1],
            codes: [[0; FRAME_SAMPLES]; CHANNELS],
            spectrum: [[V::default(); FRAME_SAMPLES / 4]; CHANNELS],
            wave: [[V::default(); FRAME_SAMPLES / 4]; CHANNELS],
            hist: [[V::default(); FRAME_SAMPLES / 4]; CHANNELS],
        });
        let pos = t.init_range(&block[h.range_offset as usize..])?;
        t.data_start = h.range_offset as usize + pos;
        if t.data_start > BLOCK_SIZE {
            return None;
        }
        t.reset();
        Some(t)
    }

    /// `init_range`: the frequency model (None if it can't be used).
    fn init_range(&mut self, buf: &[u8]) -> Option<usize> {
        let mut off = 0;
        for i in 0..256 {
            let mut f = *buf.get(off)? as u16;
            off += 1;
            if f & 0x80 != 0 {
                f &= 0x7f;
                f |= (*buf.get(off)? as u16) << 7;
                off += 1;
            }
            self.symbol_frequency[i] = f;
        }
        self.symbol_frequency[256] = 1;
        self.code_history = [[0; 32]; CHANNELS];
        self.cumulative_frequency[0] = 0;
        for i in 0..257 {
            self.cumulative_frequency[i + 1] = self.cumulative_frequency[i].wrapping_add(self.symbol_frequency[i]);
        }
        let mut symbol: u8 = 0;
        let mut guard = 0;
        while self.symbol_frequency[symbol as usize] == 0 {
            symbol = symbol.wrapping_add(1);
            guard += 1;
            if guard > 256 {
                return None;
            }
        }
        for i in 0..RANGE_FREQUENCY_SIZE - 1 {
            if i >= self.cumulative_frequency[symbol as usize + 1] as usize {
                let mut guard = 0;
                loop {
                    symbol = symbol.wrapping_add(1);
                    guard += 1;
                    if self.symbol_frequency[symbol as usize] != 0 || guard > 256 {
                        break;
                    }
                }
                if guard > 256 {
                    return None;
                }
            }
            self.symbol_lookup[i] = symbol;
        }
        Some(off)
    }

    /// `tac_reset`
    pub fn reset(&mut self) {
        self.frame_offset = self.data_start;
        self.frame_number = 1;
        self.hist = [[V::default(); FRAME_SAMPLES / 4]; CHANNELS];
        self.code_history = [[0; 32]; CHANNELS];
    }

    /// `tac_decode_frame`: decodes the next frame of `block`.
    pub fn decode_frame(&mut self, block: &[u8]) -> Step {
        let pos = self.frame_offset;
        if self.frame_number > self.frame_count as u32 {
            return Step::Done;
        }
        if pos > BLOCK_SIZE - 4 {
            return Step::Error;
        }
        if u32::from_le_bytes(block[pos..pos + 4].try_into().unwrap()) == 0xFFFF_FFFF {
            self.frame_offset = 0;
            return Step::NextBlock;
        }
        if pos > BLOCK_SIZE - 0x0c {
            return Step::Error;
        }
        let buf = &block[pos..];
        let crc = u16::from_le_bytes([buf[0], buf[1]]);
        let use_hist = u16::from_le_bytes([buf[2], buf[3]]) >> 15;
        let frame_size = (u16::from_le_bytes([buf[2], buf[3]]) & 0x7fff) as usize;
        let id = u16::from_le_bytes([buf[4], buf[5]]);
        let frame_codes = u16::from_le_bytes([buf[6], buf[7]]);
        let base_code = u32::from_be_bytes(buf[8..12].try_into().unwrap());
        if id as u32 != self.frame_number {
            return Step::Error;
        }
        if pos > BLOCK_SIZE - 8 - frame_size {
            return Step::Error;
        }
        self.frame_number += 1;
        self.frame_offset += 8 + frame_size;
        if crc != crc16(&buf[4..8 + frame_size]) {
            return Step::Error;
        }
        let end = (0x0c + frame_size).min(buf.len());
        let read = self.read_codes(&buf[0x0c..end], use_hist != 0, base_code);
        if read as u16 != frame_codes {
            return Step::Error;
        }
        for ch in 0..CHANNELS {
            let codes = self.codes[ch];
            unpack_channel(&mut self.spectrum[ch], &codes);
            transform(&mut self.wave[ch], &self.spectrum[ch]);
            process(&mut self.wave[ch], &mut self.hist[ch]);
        }
        if self.joint_stereo != 0 {
            let (l, r) = self.wave.split_at_mut(1);
            for i in 0..TOTAL_POINTS * 8 {
                let (a, b) = (l[0][i], r[0][i]);
                let mut sl = V::default();
                let mut sr = V::default();
                unsafe {
                    add(XYZW, p!(sl), &a, &b, None);
                    sub(XYZW, p!(sr), &a, &b, None);
                }
                l[0][i] = sl;
                r[0][i] = sr;
            }
        }
        Step::Ok
    }

    /// `read_codes`: range-decodes the codes of both channels; how many were read (or -1).
    fn read_codes(&mut self, data: &[u8], use_hist: bool, base_code: u32) -> i32 {
        if data.is_empty() {
            return -1;
        }
        let mut p = 0usize;
        let mut total = 0i32;
        let mut code = base_code;
        let mut lower = 0u32;
        let mut range = 0xFFFF_FFFFu32;
        for ch in 0..CHANNELS {
            let mut max_codes = 28;
            let mut index = 0;
            while index < max_codes {
                range >>= 14;
                if range == 0 {
                    return -1;
                }
                let lookup = (code.wrapping_sub(lower) / range) as usize;
                if lookup >= RANGE_FREQUENCY_SIZE - 1 {
                    return -1;
                }
                let mut symbol = self.symbol_lookup[lookup] as u32;
                lower = lower.wrapping_add((self.cumulative_frequency[symbol as usize] as u32).wrapping_mul(range));
                range = range.wrapping_mul(self.symbol_frequency[symbol as usize] as u32);
                macro_rules! normalize {
                    () => {
                        while 0xFFFFFF >= (lower ^ lower.wrapping_add(range)) {
                            if p >= data.len() {
                                return -1;
                            }
                            code = (code << 8) | data[p] as u32;
                            p += 1;
                            range <<= 8;
                            lower <<= 8;
                        }
                        while 0xFFFF >= range {
                            if p >= data.len() {
                                return -1;
                            }
                            code = (code << 8) | data[p] as u32;
                            p += 1;
                            range = ((!lower).wrapping_add(1) & 0xFFFF) << 8;
                            lower <<= 8;
                        }
                    };
                }
                normalize!();
                if symbol >= 0xFE {
                    let extended = symbol == 0xFE;
                    range >>= if extended { 8 } else { 13 };
                    if range == 0 {
                        return -1;
                    }
                    symbol = code.wrapping_sub(lower) / range;
                    lower = lower.wrapping_add(symbol.wrapping_mul(range));
                    if extended {
                        symbol = symbol.wrapping_add(0xFE);
                    }
                    normalize!();
                }
                let mut value: i16 = if symbol & 1 != 0 { (-((symbol as i16 as i32 + 1) / 2)) as i16 } else { (symbol / 2) as i16 };
                if index < 28 {
                    if use_hist {
                        value = value.wrapping_add(self.code_history[ch][index]);
                    }
                    self.code_history[ch][index] = value;
                    if index != 0 && value != 0 {
                        max_codes += 32;
                    }
                }
                if index < FRAME_SAMPLES {
                    self.codes[ch][index] = value;
                }
                index += 1;
            }
            total += index as i32;
            for i in index..MAX_CODES {
                self.codes[ch][i] = 0;
            }
        }
        total
    }

    /// Samples of the last decoded frame as vgmstream outputs them: float, then truncated
    /// and clamped to 16 bits. Interleaved L, R.
    pub fn samples(&self, out: &mut [i16]) {
        for ch in 0..CHANNELS {
            for i in 0..FRAME_SAMPLES / 4 {
                for k in 0..4 {
                    let f = self.wave[ch][i].0[k];
                    out[(i * 4 + k) * CHANNELS + ch] = (f as i32).clamp(-32768, 32767) as i16;
                }
            }
        }
    }
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc ^ 0xFFFF
}

fn unpack_antialias(spectrum: &mut [V; FRAME_SAMPLES / 4]) {
    let at = &ANTIALIASING_TABLE;
    let mut pos_lo = 7usize;
    let mut pos_hi = 8usize;
    for _ in 0..CODED_BANDS {
        for j in 0..4 {
            let lo_in = spectrum[pos_lo - j];
            let hi_in = spectrum[pos_hi + j];
            let mut lo_out = V::default();
            let mut hi_out = V::default();
            unsafe {
                mul(W, p!(lo_out), &lo_in, &at[j], Some(0));
                msub(W, p!(lo_out), &at[0xF - j], &hi_in, Some(0));
                mul(X, p!(hi_out), &hi_in, &at[7 - j], Some(3));
                madd(X, p!(hi_out), &at[8 + j], &lo_in, Some(3));

                mul(Z, p!(lo_out), &lo_in, &at[j], Some(1));
                msub(Z, p!(lo_out), &at[0xF - j], &hi_in, Some(1));
                mul(Y, p!(hi_out), &hi_in, &at[7 - j], Some(2));
                madd(Y, p!(hi_out), &at[8 + j], &lo_in, Some(2));

                mul(Y, p!(lo_out), &lo_in, &at[j], Some(2));
                msub(Y, p!(lo_out), &at[0xF - j], &hi_in, Some(2));
                mul(Z, p!(hi_out), &hi_in, &at[7 - j], Some(1));
                madd(Z, p!(hi_out), &at[8 + j], &lo_in, Some(1));

                mul(X, p!(lo_out), &lo_in, &at[j], Some(3));
                msub(X, p!(lo_out), &at[0xF - j], &hi_in, Some(3));
                mul(W, p!(hi_out), &hi_in, &at[7 - j], Some(0));
                madd(W, p!(hi_out), &at[8 + j], &lo_in, Some(0));
            }
            spectrum[pos_lo - j] = lo_out;
            spectrum[pos_hi + j] = hi_out;
        }
        pos_lo += 8;
        pos_hi += 8;
    }
}

fn clamp_s16(v: i16, min: i16, max: i16) -> i16 {
    v.clamp(min, max)
}

fn unpack_code4(spectrum: &mut [V; FRAME_SAMPLES / 4], spc1: &V, spc2: &V, code: &V, idx: &[i32; 4], out_pos: usize) {
    let st = &SCALE_TABLE;
    let mut tbc1 = V::default();
    let mut tbc2 = V::default();
    let mut out = V::default();
    let masks = [X, Y, Z, W];
    for k in 0..4 {
        unsafe {
            if code.0[k] != 0.0 {
                movex(masks[k], p!(tbc1), &st[idx[k] as usize]);
                movex(masks[k], p!(tbc2), &st[idx[k] as usize + 1]);
            } else {
                movex(masks[k], p!(tbc1), &VECTOR_ZERO);
                movex(masks[k], p!(tbc2), &VECTOR_ZERO);
            }
        }
    }
    unsafe {
        div(XYZW, p!(out), spc1, spc2);
        sub(XYZW, p!(tbc2), p!(tbc2), p!(tbc1), None);
        mul(XYZW, p!(out), p!(out), p!(tbc2), None);
        add(XYZW, p!(out), p!(out), p!(tbc1), None);
        sign(XYZW, p!(out), code);
    }
    spectrum[out_pos] = out;
}

fn unpack_band(spectrum: &mut [V; FRAME_SAMPLES / 4], codes: &[i16; FRAME_SAMPLES], band_pos: usize, code_pos: &mut usize, out_pos: usize) {
    let st = &SCALE_TABLE;
    let base_index = clamp_s16(codes[0], 0, SCALE_TABLE_MAX_INDEX);
    let band_index = clamp_s16(codes[band_pos], 0, SCALE_TABLE_MAX_INDEX - 128);
    if band_index == 0 {
        for i in 0..CODED_COEFS / 4 {
            spectrum[out_pos + i] = VECTOR_ZERO;
        }
        return;
    }
    let mut scale = V::default();
    unsafe { mul(Y, p!(scale), &st[128 + band_index as usize], &st[base_index as usize], Some(1)) };
    let f512 = 512.0f64 as f32;
    let f_inv = 0.00195313f64 as f32;
    for i in 0..8 {
        let mut code = V::default();
        for k in 0..4 {
            code.0[k] = codes.get(*code_pos + k).copied().unwrap_or(0) as f32;
        }
        *code_pos += 4;
        let mut tm01 = V::default();
        let mut tm02 = V::default();
        let mut tm03 = V::default();
        let mut spc1 = V::default();
        let mut spc2 = V::default();
        let mut idx = [0i32; 4];
        unsafe {
            for k in 0..4 {
                tm01.0[k] = code.0[k].abs();
            }
            mul(XYZW, p!(tm01), p!(tm01), &scale, Some(1));
            fmul(XYZW, p!(tm02), &tm01, f512);
            add(XYZW, p!(tm03), &tm02, &VECTOR_ONE, None);
            // FTOI0 / ITOF0
            for k in 0..4 {
                idx[k] = tm02.0[k] as i32;
                tm02.0[k] = idx[k] as f32;
            }
            fmul(XYZW, p!(tm02), p!(tm02), f_inv);
            for k in 0..4 {
                tm03.0[k] = (tm03.0[k] as i32) as f32;
            }
            fmul(XYZW, p!(tm03), p!(tm03), f_inv);
            sub(XYZW, p!(spc1), &tm01, &tm02, None);
            sub(XYZW, p!(spc2), &tm03, &tm02, None);
        }
        for k in 0..4 {
            idx[k] = clamp_s16(idx[k] as i16, 0, SCALE_TABLE_MAX_INDEX) as i32;
        }
        unpack_code4(spectrum, &spc1, &spc2, &code, &idx, out_pos + i);
    }
}

fn unpack_channel(spectrum: &mut [V; FRAME_SAMPLES / 4], codes: &[i16; FRAME_SAMPLES]) {
    let mut code_pos = CODED_BANDS + 1;
    let mut out_pos = 0;
    for i in 1..CODED_BANDS + 1 {
        unpack_band(spectrum, codes, i, &mut code_pos, out_pos);
        out_pos += CODED_COEFS / 4;
    }
    for s in spectrum.iter_mut().take(0x100).skip(0xD8) {
        *s = VECTOR_ZERO;
    }
    unpack_antialias(spectrum);
}

/// `transform_dot_product`: the MUL + 7 MADDs, summed pairwise as vgmstream-cli's MSVC
/// /fp:fast build does (read from its machine code).
fn dot(mac: &mut V, spectrum: &[V; FRAME_SAMPLES / 4], tt: &[V], pi: usize, pt: usize) {
    for l in 0..4 {
        let p: [f32; 8] = std::array::from_fn(|k| spectrum[pi + k].0[l] * tt[pt + k].0[l]);
        mac.0[l] = ((p[0] + p[1]) + (p[2] + p[3])) + ((p[4] + p[5]) + (p[6] + p[7]));
    }
}

fn transform(wave: &mut [V; FRAME_SAMPLES / 4], spectrum: &[V; FRAME_SAMPLES / 4]) {
    let tt = &TRANSFORM_TABLE;
    let mut pos_t = 0;
    let mut pos_o = 0;
    for _ in 0..TOTAL_POINTS {
        let mut pos_i = 0;
        for _ in 0..8 {
            let mut mac = V::default();
            let mut ror = V::default();
            let mut out = V::default();
            unsafe {
                dot(&mut mac, spectrum, tt, pos_i, pos_t);
                pos_i += 8;
                mr32(XYZW, p!(ror), &mac);
                add(XZ, p!(ror), p!(ror), &mac, None);
                add(X, p!(out), &ror, &ror, Some(2));

                dot(&mut mac, spectrum, tt, pos_i, pos_t);
                pos_i += 8;
                mr32(XYZW, p!(ror), &mac);
                add(YW, p!(ror), p!(ror), &mac, None);
                add(Y, p!(out), &ror, &ror, Some(3));

                dot(&mut mac, spectrum, tt, pos_i, pos_t);
                pos_i += 8;
                mr32(XYZW, p!(ror), &mac);
                add(XZ, p!(ror), p!(ror), &mac, None);
                add(Z, p!(out), &ror, &ror, Some(0));

                dot(&mut mac, spectrum, tt, pos_i, pos_t);
                pos_i += 8;
                mr32(XYZW, p!(ror), &mac);
                add(YW, p!(ror), p!(ror), &mac, None);
                add(W, p!(out), &ror, &ror, Some(1));

                fmul(XYZW, p!(out), p!(out), 0.25);
            }
            wave[pos_o] = out;
            pos_o += 1;
        }
        pos_t += 8;
    }
}

// ---------------------------------------------------------------------- stream

pub fn decode(track: &Track, s: &mut Stream, _p: &Params, sink: Sink) -> io::Result<()> {
    let mut block = vec![0u8; BLOCK_SIZE];
    let len = s.len();
    let first = (BLOCK_SIZE as u64).min(len) as usize;
    s.read(0, &mut block[..first])?;
    let Some(mut tac) = Tac::new(&block[..first]) else {
        return Err(io::Error::other("TAC: bad header"));
    };
    let mut offset = first as u64;
    let mut left = track.samples;
    let mut out = vec![0i16; FRAME_SAMPLES * CHANNELS];
    while left > 0 {
        match tac.decode_frame(&block) {
            Step::Ok => {
                tac.samples(&mut out);
                let n = (FRAME_SAMPLES as u64).min(left) as usize;
                left -= n as u64;
                if !sink(&out[..n * CHANNELS])? {
                    return Ok(());
                }
            }
            Step::NextBlock => {
                // the next block replaces this one (near the end only in part)
                if offset >= len {
                    return Ok(());
                }
                let n = (BLOCK_SIZE as u64).min(len - offset) as usize;
                s.read(offset, &mut block[..n])?;
                offset += n as u64;
            }
            Step::Done | Step::Error => return Ok(()),
        }
    }
    Ok(())
}

/// Window/overlap synthesis, like MP3's (vgmstream's `process`). The butterfly part is
/// vgmstream's long list of VU ops as the MSVC /fp:fast build of vgmstream-cli (r2117)
/// evaluates it (some sums regrouped): each value below was read from that build's machine
/// code, so the rounding is the same. `w[k][lane]` is the input, `hist` gets 16 outputs.
#[allow(clippy::neg_multiply)]
fn process(wave: &mut [V; FRAME_SAMPLES / 4], hist: &mut [V; FRAME_SAMPLES / 4]) {
    let mut pos_o = 0usize;
    let mut pos_w = 0usize;
    let mut pos_r: i32 = 0x200;
    for _ in 0..TOTAL_POINTS {
        let mut pos_h = (pos_r & 0xFF) as usize;
        pos_r -= 0x10;
        let w: [[f32; 4]; 8] = std::array::from_fn(|k| wave[pos_w + k].0);
        pos_w += 8;
        let t0: f32 = w[0][0] + w[7][3];
        let t1: f32 = w[3][3] + w[4][0];
        let t2: f32 = t0 + t1;
        let t3: f32 = w[1][3] + w[2][0];
        let t4: f32 = w[5][3] + w[6][0];
        let t5: f32 = t3 + t4;
        let t6: f32 = t2 + t5;
        let t7: f32 = w[0][3] + w[1][0];
        let t8: f32 = w[6][3] + w[7][0];
        let t9: f32 = t7 + t8;
        let t10: f32 = w[2][3] + w[3][0];
        let t11: f32 = w[4][3] + w[5][0];
        let t12: f32 = t10 + t11;
        let t13: f32 = t9 + t12;
        let t14: f32 = t6 + t13;
        let t15: f32 = w[0][1] + w[0][2];
        let t16: f32 = w[7][1] + w[7][2];
        let t17: f32 = t15 + t16;
        let t18: f32 = w[3][1] + w[3][2];
        let t19: f32 = w[4][1] + w[4][2];
        let t20: f32 = t18 + t19;
        let t21: f32 = t17 + t20;
        let t22: f32 = w[1][1] + w[1][2];
        let t23: f32 = w[2][1] + w[2][2];
        let t24: f32 = t22 + t23;
        let t25: f32 = w[5][1] + w[5][2];
        let t26: f32 = w[6][1] + w[6][2];
        let t27: f32 = t25 + t26;
        let t28: f32 = t24 + t27;
        let t29: f32 = t21 + t28;
        let t30: f32 = t14 - t29;
        let t31: f32 = t30 * f32::from_bits(0x3F3504F2);
        let t32: f32 = w[0][0] - w[7][3];
        let t33: f32 = t3 - t4;
        let t34: f32 = t33 * f32::from_bits(0x3F3504F2);
        let t35: f32 = t32 + t34;
        let t36: f32 = t7 - t8;
        let t37: f32 = t7 + t10;
        let t38: f32 = t11 + t8;
        let t39: f32 = t37 - t38;
        let t40: f32 = t39 * f32::from_bits(0x3F3504F2);
        let t41: f32 = t36 + t40;
        let t42: f32 = t41 * f32::from_bits(0x3F0A8BD4);
        let t43: f32 = t35 + t42;
        let t44: f32 = t15 - t16;
        let t45: f32 = t24 - t27;
        let t46: f32 = t45 * f32::from_bits(0x3F3504F2);
        let t47: f32 = t44 + t46;
        let t48: f32 = t15 + t22;
        let t49: f32 = t26 + t16;
        let t50: f32 = t48 - t49;
        let t51: f32 = t23 + t18;
        let t52: f32 = t48 + t51;
        let t53: f32 = t19 + t25;
        let t54: f32 = t53 + t49;
        let t55: f32 = t52 - t54;
        let t56: f32 = t55 * f32::from_bits(0x3F3504F2);
        let t57: f32 = t50 + t56;
        let t58: f32 = t57 * f32::from_bits(0x3F0A8BD4);
        let t59: f32 = t47 + t58;
        let t60: f32 = t59 * f32::from_bits(0x3F0281F6);
        let t61: f32 = t43 - t60;
        let t62: f32 = w[3][2] + w[3][3];
        let t63: f32 = w[4][0] + w[4][1];
        let t64: f32 = t62 - t63;
        let t65: f32 = w[1][2] + w[1][3];
        let t66: f32 = w[2][0] + w[2][1];
        let t67: f32 = t65 - t66;
        let t68: f32 = w[5][2] + w[5][3];
        let t69: f32 = w[6][0] + w[6][1];
        let t70: f32 = t68 - t69;
        let t71: f32 = t67 + t70;
        let t72: f32 = t71 * f32::from_bits(0x3F3504F2);
        let t73: f32 = t64 + t72;
        let t74: f32 = w[2][2] + w[2][3];
        let t75: f32 = w[3][0] + w[3][1];
        let t76: f32 = t74 - t75;
        let t77: f32 = w[4][2] + w[4][3];
        let t78: f32 = w[5][0] + w[5][1];
        let t79: f32 = t77 - t78;
        let t80: f32 = t76 + t79;
        let t81: f32 = w[0][2] + w[0][3];
        let t82: f32 = w[1][0] + w[1][1];
        let t83: f32 = t81 - t82;
        let t84: f32 = t83 + t76;
        let t85: f32 = w[6][2] + w[6][3];
        let t86: f32 = w[7][0] + w[7][1];
        let t87: f32 = t85 - t86;
        let t88: f32 = t79 + t87;
        let t89: f32 = t84 + t88;
        let t90: f32 = t89 * f32::from_bits(0x3F3504F2);
        let t91: f32 = t80 + t90;
        let t92: f32 = t91 * f32::from_bits(0x3F0A8BD4);
        let t93: f32 = t73 + t92;
        let t94: f32 = t75 - t62;
        let t95: f32 = t63 - t77;
        let t96: f32 = t94 + t95;
        let t97: f32 = t82 - t65;
        let t98: f32 = t66 - t74;
        let t99: f32 = t97 + t98;
        let t100: f32 = t78 - t68;
        let t101: f32 = t69 - t85;
        let t102: f32 = t100 + t101;
        let t103: f32 = t99 + t102;
        let t104: f32 = t103 * f32::from_bits(0x3F3504F2);
        let t105: f32 = t96 + t104;
        let t106: f32 = t98 + t94;
        let t107: f32 = t95 + t100;
        let t108: f32 = t106 + t107;
        let t109: f32 = w[0][0] + w[0][1];
        let t110: f32 = t109 - t81;
        let t111: f32 = t110 + t97;
        let t112: f32 = t111 + t106;
        let t113: f32 = w[7][2] + w[7][3];
        let t114: f32 = t86 - t113;
        let t115: f32 = t101 + t114;
        let t116: f32 = t107 + t115;
        let t117: f32 = t112 + t116;
        let t118: f32 = t117 * f32::from_bits(0x3F3504F2);
        let t119: f32 = t108 + t118;
        let t120: f32 = t119 * f32::from_bits(0x3F0A8BD4);
        let t121: f32 = t105 + t120;
        let t122: f32 = t121 * f32::from_bits(0x3F0281F6);
        let t123: f32 = t93 - t122;
        let t124: f32 = t123 * f32::from_bits(0x3F009E8D);
        let t125: f32 = t61 + t124;
        let t126: f32 = t125 * f32::from_bits(0x3F3E99ED);
        let t127: f32 = t0 - t1;
        let t128: f32 = t9 - t12;
        let t129: f32 = t128 * f32::from_bits(0x3F3504F2);
        let t130: f32 = t127 + t129;
        let t131: f32 = t17 - t20;
        let t132: f32 = t48 + t49;
        let t133: f32 = t51 + t53;
        let t134: f32 = t132 - t133;
        let t135: f32 = t134 * f32::from_bits(0x3F3504F2);
        let t136: f32 = t131 + t135;
        let t137: f32 = t136 * f32::from_bits(0x3F0A8BD4);
        let t138: f32 = t130 - t137;
        let t139: f32 = t67 - t70;
        let t140: f32 = t84 - t88;
        let t141: f32 = t140 * f32::from_bits(0x3F3504F2);
        let t142: f32 = t139 + t141;
        let t143: f32 = t99 - t102;
        let t144: f32 = t112 - t116;
        let t145: f32 = t144 * f32::from_bits(0x3F3504F2);
        let t146: f32 = t143 + t145;
        let t147: f32 = t146 * f32::from_bits(0x3F0A8BD4);
        let t148: f32 = t142 - t147;
        let t149: f32 = t148 * f32::from_bits(0x3F0281F6);
        let t150: f32 = t138 + t149;
        let t151: f32 = t150 * f32::from_bits(0x3F49C480);
        let t152: f32 = t32 - t34;
        let t153: f32 = t36 - t40;
        let t154: f32 = t153 * f32::from_bits(0x3FA73D74);
        let t155: f32 = t152 + t154;
        let t156: f32 = t44 - t46;
        let t157: f32 = t50 - t56;
        let t158: f32 = t157 * f32::from_bits(0x3FA73D74);
        let t159: f32 = t156 + t158;
        let t160: f32 = t159 * f32::from_bits(0x3F19F1BD);
        let t161: f32 = t155 - t160;
        let t162: f32 = t64 - t72;
        let t163: f32 = t80 - t90;
        let t164: f32 = t163 * f32::from_bits(0x3FA73D74);
        let t165: f32 = t162 + t164;
        let t166: f32 = t96 - t104;
        let t167: f32 = t108 - t118;
        let t168: f32 = t167 * f32::from_bits(0x3FA73D74);
        let t169: f32 = t166 + t168;
        let t170: f32 = t169 * f32::from_bits(0x3F19F1BD);
        let t171: f32 = t165 - t170;
        let t172: f32 = t171 * f32::from_bits(0x3F05C278);
        let t173: f32 = t161 - t172;
        let t174: f32 = t173 * f32::from_bits(0x3F56DF9E);
        let t175: f32 = t2 - t5;
        let t176: f32 = t21 - t28;
        let t177: f32 = t176 * f32::from_bits(0x3F3504F2);
        let t178: f32 = t175 - t177;
        let t179: f32 = t83 + t79;
        let t180: f32 = t179 - t76;
        let t181: f32 = t180 - t87;
        let t182: f32 = t111 - t106;
        let t183: f32 = t182 + t107;
        let t184: f32 = t183 - t115;
        let t185: f32 = t184 * f32::from_bits(0x3F3504F2);
        let t186: f32 = t181 - t185;
        let t187: f32 = t186 * f32::from_bits(0x3F0A8BD4);
        let t188: f32 = t178 + t187;
        let t189: f32 = t188 * f32::from_bits(0x3F6664D7);
        let t190: f32 = t152 - t154;
        let t191: f32 = t156 - t158;
        let t192: f32 = t191 * f32::from_bits(0x3F6664D7);
        let t193: f32 = t190 - t192;
        let t194: f32 = t162 - t164;
        let t195: f32 = t166 - t168;
        let t196: f32 = t195 * f32::from_bits(0x3F6664D7);
        let t197: f32 = t194 - t196;
        let t198: f32 = t197 * f32::from_bits(0x3F11233F);
        let t199: f32 = t193 + t198;
        let t200: f32 = t199 * f32::from_bits(0x3F78FA3B);
        let t201: f32 = t127 - t129;
        let t202: f32 = t131 - t135;
        let t203: f32 = t202 * f32::from_bits(0x3FA73D74);
        let t204: f32 = t201 - t203;
        let t205: f32 = t139 - t141;
        let t206: f32 = t143 - t145;
        let t207: f32 = t206 * f32::from_bits(0x3FA73D74);
        let t208: f32 = t205 - t207;
        let t209: f32 = t208 * f32::from_bits(0x3F19F1BD);
        let t210: f32 = t204 - t209;
        let t211: f32 = t210 * f32::from_bits(0x3F87C449);
        let t212: f32 = t35 - t42;
        let t213: f32 = t47 - t58;
        let t214: f32 = t213 * f32::from_bits(0x402406CE);
        let t215: f32 = t212 - t214;
        let t216: f32 = t73 - t92;
        let t217: f32 = t105 - t120;
        let t218: f32 = t217 * f32::from_bits(0x402406CE);
        let t219: f32 = t216 - t218;
        let t220: f32 = t219 * f32::from_bits(0x3F25961C);
        let t221: f32 = t215 - t220;
        let t222: f32 = t221 * f32::from_bits(0x3F95B034);
        let t223: f32 = t6 - t13;
        let t224: f32 = t110 - t97;
        let t225: f32 = t98 - t94;
        let t226: f32 = t224 + t225;
        let t227: f32 = t95 - t100;
        let t228: f32 = t101 - t114;
        let t229: f32 = t227 + t228;
        let t230: f32 = t226 + t229;
        let t231: f32 = t230 * f32::from_bits(0x3F3504F2);
        let t232: f32 = t223 - t231;
        let t233: f32 = t232 * f32::from_bits(0x3FA73D74);
        let t234: f32 = t212 + t214;
        let t235: f32 = t216 * f32::from_bits(0x3F49C480);
        let t236: f32 = f32::from_bits(0x3F49C480) * t218;
        let t237: f32 = t235 + t236;
        let t238: f32 = t234 + t237;
        let t239: f32 = t238 * f32::from_bits(0x3FBDF91B);
        let t240: f32 = t201 + t203;
        let t241: f32 = t205 + t207;
        let t242: f32 = t241 * f32::from_bits(0x3F6664D7);
        let t243: f32 = t240 + t242;
        let t244: f32 = t243 * f32::from_bits(0x3FDC7925);
        let t245: f32 = t190 + t192;
        let t246: f32 = t194 + t196;
        let t247: f32 = t246 * f32::from_bits(0x3F87C449);
        let t248: f32 = t245 - t247;
        let t249: f32 = t248 * f32::from_bits(0x4003B2AF);
        let t250: f32 = t175 + t177;
        let t251: f32 = t181 + t185;
        let t252: f32 = t251 * f32::from_bits(0x3FA73D74);
        let t253: f32 = t250 - t252;
        let t254: f32 = t253 * f32::from_bits(0x402406CE);
        let t255: f32 = t155 + t160;
        let t256: f32 = t165 * f32::from_bits(0x3FDC7925);
        let t257: f32 = f32::from_bits(0x3FDC7925) * t170;
        let t258: f32 = t256 + t257;
        let t259: f32 = t255 + t258;
        let t260: f32 = t259 * f32::from_bits(0x405A1641);
        let t261: f32 = t130 + t137;
        let t262: f32 = t142 + t147;
        let t263: f32 = t262 * f32::from_bits(0x402406CE);
        let t264: f32 = t261 - t263;
        let t265: f32 = t264 * f32::from_bits(0x40A33C9C);
        let t266: f32 = t43 + t60;
        let t267: f32 = t93 + t122;
        let t268: f32 = t267 * f32::from_bits(0x40A33C9C);
        let t269: f32 = t266 - t268;
        let t270: f32 = t269 * f32::from_bits(0x41230A46);
        let t271: f32 = t61 - t124;
        let t272: f32 = t271 * f32::from_bits(0x3F2CC03D);
        let t273: f32 = t138 - t149;
        let t274: f32 = t273 * f32::from_bits(0x3F25961C);
        let t275: f32 = t161 + t172;
        let t276: f32 = t275 * f32::from_bits(0x3F1F5C6E);
        let t277: f32 = t178 - t187;
        let t278: f32 = t277 * f32::from_bits(0x3F19F1BD);
        let t279: f32 = t193 - t198;
        let t280: f32 = t279 * f32::from_bits(0x3F153B3A);
        let t281: f32 = t204 + t209;
        let t282: f32 = t281 * f32::from_bits(0x3F11233F);
        let t283: f32 = t215 + t220;
        let t284: f32 = t283 * f32::from_bits(0x3F0D9837);
        let t285: f32 = t223 + t231;
        let t286: f32 = t285 * f32::from_bits(0x3F0A8BD4);
        let t287: f32 = t234 - t237;
        let t288: f32 = t287 * f32::from_bits(0x3F07F268);
        let t289: f32 = t240 - t242;
        let t290: f32 = t289 * f32::from_bits(0x3F05C278);
        let t291: f32 = t245 + t247;
        let t292: f32 = t291 * f32::from_bits(0x3F03F45A);
        let t293: f32 = t250 + t252;
        let t294: f32 = t293 * f32::from_bits(0x3F0281F6);
        let t295: f32 = t255 - t258;
        let t296: f32 = t295 * f32::from_bits(0x3F01668B);
        let t297: f32 = t261 + t263;
        let t298: f32 = t297 * f32::from_bits(0x3F009E8D);
        let t299: f32 = t266 + t268;
        let t300: f32 = t299 * f32::from_bits(0x3F002785);
        let t301: f32 = t14 + t29;
        hist[pos_h + 0x0].0[0] = t31;
        hist[pos_h + 0x0].0[1] = t126;
        hist[pos_h + 0x0].0[2] = t151;
        hist[pos_h + 0x0].0[3] = t174;
        hist[pos_h + 0x1].0[0] = t189;
        hist[pos_h + 0x1].0[1] = t200;
        hist[pos_h + 0x1].0[2] = t211;
        hist[pos_h + 0x1].0[3] = t222;
        hist[pos_h + 0x2].0[0] = t233;
        hist[pos_h + 0x2].0[1] = t239;
        hist[pos_h + 0x2].0[2] = t244;
        hist[pos_h + 0x2].0[3] = t249;
        hist[pos_h + 0x3].0[0] = t254;
        hist[pos_h + 0x3].0[1] = t260;
        hist[pos_h + 0x3].0[2] = t265;
        hist[pos_h + 0x3].0[3] = t270;
        hist[pos_h + 0x4].0[0] = f32::from_bits(0x00000000);
        hist[pos_h + 0x4].0[1] = -t270;
        hist[pos_h + 0x4].0[2] = -t265;
        hist[pos_h + 0x4].0[3] = -t260;
        hist[pos_h + 0x5].0[0] = -t254;
        hist[pos_h + 0x5].0[1] = -t249;
        hist[pos_h + 0x5].0[2] = -t244;
        hist[pos_h + 0x5].0[3] = -t239;
        hist[pos_h + 0x6].0[0] = -t233;
        hist[pos_h + 0x6].0[1] = -t222;
        hist[pos_h + 0x6].0[2] = -t211;
        hist[pos_h + 0x6].0[3] = -t200;
        hist[pos_h + 0x7].0[0] = -t189;
        hist[pos_h + 0x7].0[1] = -t174;
        hist[pos_h + 0x7].0[2] = -t151;
        hist[pos_h + 0x7].0[3] = -t126;
        hist[pos_h + 0x8].0[0] = -t31;
        hist[pos_h + 0x8].0[1] = -t272;
        hist[pos_h + 0x8].0[2] = -t274;
        hist[pos_h + 0x8].0[3] = -t276;
        hist[pos_h + 0x9].0[0] = -t278;
        hist[pos_h + 0x9].0[1] = -t280;
        hist[pos_h + 0x9].0[2] = -t282;
        hist[pos_h + 0x9].0[3] = -t284;
        hist[pos_h + 0xA].0[0] = -t286;
        hist[pos_h + 0xA].0[1] = -t288;
        hist[pos_h + 0xA].0[2] = -t290;
        hist[pos_h + 0xA].0[3] = -t292;
        hist[pos_h + 0xB].0[0] = -t294;
        hist[pos_h + 0xB].0[1] = -t296;
        hist[pos_h + 0xB].0[2] = -t298;
        hist[pos_h + 0xB].0[3] = -t300;
        hist[pos_h + 0xC].0[0] = -t301;
        hist[pos_h + 0xC].0[1] = -t300;
        hist[pos_h + 0xC].0[2] = -t298;
        hist[pos_h + 0xC].0[3] = -t296;
        hist[pos_h + 0xD].0[0] = -t294;
        hist[pos_h + 0xD].0[1] = -t292;
        hist[pos_h + 0xD].0[2] = -t290;
        hist[pos_h + 0xD].0[3] = -t288;
        hist[pos_h + 0xE].0[0] = -t286;
        hist[pos_h + 0xE].0[1] = -t284;
        hist[pos_h + 0xE].0[2] = -t282;
        hist[pos_h + 0xE].0[3] = -t280;
        hist[pos_h + 0xF].0[0] = -t278;
        hist[pos_h + 0xF].0[1] = -t276;
        hist[pos_h + 0xF].0[2] = -t274;
        hist[pos_h + 0xF].0[3] = -t272;
        let wt = &WINDOW_TABLE;
        for j in 0..8 {
            let mut o = V::default();
            let mut rnd = VECTOR_ROUND;
            // the build sums the 16 products in this order (same for all 4 lanes)
            const OFFS: [usize; 16] = [0x00, 0x18, 0x20, 0x38, 0x40, 0x58, 0x60, 0x78, 0x80, 0x98, 0xA0, 0xB8, 0xC0, 0xD8, 0xE0, 0xF8];
            for l in 0..4 {
                let t: [f32; 16] = std::array::from_fn(|k| hist[(pos_h + OFFS[k]) & 0xFF].0[l] * wt[0x08 * k + j].0[l]);
                let mut sum = ((((((t[3] + t[4]) + t[5]) + ((t[0] + t[1]) + t[2])) + t[6]) + t[7]) + t[8]) + (t[9] + t[10]);
                sum += ((t[11] + t[12]) + t[13]) + (t[14] + t[15]);
                o.0[l] = sum;
            }
            unsafe {
                mul(XYZW, p!(o), p!(o), &VECTOR_VOLUME, None);
                sign(XYZW, p!(rnd), &o);
                add(XYZW, p!(o), p!(o), &rnd, None);
            }
            pos_h += 1;
            wave[pos_o] = o;
            pos_o += 1;
        }
    }
}

const ANTIALIASING_TABLE: [V; 16] = [
    v([0x3F5B84A8, 0x3F5E682F, 0x3F61B9D7, 0x3F6B2DE7]),
    v([0x3F731ADE, 0x3F77C337, 0x3F7BBA82, 0x3F7D8708]),
    v([0x3F7EDA43, 0x3F51B92F, 0x3F7FC8FC, 0x3F7B0756]),
    v([0x3F7FF966, 0x3F7FFE66, 0x3F7FFF8E, 0x3F800000]),
    v([0x3F800000, 0x3F7FFF8E, 0x3F7FFE66, 0x3F7FF966]),
    v([0x3F7B0756, 0x3F7FC8FC, 0x3F51B92F, 0x3F7EDA43]),
    v([0x3F7D8708, 0x3F7BBA82, 0x3F77C337, 0x3F731ADE]),
    v([0x3F6B2DE7, 0x3F61B9D7, 0x3F5E682F, 0x3F5B84A8]),
    v([0xBF03B5FF, 0xBEFD8B40, 0xBEF186DA, 0xBECA4114]),
    v([0xBEA07304, 0xBE80D626, 0xBE3A4775, 0xBE0DF9B3]),
    v([0xBDC1B01E, 0xBF12CE6D, 0xBD27CB87, 0xBE48D2AC]),
    v([0xBC68A11E, 0xBBE55ED3, 0xBB727B47, 0x00000000]),
    v([0x00000000, 0xBB727B47, 0xBBE55ED3, 0xBC68A11E]),
    v([0xBE48D2AC, 0xBD27CB87, 0xBF12CE6D, 0xBDC1B01E]),
    v([0xBE0DF9B3, 0xBE3A4775, 0xBE80D626, 0xBEA07304]),
    v([0xBECA4114, 0xBEF186DA, 0xBEFD8B40, 0xBF03B5FF]),
];

const SCALE_TABLE: [V; 513] = [
    v([0x00000000, 0x3F7F8000, 0x00000000, 0x3F7F8000]),
    v([0x3A000000, 0x3F5744FD, 0x3A000000, 0x3F5744FD]),
    v([0x3AA14518, 0x3F3504F3, 0x3AA14518, 0x3F3504F3]),
    v([0x3B0A74BA, 0x3F1837F0, 0x3B0A74BA, 0x3F1837F0]),
    v([0x3B4B2FF5, 0x3F000000, 0x3B4B2FF5, 0x3F000000]),
    v([0x3B88CC4F, 0x3ED744FD, 0x3B88CC4F, 0x3ED744FD]),
    v([0x3BAE718E, 0x3EB504F3, 0x3BAE718E, 0x3EB504F3]),
    v([0x3BD63F90, 0x3E9837F0, 0x3BD63F90, 0x3E9837F0]),
    v([0x3C000000, 0x3E800000, 0x3C000000, 0x3E800000]),
    v([0x3C15C41B, 0x3E5744FD, 0x3C15C41B, 0x3E5744FD]),
    v([0x3C2C5AD3, 0x3E3504F3, 0x3C2C5AD3, 0x3E3504F3]),
    v([0x3C43B5D3, 0x3E1837F0, 0x3C43B5D3, 0x3E1837F0]),
    v([0x3C5BC8FF, 0x3E000000, 0x3C5BC8FF, 0x3E000000]),
    v([0x3C7489EF, 0x3DD744FD, 0x3C7489EF, 0x3DD744FD]),
    v([0x3C86F7CD, 0x3DB504F3, 0x3C86F7CD, 0x3DB504F3]),
    v([0x3C93F904, 0x3D9837F0, 0x3C93F904, 0x3D9837F0]),
    v([0x3CA14518, 0x3D800000, 0x3CA14518, 0x3D800000]),
    v([0x3CAED8DF, 0x3D5744FD, 0x3CAED8DF, 0x3D5744FD]),
    v([0x3CBCB181, 0x3D3504F3, 0x3CBCB181, 0x3D3504F3]),
    v([0x3CCACC6C, 0x3D1837F0, 0x3CCACC6C, 0x3D1837F0]),
    v([0x3CD92746, 0x3D000000, 0x3CD92746, 0x3D000000]),
    v([0x3CE7BFE8, 0x3CD744FD, 0x3CE7BFE8, 0x3CD744FD]),
    v([0x3CF69458, 0x3CB504F3, 0x3CF69458, 0x3CB504F3]),
    v([0x3D02D161, 0x3C9837F0, 0x3D02D161, 0x3C9837F0]),
    v([0x3D0A74BA, 0x3C800000, 0x3D0A74BA, 0x3C800000]),
    v([0x3D12336D, 0x3C5744FD, 0x3D12336D, 0x3C5744FD]),
    v([0x3D1A0CBF, 0x3C3504F3, 0x3D1A0CBF, 0x3C3504F3]),
    v([0x3D220000, 0x3C1837F0, 0x3D220000, 0x3C1837F0]),
    v([0x3D2A0C8A, 0x3C000000, 0x3D2A0C8A, 0x3C000000]),
    v([0x3D3231C3, 0x3BD744FD, 0x3D3231C3, 0x3BD744FD]),
    v([0x3D3A6F17, 0x3BB504F3, 0x3D3A6F17, 0x3BB504F3]),
    v([0x3D42C3FE, 0x3B9837F0, 0x3D42C3FE, 0x3B9837F0]),
    v([0x3D4B2FF5, 0x3B800000, 0x3D4B2FF5, 0x3B800000]),
    v([0x3D53B280, 0x3B5744FD, 0x3D53B280, 0x3B5744FD]),
    v([0x3D5C4B2A, 0x3B3504F3, 0x3D5C4B2A, 0x3B3504F3]),
    v([0x3D64F982, 0x3B1837F0, 0x3D64F982, 0x3B1837F0]),
    v([0x3D6DBD20, 0x3B000000, 0x3D6DBD20, 0x3B000000]),
    v([0x3D76959C, 0x3AD744FD, 0x3D76959C, 0x3AD744FD]),
    v([0x3D7F8298, 0x3AB504F3, 0x3D7F8298, 0x3AB504F3]),
    v([0x3D8441DB, 0x3A9837F0, 0x3D8441DB, 0x3A9837F0]),
    v([0x3D88CC4F, 0x3A800000, 0x3D88CC4F, 0x3A800000]),
    v([0x3D8D607D, 0x3A5744FD, 0x3D8D607D, 0x3A5744FD]),
    v([0x3D91FE3D, 0x3A3504F3, 0x3D91FE3D, 0x3A3504F3]),
    v([0x3D96A568, 0x3A1837F0, 0x3D96A568, 0x3A1837F0]),
    v([0x3D9B55D8, 0x3A000000, 0x3D9B55D8, 0x3A000000]),
    v([0x3DA00F69, 0x39D744FD, 0x3DA00F69, 0x39D744FD]),
    v([0x3DA4D1F9, 0x39B504F3, 0x3DA4D1F9, 0x39B504F3]),
    v([0x3DA99D65, 0x399837F0, 0x3DA99D65, 0x399837F0]),
    v([0x3DAE718E, 0x39800000, 0x3DAE718E, 0x39800000]),
    v([0x3DB34E55, 0x395744FD, 0x3DB34E55, 0x395744FD]),
    v([0x3DB8339A, 0x393504F3, 0x3DB8339A, 0x393504F3]),
    v([0x3DBD2142, 0x391837F0, 0x3DBD2142, 0x391837F0]),
    v([0x3DC21730, 0x39000000, 0x3DC21730, 0x39000000]),
    v([0x3DC71549, 0x38D744FD, 0x3DC71549, 0x38D744FD]),
    v([0x3DCC1B72, 0x38B504F3, 0x3DCC1B72, 0x38B504F3]),
    v([0x3DD12992, 0x389837F0, 0x3DD12992, 0x389837F0]),
    v([0x3DD63F90, 0x38800000, 0x3DD63F90, 0x38800000]),
    v([0x3DDB5D54, 0x385744FD, 0x3DDB5D54, 0x385744FD]),
    v([0x3DE082C7, 0x383504F3, 0x3DE082C7, 0x383504F3]),
    v([0x3DE5AFD1, 0x381837F0, 0x3DE5AFD1, 0x381837F0]),
    v([0x3DEAE45E, 0x38000000, 0x3DEAE45E, 0x38000000]),
    v([0x3DF02057, 0x37D744FD, 0x3DF02057, 0x37D744FD]),
    v([0x3DF563A8, 0x37B504F3, 0x3DF563A8, 0x37B504F3]),
    v([0x3DFAAE3C, 0x379837F0, 0x3DFAAE3C, 0x379837F0]),
    v([0x3E000000, 0x37800000, 0x3E000000, 0x37800000]),
    v([0x3E02AC70, 0x375744FD, 0x3E02AC70, 0x375744FD]),
    v([0x3E055C65, 0x373504F3, 0x3E055C65, 0x373504F3]),
    v([0x3E080FD6, 0x371837F0, 0x3E080FD6, 0x371837F0]),
    v([0x3E0AC6BA, 0x37000000, 0x3E0AC6BA, 0x37000000]),
    v([0x3E0D8108, 0x36D744FD, 0x3E0D8108, 0x36D744FD]),
    v([0x3E103EB7, 0x36B504F3, 0x3E103EB7, 0x36B504F3]),
    v([0x3E12FFC0, 0x369837F0, 0x3E12FFC0, 0x369837F0]),
    v([0x3E15C41B, 0x36800000, 0x3E15C41B, 0x36800000]),
    v([0x3E188BBF, 0x365744FD, 0x3E188BBF, 0x365744FD]),
    v([0x3E1B56A5, 0x363504F3, 0x3E1B56A5, 0x363504F3]),
    v([0x3E1E24C5, 0x361837F0, 0x3E1E24C5, 0x361837F0]),
    v([0x3E20F618, 0x36000000, 0x3E20F618, 0x36000000]),
    v([0x3E23CA96, 0x35D744FD, 0x3E23CA96, 0x35D744FD]),
    v([0x3E26A239, 0x35B504F3, 0x3E26A239, 0x35B504F3]),
    v([0x3E297CFA, 0x359837F0, 0x3E297CFA, 0x359837F0]),
    v([0x3E2C5AD3, 0x35800000, 0x3E2C5AD3, 0x35800000]),
    v([0x3E2F3BBB, 0x355744FD, 0x3E2F3BBB, 0x355744FD]),
    v([0x3E321FAD, 0x353504F3, 0x3E321FAD, 0x353504F3]),
    v([0x3E3506A4, 0x351837F0, 0x3E3506A4, 0x351837F0]),
    v([0x3E37F097, 0x35000000, 0x3E37F097, 0x35000000]),
    v([0x3E3ADD82, 0x34D744FD, 0x3E3ADD82, 0x34D744FD]),
    v([0x3E3DCD5E, 0x34B504F3, 0x3E3DCD5E, 0x34B504F3]),
    v([0x3E40C025, 0x349837F0, 0x3E40C025, 0x349837F0]),
    v([0x3E43B5D3, 0x34800000, 0x3E43B5D3, 0x34800000]),
    v([0x3E46AE60, 0x345744FD, 0x3E46AE60, 0x345744FD]),
    v([0x3E49A9C8, 0x343504F3, 0x3E49A9C8, 0x343504F3]),
    v([0x3E4CA806, 0x341837F0, 0x3E4CA806, 0x341837F0]),
    v([0x3E4FA913, 0x34000000, 0x3E4FA913, 0x34000000]),
    v([0x3E52ACEA, 0x33D744FD, 0x3E52ACEA, 0x33D744FD]),
    v([0x3E55B388, 0x33B504F3, 0x3E55B388, 0x33B504F3]),
    v([0x3E58BCE5, 0x339837F0, 0x3E58BCE5, 0x339837F0]),
    v([0x3E5BC8FF, 0x33800000, 0x3E5BC8FF, 0x33800000]),
    v([0x3E5ED7CE, 0x335744FD, 0x3E5ED7CE, 0x335744FD]),
    v([0x3E61E950, 0x333504F3, 0x3E61E950, 0x333504F3]),
    v([0x3E64FD7F, 0x331837F0, 0x3E64FD7F, 0x331837F0]),
    v([0x3E681456, 0x33000000, 0x3E681456, 0x33000000]),
    v([0x3E6B2DD2, 0x32D744FD, 0x3E6B2DD2, 0x32D744FD]),
    v([0x3E6E49ED, 0x32B504F3, 0x3E6E49ED, 0x32B504F3]),
    v([0x3E7168A3, 0x329837F0, 0x3E7168A3, 0x329837F0]),
    v([0x3E7489EF, 0x32800000, 0x3E7489EF, 0x32800000]),
    v([0x3E77ADCF, 0x325744FD, 0x3E77ADCF, 0x325744FD]),
    v([0x3E7AD43C, 0x323504F3, 0x3E7AD43C, 0x323504F3]),
    v([0x3E7DFD34, 0x321837F0, 0x3E7DFD34, 0x321837F0]),
    v([0x3E809459, 0x32000000, 0x3E809459, 0x32000000]),
    v([0x3E822B59, 0x31D744FD, 0x3E822B59, 0x31D744FD]),
    v([0x3E83C399, 0x31B504F3, 0x3E83C399, 0x31B504F3]),
    v([0x3E855D15, 0x319837F0, 0x3E855D15, 0x319837F0]),
    v([0x3E86F7CD, 0x31800000, 0x3E86F7CD, 0x31800000]),
    v([0x3E8893BE, 0x315744FD, 0x3E8893BE, 0x315744FD]),
    v([0x3E8A30E6, 0x313504F3, 0x3E8A30E6, 0x313504F3]),
    v([0x3E8BCF45, 0x311837F0, 0x3E8BCF45, 0x311837F0]),
    v([0x3E8D6ED7, 0x31000000, 0x3E8D6ED7, 0x31000000]),
    v([0x3E8F0F9C, 0x30D744FD, 0x3E8F0F9C, 0x30D744FD]),
    v([0x3E90B190, 0x30B504F3, 0x3E90B190, 0x30B504F3]),
    v([0x3E9254B4, 0x309837F0, 0x3E9254B4, 0x309837F0]),
    v([0x3E93F904, 0x30800000, 0x3E93F904, 0x30800000]),
    v([0x3E959E80, 0x305744FD, 0x3E959E80, 0x305744FD]),
    v([0x3E974526, 0x303504F3, 0x3E974526, 0x303504F3]),
    v([0x3E98ECF3, 0x301837F0, 0x3E98ECF3, 0x301837F0]),
    v([0x3E9A95E7, 0x30000000, 0x3E9A95E7, 0x30000000]),
    v([0x3E9C4000, 0x2FD744FD, 0x3E9C4000, 0x2FD744FD]),
    v([0x3E9DEB3C, 0x2FB504F3, 0x3E9DEB3C, 0x2FB504F3]),
    v([0x3E9F979A, 0x2F9837F0, 0x3E9F979A, 0x2F9837F0]),
    v([0x3EA14518, 0x3F800000, 0x3EA14518, 0x3F800000]),
    v([0x3EA2F3B4, 0x3F000000, 0x3EA2F3B4, 0x3F000000]),
    v([0x3EA4A36E, 0x3E800000, 0x3EA4A36E, 0x3E800000]),
    v([0x3EA65444, 0x3E000000, 0x3EA65444, 0x3E000000]),
    v([0x3EA80634, 0x3D800000, 0x3EA80634, 0x3D800000]),
    v([0x3EA9B93D, 0x3D000000, 0x3EA9B93D, 0x3D000000]),
    v([0x3EAB6D5D, 0x3C800000, 0x3EAB6D5D, 0x3C800000]),
    v([0x3EAD2294, 0x3C000000, 0x3EAD2294, 0x3C000000]),
    v([0x3EAED8DF, 0x3B800000, 0x3EAED8DF, 0x3B800000]),
    v([0x3EB0903D, 0x3B000000, 0x3EB0903D, 0x3B000000]),
    v([0x3EB248AE, 0x3A800000, 0x3EB248AE, 0x3A800000]),
    v([0x3EB4022F, 0x3A000000, 0x3EB4022F, 0x3A000000]),
    v([0x3EB5BCBF, 0x39800000, 0x3EB5BCBF, 0x39800000]),
    v([0x3EB7785E, 0x39000000, 0x3EB7785E, 0x39000000]),
    v([0x3EB93509, 0x38800000, 0x3EB93509, 0x38800000]),
    v([0x3EBAF2C0, 0x38000000, 0x3EBAF2C0, 0x38000000]),
    v([0x3EBCB181, 0x00000000, 0x3EBCB181, 0x00000000]),
    v([0x3EBE714C, 0x00000000, 0x3EBE714C, 0x00000000]),
    v([0x3EC0321E, 0x00000000, 0x3EC0321E, 0x00000000]),
    v([0x3EC1F3F6, 0x00000000, 0x3EC1F3F6, 0x00000000]),
    v([0x3EC3B6D5, 0x00000000, 0x3EC3B6D5, 0x00000000]),
    v([0x3EC57AB7, 0x00000000, 0x3EC57AB7, 0x00000000]),
    v([0x3EC73F9C, 0x00000000, 0x3EC73F9C, 0x00000000]),
    v([0x3EC90584, 0x00000000, 0x3EC90584, 0x00000000]),
    v([0x3ECACC6C, 0x00000000, 0x3ECACC6C, 0x00000000]),
    v([0x3ECC9454, 0x00000000, 0x3ECC9454, 0x00000000]),
    v([0x3ECE5D3A, 0x00000000, 0x3ECE5D3A, 0x00000000]),
    v([0x3ED0271E, 0x00000000, 0x3ED0271E, 0x00000000]),
    v([0x3ED1F1FF, 0x00000000, 0x3ED1F1FF, 0x00000000]),
    v([0x3ED3BDDA, 0x00000000, 0x3ED3BDDA, 0x00000000]),
    v([0x3ED58AB0, 0x00000000, 0x3ED58AB0, 0x00000000]),
    v([0x3ED7587F, 0x00000000, 0x3ED7587F, 0x00000000]),
    v([0x3ED92746, 0x00000000, 0x3ED92746, 0x00000000]),
    v([0x3EDAF704, 0x00000000, 0x3EDAF704, 0x00000000]),
    v([0x3EDCC7B8, 0x00000000, 0x3EDCC7B8, 0x00000000]),
    v([0x3EDE9961, 0x00000000, 0x3EDE9961, 0x00000000]),
    v([0x3EE06BFE, 0x00000000, 0x3EE06BFE, 0x00000000]),
    v([0x3EE23F8F, 0x00000000, 0x3EE23F8F, 0x00000000]),
    v([0x3EE41411, 0x00000000, 0x3EE41411, 0x00000000]),
    v([0x3EE5E984, 0x00000000, 0x3EE5E984, 0x00000000]),
    v([0x3EE7BFE8, 0x00000000, 0x3EE7BFE8, 0x00000000]),
    v([0x3EE9973A, 0x00000000, 0x3EE9973A, 0x00000000]),
    v([0x3EEB6F7B, 0x00000000, 0x3EEB6F7B, 0x00000000]),
    v([0x3EED48AA, 0x00000000, 0x3EED48AA, 0x00000000]),
    v([0x3EEF22C4, 0x00000000, 0x3EEF22C4, 0x00000000]),
    v([0x3EF0FDCA, 0x00000000, 0x3EF0FDCA, 0x00000000]),
    v([0x3EF2D9BB, 0x00000000, 0x3EF2D9BB, 0x00000000]),
    v([0x3EF4B695, 0x00000000, 0x3EF4B695, 0x00000000]),
    v([0x3EF69458, 0x00000000, 0x3EF69458, 0x00000000]),
    v([0x3EF87302, 0x00000000, 0x3EF87302, 0x00000000]),
    v([0x3EFA5294, 0x00000000, 0x3EFA5294, 0x00000000]),
    v([0x3EFC330C, 0x00000000, 0x3EFC330C, 0x00000000]),
    v([0x3EFE1469, 0x00000000, 0x3EFE1469, 0x00000000]),
    v([0x3EFFF6AB, 0x00000000, 0x3EFFF6AB, 0x00000000]),
    v([0x3F00ECE8, 0x00000000, 0x3F00ECE8, 0x00000000]),
    v([0x3F01DEEC, 0x00000000, 0x3F01DEEC, 0x00000000]),
    v([0x3F02D161, 0x00000000, 0x3F02D161, 0x00000000]),
    v([0x3F03C446, 0x00000000, 0x3F03C446, 0x00000000]),
    v([0x3F04B79C, 0x00000000, 0x3F04B79C, 0x00000000]),
    v([0x3F05AB61, 0x00000000, 0x3F05AB61, 0x00000000]),
    v([0x3F069F96, 0x00000000, 0x3F069F96, 0x00000000]),
    v([0x3F079439, 0x00000000, 0x3F079439, 0x00000000]),
    v([0x3F08894B, 0x00000000, 0x3F08894B, 0x00000000]),
    v([0x3F097ECC, 0x00000000, 0x3F097ECC, 0x00000000]),
    v([0x3F0A74BA, 0x00000000, 0x3F0A74BA, 0x00000000]),
    v([0x3F0B6B15, 0x00000000, 0x3F0B6B15, 0x00000000]),
    v([0x3F0C61DE, 0x00000000, 0x3F0C61DE, 0x00000000]),
    v([0x3F0D5913, 0x00000000, 0x3F0D5913, 0x00000000]),
    v([0x3F0E50B4, 0x00000000, 0x3F0E50B4, 0x00000000]),
    v([0x3F0F48C2, 0x00000000, 0x3F0F48C2, 0x00000000]),
    v([0x3F10413A, 0x00000000, 0x3F10413A, 0x00000000]),
    v([0x3F113A1E, 0x00000000, 0x3F113A1E, 0x00000000]),
    v([0x3F12336D, 0x00000000, 0x3F12336D, 0x00000000]),
    v([0x3F132D27, 0x00000000, 0x3F132D27, 0x00000000]),
    v([0x3F14274A, 0x00000000, 0x3F14274A, 0x00000000]),
    v([0x3F1521D7, 0x00000000, 0x3F1521D7, 0x00000000]),
    v([0x3F161CCE, 0x00000000, 0x3F161CCE, 0x00000000]),
    v([0x3F17182D, 0x00000000, 0x3F17182D, 0x00000000]),
    v([0x3F1813F6, 0x00000000, 0x3F1813F6, 0x00000000]),
    v([0x3F191027, 0x00000000, 0x3F191027, 0x00000000]),
    v([0x3F1A0CBF, 0x00000000, 0x3F1A0CBF, 0x00000000]),
    v([0x3F1B09C0, 0x00000000, 0x3F1B09C0, 0x00000000]),
    v([0x3F1C0728, 0x00000000, 0x3F1C0728, 0x00000000]),
    v([0x3F1D04F7, 0x00000000, 0x3F1D04F7, 0x00000000]),
    v([0x3F1E032C, 0x00000000, 0x3F1E032C, 0x00000000]),
    v([0x3F1F01C9, 0x00000000, 0x3F1F01C9, 0x00000000]),
    v([0x3F2000CB, 0x00000000, 0x3F2000CB, 0x00000000]),
    v([0x3F210033, 0x00000000, 0x3F210033, 0x00000000]),
    v([0x3F220000, 0x00000000, 0x3F220000, 0x00000000]),
    v([0x3F230033, 0x00000000, 0x3F230033, 0x00000000]),
    v([0x3F2400CA, 0x00000000, 0x3F2400CA, 0x00000000]),
    v([0x3F2501C6, 0x00000000, 0x3F2501C6, 0x00000000]),
    v([0x3F260326, 0x00000000, 0x3F260326, 0x00000000]),
    v([0x3F2704EA, 0x00000000, 0x3F2704EA, 0x00000000]),
    v([0x3F280711, 0x00000000, 0x3F280711, 0x00000000]),
    v([0x3F29099C, 0x00000000, 0x3F29099C, 0x00000000]),
    v([0x3F2A0C8A, 0x00000000, 0x3F2A0C8A, 0x00000000]),
    v([0x3F2B0FDB, 0x00000000, 0x3F2B0FDB, 0x00000000]),
    v([0x3F2C138E, 0x00000000, 0x3F2C138E, 0x00000000]),
    v([0x3F2D17A3, 0x00000000, 0x3F2D17A3, 0x00000000]),
    v([0x3F2E1C1A, 0x00000000, 0x3F2E1C1A, 0x00000000]),
    v([0x3F2F20F2, 0x00000000, 0x3F2F20F2, 0x00000000]),
    v([0x3F30262C, 0x00000000, 0x3F30262C, 0x00000000]),
    v([0x3F312BC7, 0x00000000, 0x3F312BC7, 0x00000000]),
    v([0x3F3231C3, 0x00000000, 0x3F3231C3, 0x00000000]),
    v([0x3F33381F, 0x00000000, 0x3F33381F, 0x00000000]),
    v([0x3F343EDB, 0x00000000, 0x3F343EDB, 0x00000000]),
    v([0x3F3545F7, 0x00000000, 0x3F3545F7, 0x00000000]),
    v([0x3F364D72, 0x00000000, 0x3F364D72, 0x00000000]),
    v([0x3F37554D, 0x00000000, 0x3F37554D, 0x00000000]),
    v([0x3F385D87, 0x00000000, 0x3F385D87, 0x00000000]),
    v([0x3F396620, 0x00000000, 0x3F396620, 0x00000000]),
    v([0x3F3A6F17, 0x00000000, 0x3F3A6F17, 0x00000000]),
    v([0x3F3B786D, 0x00000000, 0x3F3B786D, 0x00000000]),
    v([0x3F3C8221, 0x00000000, 0x3F3C8221, 0x00000000]),
    v([0x3F3D8C32, 0x00000000, 0x3F3D8C32, 0x00000000]),
    v([0x3F3E96A1, 0x00000000, 0x3F3E96A1, 0x00000000]),
    v([0x3F3FA16D, 0x00000000, 0x3F3FA16D, 0x00000000]),
    v([0x3F40AC96, 0x00000000, 0x3F40AC96, 0x00000000]),
    v([0x3F41B81C, 0x00000000, 0x3F41B81C, 0x00000000]),
    v([0x3F42C3FE, 0x00000000, 0x3F42C3FE, 0x00000000]),
    v([0x3F43D03D, 0x00000000, 0x3F43D03D, 0x00000000]),
    v([0x3F44DCD8, 0x00000000, 0x3F44DCD8, 0x00000000]),
    v([0x3F45E9CE, 0x00000000, 0x3F45E9CE, 0x00000000]),
    v([0x3F46F720, 0x00000000, 0x3F46F720, 0x00000000]),
    v([0x3F4804CD, 0x00000000, 0x3F4804CD, 0x00000000]),
    v([0x3F4912D5, 0x00000000, 0x3F4912D5, 0x00000000]),
    v([0x3F4A2138, 0x00000000, 0x3F4A2138, 0x00000000]),
    v([0x3F4B2FF5, 0x00000000, 0x3F4B2FF5, 0x00000000]),
    v([0x3F4C3F0D, 0x00000000, 0x3F4C3F0D, 0x00000000]),
    v([0x3F4D4E7F, 0x00000000, 0x3F4D4E7F, 0x00000000]),
    v([0x3F4E5E4A, 0x00000000, 0x3F4E5E4A, 0x00000000]),
    v([0x3F4F6E70, 0x00000000, 0x3F4F6E70, 0x00000000]),
    v([0x3F507EEE, 0x00000000, 0x3F507EEE, 0x00000000]),
    v([0x3F518FC6, 0x00000000, 0x3F518FC6, 0x00000000]),
    v([0x3F52A0F7, 0x00000000, 0x3F52A0F7, 0x00000000]),
    v([0x3F53B280, 0x00000000, 0x3F53B280, 0x00000000]),
    v([0x3F54C462, 0x00000000, 0x3F54C462, 0x00000000]),
    v([0x3F55D69C, 0x00000000, 0x3F55D69C, 0x00000000]),
    v([0x3F56E92E, 0x00000000, 0x3F56E92E, 0x00000000]),
    v([0x3F57FC18, 0x00000000, 0x3F57FC18, 0x00000000]),
    v([0x3F590F5A, 0x00000000, 0x3F590F5A, 0x00000000]),
    v([0x3F5A22F2, 0x00000000, 0x3F5A22F2, 0x00000000]),
    v([0x3F5B36E3, 0x00000000, 0x3F5B36E3, 0x00000000]),
    v([0x3F5C4B2A, 0x00000000, 0x3F5C4B2A, 0x00000000]),
    v([0x3F5D5FC7, 0x00000000, 0x3F5D5FC7, 0x00000000]),
    v([0x3F5E74BC, 0x00000000, 0x3F5E74BC, 0x00000000]),
    v([0x3F5F8A06, 0x00000000, 0x3F5F8A06, 0x00000000]),
    v([0x3F609FA7, 0x00000000, 0x3F609FA7, 0x00000000]),
    v([0x3F61B59D, 0x00000000, 0x3F61B59D, 0x00000000]),
    v([0x3F62CBE9, 0x00000000, 0x3F62CBE9, 0x00000000]),
    v([0x3F63E28B, 0x00000000, 0x3F63E28B, 0x00000000]),
    v([0x3F64F982, 0x00000000, 0x3F64F982, 0x00000000]),
    v([0x3F6610CE, 0x00000000, 0x3F6610CE, 0x00000000]),
    v([0x3F67286F, 0x00000000, 0x3F67286F, 0x00000000]),
    v([0x3F684065, 0x00000000, 0x3F684065, 0x00000000]),
    v([0x3F6958AF, 0x00000000, 0x3F6958AF, 0x00000000]),
    v([0x3F6A714D, 0x00000000, 0x3F6A714D, 0x00000000]),
    v([0x3F6B8A3F, 0x00000000, 0x3F6B8A3F, 0x00000000]),
    v([0x3F6CA386, 0x00000000, 0x3F6CA386, 0x00000000]),
    v([0x3F6DBD20, 0x00000000, 0x3F6DBD20, 0x00000000]),
    v([0x3F6ED70D, 0x00000000, 0x3F6ED70D, 0x00000000]),
    v([0x3F6FF14E, 0x00000000, 0x3F6FF14E, 0x00000000]),
    v([0x3F710BE1, 0x00000000, 0x3F710BE1, 0x00000000]),
    v([0x3F7226C8, 0x00000000, 0x3F7226C8, 0x00000000]),
    v([0x3F734202, 0x00000000, 0x3F734202, 0x00000000]),
    v([0x3F745D8E, 0x00000000, 0x3F745D8E, 0x00000000]),
    v([0x3F75796C, 0x00000000, 0x3F75796C, 0x00000000]),
    v([0x3F76959C, 0x00000000, 0x3F76959C, 0x00000000]),
    v([0x3F77B21F, 0x00000000, 0x3F77B21F, 0x00000000]),
    v([0x3F78CEF3, 0x00000000, 0x3F78CEF3, 0x00000000]),
    v([0x3F79EC19, 0x00000000, 0x3F79EC19, 0x00000000]),
    v([0x3F7B0990, 0x00000000, 0x3F7B0990, 0x00000000]),
    v([0x3F7C2759, 0x00000000, 0x3F7C2759, 0x00000000]),
    v([0x3F7D4572, 0x00000000, 0x3F7D4572, 0x00000000]),
    v([0x3F7E63DD, 0x00000000, 0x3F7E63DD, 0x00000000]),
    v([0x3F7F8298, 0x00000000, 0x3F7F8298, 0x00000000]),
    v([0x3F8050D2, 0x00000000, 0x3F8050D2, 0x00000000]),
    v([0x3F80E080, 0x00000000, 0x3F80E080, 0x00000000]),
    v([0x3F817056, 0x00000000, 0x3F817056, 0x00000000]),
    v([0x3F820054, 0x00000000, 0x3F820054, 0x00000000]),
    v([0x3F82907A, 0x00000000, 0x3F82907A, 0x00000000]),
    v([0x3F8320C8, 0x00000000, 0x3F8320C8, 0x00000000]),
    v([0x3F83B13E, 0x00000000, 0x3F83B13E, 0x00000000]),
    v([0x3F8441DB, 0x00000000, 0x3F8441DB, 0x00000000]),
    v([0x3F84D2A0, 0x00000000, 0x3F84D2A0, 0x00000000]),
    v([0x3F85638C, 0x00000000, 0x3F85638C, 0x00000000]),
    v([0x3F85F4A0, 0x00000000, 0x3F85F4A0, 0x00000000]),
    v([0x3F8685DB, 0x00000000, 0x3F8685DB, 0x00000000]),
    v([0x3F87173D, 0x00000000, 0x3F87173D, 0x00000000]),
    v([0x3F87A8C7, 0x00000000, 0x3F87A8C7, 0x00000000]),
    v([0x3F883A77, 0x00000000, 0x3F883A77, 0x00000000]),
    v([0x3F88CC4F, 0x00000000, 0x3F88CC4F, 0x00000000]),
    v([0x3F895E4D, 0x00000000, 0x3F895E4D, 0x00000000]),
    v([0x3F89F072, 0x00000000, 0x3F89F072, 0x00000000]),
    v([0x3F8A82BE, 0x00000000, 0x3F8A82BE, 0x00000000]),
    v([0x3F8B1531, 0x00000000, 0x3F8B1531, 0x00000000]),
    v([0x3F8BA7CA, 0x00000000, 0x3F8BA7CA, 0x00000000]),
    v([0x3F8C3A8A, 0x00000000, 0x3F8C3A8A, 0x00000000]),
    v([0x3F8CCD70, 0x00000000, 0x3F8CCD70, 0x00000000]),
    v([0x3F8D607D, 0x00000000, 0x3F8D607D, 0x00000000]),
    v([0x3F8DF3B0, 0x00000000, 0x3F8DF3B0, 0x00000000]),
    v([0x3F8E8709, 0x00000000, 0x3F8E8709, 0x00000000]),
    v([0x3F8F1A88, 0x00000000, 0x3F8F1A88, 0x00000000]),
    v([0x3F8FAE2D, 0x00000000, 0x3F8FAE2D, 0x00000000]),
    v([0x3F9041F8, 0x00000000, 0x3F9041F8, 0x00000000]),
    v([0x3F90D5EA, 0x00000000, 0x3F90D5EA, 0x00000000]),
    v([0x3F916A00, 0x00000000, 0x3F916A00, 0x00000000]),
    v([0x3F91FE3D, 0x00000000, 0x3F91FE3D, 0x00000000]),
    v([0x3F92929F, 0x00000000, 0x3F92929F, 0x00000000]),
    v([0x3F932727, 0x00000000, 0x3F932727, 0x00000000]),
    v([0x3F93BBD5, 0x00000000, 0x3F93BBD5, 0x00000000]),
    v([0x3F9450A8, 0x00000000, 0x3F9450A8, 0x00000000]),
    v([0x3F94E5A0, 0x00000000, 0x3F94E5A0, 0x00000000]),
    v([0x3F957ABD, 0x00000000, 0x3F957ABD, 0x00000000]),
    v([0x3F961000, 0x00000000, 0x3F961000, 0x00000000]),
    v([0x3F96A568, 0x00000000, 0x3F96A568, 0x00000000]),
    v([0x3F973AF5, 0x00000000, 0x3F973AF5, 0x00000000]),
    v([0x3F97D0A7, 0x00000000, 0x3F97D0A7, 0x00000000]),
    v([0x3F98667E, 0x00000000, 0x3F98667E, 0x00000000]),
    v([0x3F98FC7A, 0x00000000, 0x3F98FC7A, 0x00000000]),
    v([0x3F99929A, 0x00000000, 0x3F99929A, 0x00000000]),
    v([0x3F9A28DF, 0x00000000, 0x3F9A28DF, 0x00000000]),
    v([0x3F9ABF49, 0x00000000, 0x3F9ABF49, 0x00000000]),
    v([0x3F9B55D8, 0x00000000, 0x3F9B55D8, 0x00000000]),
    v([0x3F9BEC8B, 0x00000000, 0x3F9BEC8B, 0x00000000]),
    v([0x3F9C8363, 0x00000000, 0x3F9C8363, 0x00000000]),
    v([0x3F9D1A5E, 0x00000000, 0x3F9D1A5E, 0x00000000]),
    v([0x3F9DB17F, 0x00000000, 0x3F9DB17F, 0x00000000]),
    v([0x3F9E48C3, 0x00000000, 0x3F9E48C3, 0x00000000]),
    v([0x3F9EE02C, 0x00000000, 0x3F9EE02C, 0x00000000]),
    v([0x3F9F77B8, 0x00000000, 0x3F9F77B8, 0x00000000]),
    v([0x3FA00F69, 0x00000000, 0x3FA00F69, 0x00000000]),
    v([0x3FA0A73E, 0x00000000, 0x3FA0A73E, 0x00000000]),
    v([0x3FA13F37, 0x00000000, 0x3FA13F37, 0x00000000]),
    v([0x3FA1D753, 0x00000000, 0x3FA1D753, 0x00000000]),
    v([0x3FA26F93, 0x00000000, 0x3FA26F93, 0x00000000]),
    v([0x3FA307F7, 0x00000000, 0x3FA307F7, 0x00000000]),
    v([0x3FA3A07F, 0x00000000, 0x3FA3A07F, 0x00000000]),
    v([0x3FA4392A, 0x00000000, 0x3FA4392A, 0x00000000]),
    v([0x3FA4D1F9, 0x00000000, 0x3FA4D1F9, 0x00000000]),
    v([0x3FA56AEB, 0x00000000, 0x3FA56AEB, 0x00000000]),
    v([0x3FA60400, 0x00000000, 0x3FA60400, 0x00000000]),
    v([0x3FA69D39, 0x00000000, 0x3FA69D39, 0x00000000]),
    v([0x3FA73695, 0x00000000, 0x3FA73695, 0x00000000]),
    v([0x3FA7D015, 0x00000000, 0x3FA7D015, 0x00000000]),
    v([0x3FA869B7, 0x00000000, 0x3FA869B7, 0x00000000]),
    v([0x3FA9037D, 0x00000000, 0x3FA9037D, 0x00000000]),
    v([0x3FA99D65, 0x00000000, 0x3FA99D65, 0x00000000]),
    v([0x3FAA3771, 0x00000000, 0x3FAA3771, 0x00000000]),
    v([0x3FAAD19F, 0x00000000, 0x3FAAD19F, 0x00000000]),
    v([0x3FAB6BF0, 0x00000000, 0x3FAB6BF0, 0x00000000]),
    v([0x3FAC0664, 0x00000000, 0x3FAC0664, 0x00000000]),
    v([0x3FACA0FB, 0x00000000, 0x3FACA0FB, 0x00000000]),
    v([0x3FAD3BB4, 0x00000000, 0x3FAD3BB4, 0x00000000]),
    v([0x3FADD690, 0x00000000, 0x3FADD690, 0x00000000]),
    v([0x3FAE718E, 0x00000000, 0x3FAE718E, 0x00000000]),
    v([0x3FAF0CAF, 0x00000000, 0x3FAF0CAF, 0x00000000]),
    v([0x3FAFA7F2, 0x00000000, 0x3FAFA7F2, 0x00000000]),
    v([0x3FB04358, 0x00000000, 0x3FB04358, 0x00000000]),
    v([0x3FB0DEE0, 0x00000000, 0x3FB0DEE0, 0x00000000]),
    v([0x3FB17A8A, 0x00000000, 0x3FB17A8A, 0x00000000]),
    v([0x3FB21656, 0x00000000, 0x3FB21656, 0x00000000]),
    v([0x3FB2B244, 0x00000000, 0x3FB2B244, 0x00000000]),
    v([0x3FB34E55, 0x00000000, 0x3FB34E55, 0x00000000]),
    v([0x3FB3EA87, 0x00000000, 0x3FB3EA87, 0x00000000]),
    v([0x3FB486DB, 0x00000000, 0x3FB486DB, 0x00000000]),
    v([0x3FB52352, 0x00000000, 0x3FB52352, 0x00000000]),
    v([0x3FB5BFE9, 0x00000000, 0x3FB5BFE9, 0x00000000]),
    v([0x3FB65CA3, 0x00000000, 0x3FB65CA3, 0x00000000]),
    v([0x3FB6F97F, 0x00000000, 0x3FB6F97F, 0x00000000]),
    v([0x3FB7967C, 0x00000000, 0x3FB7967C, 0x00000000]),
    v([0x3FB8339A, 0x00000000, 0x3FB8339A, 0x00000000]),
    v([0x3FB8D0DB, 0x00000000, 0x3FB8D0DB, 0x00000000]),
    v([0x3FB96E3C, 0x00000000, 0x3FB96E3C, 0x00000000]),
    v([0x3FBA0BBF, 0x00000000, 0x3FBA0BBF, 0x00000000]),
    v([0x3FBAA964, 0x00000000, 0x3FBAA964, 0x00000000]),
    v([0x3FBB472A, 0x00000000, 0x3FBB472A, 0x00000000]),
    v([0x3FBBE511, 0x00000000, 0x3FBBE511, 0x00000000]),
    v([0x3FBC8319, 0x00000000, 0x3FBC8319, 0x00000000]),
    v([0x3FBD2142, 0x00000000, 0x3FBD2142, 0x00000000]),
    v([0x3FBDBF8D, 0x00000000, 0x3FBDBF8D, 0x00000000]),
    v([0x3FBE5DF8, 0x00000000, 0x3FBE5DF8, 0x00000000]),
    v([0x3FBEFC85, 0x00000000, 0x3FBEFC85, 0x00000000]),
    v([0x3FBF9B32, 0x00000000, 0x3FBF9B32, 0x00000000]),
    v([0x3FC03A01, 0x00000000, 0x3FC03A01, 0x00000000]),
    v([0x3FC0D8F0, 0x00000000, 0x3FC0D8F0, 0x00000000]),
    v([0x3FC17800, 0x00000000, 0x3FC17800, 0x00000000]),
    v([0x3FC21730, 0x00000000, 0x3FC21730, 0x00000000]),
    v([0x3FC2B682, 0x00000000, 0x3FC2B682, 0x00000000]),
    v([0x3FC355F3, 0x00000000, 0x3FC355F3, 0x00000000]),
    v([0x3FC3F586, 0x00000000, 0x3FC3F586, 0x00000000]),
    v([0x3FC49539, 0x00000000, 0x3FC49539, 0x00000000]),
    v([0x3FC5350C, 0x00000000, 0x3FC5350C, 0x00000000]),
    v([0x3FC5D500, 0x00000000, 0x3FC5D500, 0x00000000]),
    v([0x3FC67514, 0x00000000, 0x3FC67514, 0x00000000]),
    v([0x3FC71549, 0x00000000, 0x3FC71549, 0x00000000]),
    v([0x3FC7B59E, 0x00000000, 0x3FC7B59E, 0x00000000]),
    v([0x3FC85613, 0x00000000, 0x3FC85613, 0x00000000]),
    v([0x3FC8F6A8, 0x00000000, 0x3FC8F6A8, 0x00000000]),
    v([0x3FC9975D, 0x00000000, 0x3FC9975D, 0x00000000]),
    v([0x3FCA3832, 0x00000000, 0x3FCA3832, 0x00000000]),
    v([0x3FCAD928, 0x00000000, 0x3FCAD928, 0x00000000]),
    v([0x3FCB7A3D, 0x00000000, 0x3FCB7A3D, 0x00000000]),
    v([0x3FCC1B72, 0x00000000, 0x3FCC1B72, 0x00000000]),
    v([0x3FCCBCC7, 0x00000000, 0x3FCCBCC7, 0x00000000]),
    v([0x3FCD5E3C, 0x00000000, 0x3FCD5E3C, 0x00000000]),
    v([0x3FCDFFD1, 0x00000000, 0x3FCDFFD1, 0x00000000]),
    v([0x3FCEA185, 0x00000000, 0x3FCEA185, 0x00000000]),
    v([0x3FCF4359, 0x00000000, 0x3FCF4359, 0x00000000]),
    v([0x3FCFE54C, 0x00000000, 0x3FCFE54C, 0x00000000]),
    v([0x3FD0875F, 0x00000000, 0x3FD0875F, 0x00000000]),
    v([0x3FD12992, 0x00000000, 0x3FD12992, 0x00000000]),
    v([0x3FD1CBE4, 0x00000000, 0x3FD1CBE4, 0x00000000]),
    v([0x3FD26E56, 0x00000000, 0x3FD26E56, 0x00000000]),
    v([0x3FD310E7, 0x00000000, 0x3FD310E7, 0x00000000]),
    v([0x3FD3B397, 0x00000000, 0x3FD3B397, 0x00000000]),
    v([0x3FD45666, 0x00000000, 0x3FD45666, 0x00000000]),
    v([0x3FD4F955, 0x00000000, 0x3FD4F955, 0x00000000]),
    v([0x3FD59C63, 0x00000000, 0x3FD59C63, 0x00000000]),
    v([0x3FD63F90, 0x00000000, 0x3FD63F90, 0x00000000]),
    v([0x3FD6E2DC, 0x00000000, 0x3FD6E2DC, 0x00000000]),
    v([0x3FD78647, 0x00000000, 0x3FD78647, 0x00000000]),
    v([0x3FD829D2, 0x00000000, 0x3FD829D2, 0x00000000]),
    v([0x3FD8CD7B, 0x00000000, 0x3FD8CD7B, 0x00000000]),
    v([0x3FD97143, 0x00000000, 0x3FD97143, 0x00000000]),
    v([0x3FDA152A, 0x00000000, 0x3FDA152A, 0x00000000]),
    v([0x3FDAB930, 0x00000000, 0x3FDAB930, 0x00000000]),
    v([0x3FDB5D54, 0x00000000, 0x3FDB5D54, 0x00000000]),
    v([0x3FDC0197, 0x00000000, 0x3FDC0197, 0x00000000]),
    v([0x3FDCA5F9, 0x00000000, 0x3FDCA5F9, 0x00000000]),
    v([0x3FDD4A7A, 0x00000000, 0x3FDD4A7A, 0x00000000]),
    v([0x3FDDEF19, 0x00000000, 0x3FDDEF19, 0x00000000]),
    v([0x3FDE93D7, 0x00000000, 0x3FDE93D7, 0x00000000]),
    v([0x3FDF38B3, 0x00000000, 0x3FDF38B3, 0x00000000]),
    v([0x3FDFDDAE, 0x00000000, 0x3FDFDDAE, 0x00000000]),
    v([0x3FE082C7, 0x00000000, 0x3FE082C7, 0x00000000]),
    v([0x3FE127FE, 0x00000000, 0x3FE127FE, 0x00000000]),
    v([0x3FE1CD54, 0x00000000, 0x3FE1CD54, 0x00000000]),
    v([0x3FE272C8, 0x00000000, 0x3FE272C8, 0x00000000]),
    v([0x3FE3185A, 0x00000000, 0x3FE3185A, 0x00000000]),
    v([0x3FE3BE0B, 0x00000000, 0x3FE3BE0B, 0x00000000]),
    v([0x3FE463DA, 0x00000000, 0x3FE463DA, 0x00000000]),
    v([0x3FE509C6, 0x00000000, 0x3FE509C6, 0x00000000]),
    v([0x3FE5AFD1, 0x00000000, 0x3FE5AFD1, 0x00000000]),
    v([0x3FE655FA, 0x00000000, 0x3FE655FA, 0x00000000]),
    v([0x3FE6FC41, 0x00000000, 0x3FE6FC41, 0x00000000]),
    v([0x3FE7A2A6, 0x00000000, 0x3FE7A2A6, 0x00000000]),
    v([0x3FE84929, 0x00000000, 0x3FE84929, 0x00000000]),
    v([0x3FE8EFC9, 0x00000000, 0x3FE8EFC9, 0x00000000]),
    v([0x3FE99688, 0x00000000, 0x3FE99688, 0x00000000]),
    v([0x3FEA3D64, 0x00000000, 0x3FEA3D64, 0x00000000]),
    v([0x3FEAE45E, 0x00000000, 0x3FEAE45E, 0x00000000]),
    v([0x3FEB8B76, 0x00000000, 0x3FEB8B76, 0x00000000]),
    v([0x3FEC32AB, 0x00000000, 0x3FEC32AB, 0x00000000]),
    v([0x3FECD9FE, 0x00000000, 0x3FECD9FE, 0x00000000]),
    v([0x3FED816E, 0x00000000, 0x3FED816E, 0x00000000]),
    v([0x3FEE28FC, 0x00000000, 0x3FEE28FC, 0x00000000]),
    v([0x3FEED0A8, 0x00000000, 0x3FEED0A8, 0x00000000]),
    v([0x3FEF7871, 0x00000000, 0x3FEF7871, 0x00000000]),
    v([0x3FF02057, 0x00000000, 0x3FF02057, 0x00000000]),
    v([0x3FF0C85B, 0x00000000, 0x3FF0C85B, 0x00000000]),
    v([0x3FF1707C, 0x00000000, 0x3FF1707C, 0x00000000]),
    v([0x3FF218BA, 0x00000000, 0x3FF218BA, 0x00000000]),
    v([0x3FF2C116, 0x00000000, 0x3FF2C116, 0x00000000]),
    v([0x3FF3698F, 0x00000000, 0x3FF3698F, 0x00000000]),
    v([0x3FF41225, 0x00000000, 0x3FF41225, 0x00000000]),
    v([0x3FF4BAD8, 0x00000000, 0x3FF4BAD8, 0x00000000]),
    v([0x3FF563A8, 0x00000000, 0x3FF563A8, 0x00000000]),
    v([0x3FF60C95, 0x00000000, 0x3FF60C95, 0x00000000]),
    v([0x3FF6B59F, 0x00000000, 0x3FF6B59F, 0x00000000]),
    v([0x3FF75EC7, 0x00000000, 0x3FF75EC7, 0x00000000]),
    v([0x3FF8080B, 0x00000000, 0x3FF8080B, 0x00000000]),
    v([0x3FF8B16C, 0x00000000, 0x3FF8B16C, 0x00000000]),
    v([0x3FF95AEA, 0x00000000, 0x3FF95AEA, 0x00000000]),
    v([0x3FFA0485, 0x00000000, 0x3FFA0485, 0x00000000]),
    v([0x3FFAAE3C, 0x00000000, 0x3FFAAE3C, 0x00000000]),
    v([0x3FFB5810, 0x00000000, 0x3FFB5810, 0x00000000]),
    v([0x3FFC0201, 0x00000000, 0x3FFC0201, 0x00000000]),
    v([0x3FFCAC0F, 0x00000000, 0x3FFCAC0F, 0x00000000]),
    v([0x3FFD5639, 0x00000000, 0x3FFD5639, 0x00000000]),
    v([0x3FFE0080, 0x00000000, 0x3FFE0080, 0x00000000]),
    v([0x3FFEAAE4, 0x00000000, 0x3FFEAAE4, 0x00000000]),
    v([0x3FFF5564, 0x00000000, 0x3FFF5564, 0x00000000]),
    v([0x00000000, 0x00000000, 0x00000000, 0x00000000]),
];

const TRANSFORM_TABLE: [V; 256] = [
    v([0x3F3504F3, 0x3F7FB10F, 0x3F7EC46D, 0x3F7D3AAC]),
    v([0x3F7B14BE, 0x3F7853F8, 0x3F74FA0B, 0x3F710908]),
    v([0x3F6C835E, 0x3F676BD8, 0x3F61C597, 0x3F5B941A]),
    v([0x3F54DB31, 0x3F4D9F02, 0x3F45E403, 0x3F3DAEF9]),
    v([0x3F3504F3, 0x3F2BEB49, 0x3F226799, 0x3F187FC0]),
    v([0x3F0E39D9, 0x3F039C3C, 0x3EF15AE7, 0x3EDAE881]),
    v([0x3EC3EF15, 0x3EAC7CD3, 0x3E94A030, 0x3E78CFC8]),
    v([0x3E47C5BC, 0x3E164085, 0x3DC8BD35, 0x3D48FB29]),
    v([0x3F3504F3, 0x3F7D3AAC, 0x3F74FA0B, 0x3F676BD8]),
    v([0x3F54DB31, 0x3F3DAEF9, 0x3F226799, 0x3F039C3C]),
    v([0x3EC3EF15, 0x3E78CFC8, 0x3DC8BD35, 0xBD48FB41]),
    v([0xBE47C5C2, 0xBEAC7CD6, 0xBEF15AED, 0xBF187FC1]),
    v([0xBF3504F3, 0xBF4D9F04, 0xBF61C599, 0xBF710909]),
    v([0xBF7B14BF, 0xBF7FB10F, 0xBF7EC46D, 0xBF7853F8]),
    v([0xBF6C835E, 0xBF5B9419, 0xBF45E402, 0xBF2BEB49]),
    v([0xBF0E39D6, 0xBEDAE87B, 0xBE94A02D, 0xBE16407F]),
    v([0x3F3504F3, 0x3F7853F8, 0x3F61C597, 0x3F3DAEF9]),
    v([0x3F0E39D9, 0x3EAC7CD3, 0x3DC8BD35, 0xBE16408A]),
    v([0xBEC3EF18, 0xBF187FC1, 0xBF45E404, 0xBF676BD8]),
    v([0xBF7B14BF, 0xBF7FB10F, 0xBF74FA0A, 0xBF5B9419]),
    v([0xBF3504F1, 0xBF039C3E, 0xBE94A02D, 0xBD48FAD2]),
    v([0x3E47C5C8, 0x3EDAE88A, 0x3F22679A, 0x3F4D9F05]),
    v([0x3F6C835F, 0x3F7D3AAD, 0x3F7EC46D, 0x3F710908]),
    v([0x3F54DB31, 0x3F2BEB49, 0x3EF15AE7, 0x3E78CFC8]),
    v([0x3F3504F3, 0x3F710908, 0x3F45E403, 0x3F039C3C]),
    v([0x3E47C5BC, 0xBE16408A, 0xBEF15AED, 0xBF3DAEFB]),
    v([0xBF6C8360, 0xBF7FB10F, 0xBF74FA0A, 0xBF4D9F02]),
    v([0xBF0E39D6, 0xBE78CFBA, 0x3DC8BD5D, 0x3EDAE88A]),
    v([0x3F3504F7, 0x3F676BDA, 0x3F7EC46E, 0x3F7853F8]),
    v([0x3F54DB31, 0x3F187FC0, 0x3E94A030, 0xBD48FB41]),
    v([0xBEC3EF18, 0xBF2BEB4B, 0xBF61C599, 0xBF7D3AAC]),
    v([0xBF7B14BE, 0xBF5B9419, 0xBF22679A, 0xBEAC7CD4]),
    v([0x3F3504F3, 0x3F676BD8, 0x3F226799, 0x3E78CFC8]),
    v([0xBE47C5C2, 0xBF187FC1, 0xBF61C599, 0xBF7FB10F]),
    v([0xBF6C835E, 0xBF2BEB49, 0xBE94A02D, 0x3E164080]),
    v([0x3F0E39DD, 0x3F5B941B, 0x3F7EC46E, 0x3F710908]),
    v([0x3F3504F3, 0x3EAC7CD3, 0xBDC8BD41, 0xBF039C3D]),
    v([0xBF54DB32, 0xBF7D3AAC, 0xBF74FA0A, 0xBF3DAEF9]),
    v([0xBEC3EF0B, 0x3D48FB58, 0x3EF15AE9, 0x3F4D9F05]),
    v([0x3F7B14BF, 0x3F7853F8, 0x3F45E403, 0x3EDAE881]),
    v([0x3F3504F3, 0x3F5B941A, 0x3EF15AE7, 0xBD48FB41]),
    v([0xBF0E39DC, 0xBF676BD8, 0xBF7EC46D, 0xBF4D9F02]),
    v([0xBEC3EF0B, 0x3E164080, 0x3F22679A, 0x3F710909]),
    v([0x3F7B14BE, 0x3F3DAEF9, 0x3E94A030, 0xBE78CFCD]),
    v([0xBF3504F3, 0xBF7853F8, 0xBF74FA0A, 0xBF2BEB49]),
    v([0xBE47C5C6, 0x3EAC7CD5, 0x3F45E405, 0x3F7D3AAD]),
    v([0x3F6C835E, 0x3F187FC0, 0x3DC8BD35, 0xBEDAE880]),
    v([0xBF54DB32, 0xBF7FB10F, 0xBF61C597, 0xBF039C3E]),
    v([0x3F3504F3, 0x3F4D9F02, 0x3E94A030, 0xBEAC7CD6]),
    v([0xBF54DB32, 0xBF7FB10F, 0xBF45E402, 0xBE78CFBA]),
    v([0x3EC3EF1B, 0x3F5B941B, 0x3F7EC46D, 0x3F3DAEF9]),
    v([0x3E47C5BC, 0xBEDAE880, 0xBF61C599, 0xBF7D3AAC]),
    v([0xBF3504F1, 0xBE16407F, 0x3EF15AE9, 0x3F676BDA]),
    v([0x3F7B14BE, 0x3F2BEB49, 0x3DC8BD35, 0xBF039C3D]),
    v([0xBF6C8360, 0xBF7853F8, 0xBF22679A, 0xBD48FAD2]),
    v([0x3F0E39DD, 0x3F710909, 0x3F74FA0B, 0x3F187FC0]),
    v([0x3F3504F3, 0x3F3DAEF9, 0x3DC8BD35, 0xBF187FC1]),
    v([0xBF7B14BF, 0xBF5B9419, 0xBE94A02D, 0x3EDAE88A]),
    v([0x3F6C835F, 0x3F710908, 0x3EF15AE7, 0xBE78CFCD]),
    v([0xBF54DB32, 0xBF7D3AAC, 0xBF22679A, 0x3D48FB58]),
    v([0x3F3504F7, 0x3F7FB10F, 0x3F45E403, 0x3E164085]),
    v([0xBF0E39DC, 0xBF7853F8, 0xBF61C597, 0xBEAC7CD4]),
    v([0x3EC3EF1B, 0x3F676BDA, 0x3F74FA0B, 0x3F039C3C]),
    v([0xBE47C5C2, 0xBF4D9F04, 0xBF7EC46D, 0xBF2BEB49]),
    v([0x3F3504F3, 0x3F2BEB49, 0xBDC8BD41, 0xBF4D9F04]),
    v([0xBF7B14BE, 0xBF039C3E, 0x3E94A03D, 0x3F676BDA]),
    v([0x3F6C835E, 0x3EAC7CD3, 0xBEF15AED, 0xBF7853F8]),
    v([0xBF54DB30, 0xBE16407F, 0x3F22679A, 0x3F7FB10F]),
    v([0x3F3504F3, 0xBD48FB41, 0xBF45E404, 0xBF7D3AAC]),
    v([0xBF0E39D6, 0x3E78CFDB, 0x3F61C599, 0x3F710908]),
    v([0x3EC3EF15, 0xBEDAE880, 0xBF74FA0B, 0xBF5B9419]),
    v([0xBE47C5C6, 0x3F187FBF, 0x3F7EC46E, 0x3F3DAEF9]),
    v([0x3F3504F3, 0x3F187FC0, 0xBE94A033, 0xBF710909]),
    v([0xBF54DB30, 0xBD48FAD2, 0x3F45E405, 0x3F7853F8]),
    v([0x3EC3EF15, 0xBF039C3D, 0xBF7EC46D, 0xBF2BEB49]),
    v([0x3E47C5C8, 0x3F676BDA, 0x3F61C597, 0x3E164085]),
    v([0xBF3504F3, 0xBF7D3AAC, 0xBEF15AE8, 0x3EDAE88A]),
    v([0x3F7B14BF, 0x3F3DAEF9, 0xBDC8BD41, 0xBF5B941A]),
    v([0xBF6C835E, 0xBE78CFBA, 0x3F22679A, 0x3F7FB10F]),
    v([0x3F0E39D9, 0xBEAC7CD6, 0xBF74FA0B, 0xBF4D9F02]),
    v([0x3F3504F3, 0x3F039C3C, 0xBEF15AED, 0xBF7FB10F]),
    v([0xBF0E39D6, 0x3EDAE88A, 0x3F7EC46E, 0x3F187FC0]),
    v([0xBEC3EF18, 0xBF7D3AAC, 0xBF22679A, 0x3EAC7CD5]),
    v([0x3F7B14BF, 0x3F2BEB49, 0xBE94A033, 0xBF7853F8]),
    v([0xBF3504F1, 0x3E78CFDB, 0x3F74FA0C, 0x3F3DAEF9]),
    v([0xBE47C5C2, 0xBF710909, 0xBF45E402, 0x3E164080]),
    v([0x3F6C835F, 0x3F4D9F02, 0xBDC8BD41, 0xBF676BD8]),
    v([0xBF54DB30, 0x3D48FB58, 0x3F61C599, 0x3F5B941A]),
    v([0x3F3504F3, 0x3EDAE881, 0xBF226799, 0xBF7853F8]),
    v([0xBE47C5C6, 0x3F4D9F05, 0x3F61C597, 0xBD48FB41]),
    v([0xBF6C8360, 0xBF3DAEF9, 0x3E94A03D, 0x3F7D3AAD]),
    v([0x3F0E39D9, 0xBF039C3D, 0xBF7EC46D, 0xBEAC7CD4]),
    v([0x3F3504F7, 0x3F710908, 0x3DC8BD35, 0xBF5B941A]),
    v([0xBF54DB30, 0x3E164080, 0x3F74FA0C, 0x3F2BEB49]),
    v([0xBEC3EF18, 0xBF7FB10F, 0xBEF15AE8, 0x3F187FBF]),
    v([0x3F7B14BE, 0x3E78CFC8, 0xBF45E404, 0xBF676BD7]),
    v([0x3F3504F3, 0x3EAC7CD3, 0xBF45E404, 0xBF5B9419]),
    v([0x3E47C5C8, 0x3F7D3AAD, 0x3EF15AE7, 0xBF2BEB4B]),
    v([0xBF6C835E, 0x3D48FB58, 0x3F74FA0C, 0x3F187FC0]),
    v([0xBF0E39DC, 0xBF7853F8, 0xBDC8BD1A, 0x3F676BDA]),
    v([0x3F3504F3, 0xBEDAE880, 0xBF7EC46D, 0xBE78CFBA]),
    v([0x3F54DB31, 0x3F4D9F02, 0xBE94A033, 0xBF7FB10F]),
    v([0xBEC3EF0B, 0x3F3DAEF9, 0x3F61C597, 0xBE16408A]),
    v([0xBF7B14BF, 0xBF039C3E, 0x3F22679A, 0x3F710908]),
    v([0x3F3504F3, 0x3E78CFC8, 0xBF61C599, 0xBF2BEB49]),
    v([0x3F0E39DD, 0x3F710908, 0xBDC8BD41, 0xBF7D3AAC]),
    v([0xBEC3EF0B, 0x3F4D9F05, 0x3F45E403, 0xBEDAE880]),
    v([0xBF7B14BE, 0xBD48FAD2, 0x3F74FA0C, 0x3F039C3C]),
    v([0xBF3504F3, 0xBF5B9419, 0x3E94A03D, 0x3F7FB10F]),
    v([0x3E47C5BC, 0xBF676BD8, 0xBF22679A, 0x3F187FBF]),
    v([0x3F6C835E, 0xBE16408A, 0xBF7EC46D, 0xBEAC7CD4]),
    v([0x3F54DB31, 0x3F3DAEF9, 0xBEF15AED, 0xBF7853F8]),
    v([0x3F3504F3, 0x3E164085, 0xBF74FA0B, 0xBEDAE87B]),
    v([0x3F54DB31, 0x3F2BEB49, 0xBF226799, 0xBF5B9419]),
    v([0x3EC3EF1B, 0x3F7853F8, 0xBDC8BD41, 0xBF7FB10F]),
    v([0xBE47C5C6, 0x3F710909, 0x3EF15AE7, 0xBF4D9F04]),
    v([0xBF3504F1, 0x3F187FBF, 0x3F61C597, 0xBEAC7CD6]),
    v([0xBF7B14BE, 0x3D48FB58, 0x3F7EC46E, 0x3E78CFC8]),
    v([0xBF6C8360, 0xBF039C3E, 0x3F45E405, 0x3F3DAEF9]),
    v([0xBF0E39DC, 0xBF676BD7, 0x3E94A03D, 0x3F7D3AAC]),
    v([0x3F3504F3, 0x3D48FB29, 0xBF7EC46D, 0xBE16407F]),
    v([0x3F7B14BF, 0x3E78CFC8, 0xBF74FA0B, 0xBEAC7CD4]),
    v([0x3F6C835F, 0x3EDAE881, 0xBF61C599, 0xBF039C3E]),
    v([0x3F54DB31, 0x3F187FC0, 0xBF45E404, 0xBF2BEB49]),
    v([0x3F3504F7, 0x3F3DAEF9, 0xBF226799, 0xBF4D9F02]),
    v([0x3F0E39DD, 0x3F5B941A, 0xBEF15AED, 0xBF676BD7]),
    v([0x3EC3EF1B, 0x3F710908, 0xBE94A033, 0xBF7853F8]),
    v([0x3E47C5C8, 0x3F7D3AAC, 0xBDC8BD41, 0xBF7FB10F]),
    v([0x3F3504F3, 0xBD48FB41, 0xBF7EC46D, 0x3E164080]),
    v([0x3F7B14BE, 0xBE78CFCD, 0xBF74FA0A, 0x3EAC7CD5]),
    v([0x3F6C835E, 0xBEDAE880, 0xBF61C597, 0x3F039C3E]),
    v([0x3F54DB31, 0xBF187FC1, 0xBF45E402, 0x3F2BEB4C]),
    v([0x3F3504F3, 0xBF3DAEFB, 0xBF22679A, 0x3F4D9F05]),
    v([0x3F0E39D9, 0xBF5B941A, 0xBEF15AE8, 0x3F676BDA]),
    v([0x3EC3EF15, 0xBF710909, 0xBE94A02D, 0x3F7853F8]),
    v([0x3E47C5BC, 0xBF7D3AAC, 0xBDC8BD1A, 0x3F7FB10F]),
    v([0x3F3504F3, 0xBE16408A, 0xBF74FA0A, 0x3EDAE88A]),
    v([0x3F54DB31, 0xBF2BEB4B, 0xBF22679A, 0x3F5B941B]),
    v([0x3EC3EF15, 0xBF7853F8, 0xBDC8BD1A, 0x3F7FB10F]),
    v([0xBE47C5C2, 0xBF710908, 0x3EF15AE9, 0x3F4D9F02]),
    v([0xBF3504F3, 0xBF187FBE, 0x3F61C599, 0x3EAC7CD3]),
    v([0xBF7B14BF, 0xBD48FAD2, 0x3F7EC46D, 0xBE78CFCD]),
    v([0xBF6C835E, 0x3F039C3E, 0x3F45E403, 0xBF3DAEFB]),
    v([0xBF0E39D6, 0x3F676BDA, 0x3E94A030, 0xBF7D3AAC]),
    v([0x3F3504F3, 0xBE78CFCD, 0xBF61C597, 0x3F2BEB4C]),
    v([0x3F0E39D9, 0xBF710909, 0xBDC8BD1A, 0x3F7D3AAC]),
    v([0xBEC3EF18, 0xBF4D9F02, 0x3F45E405, 0x3EDAE881]),
    v([0xBF7B14BF, 0x3D48FB58, 0x3F74FA0B, 0xBF039C3D]),
    v([0xBF3504F1, 0x3F5B941B, 0x3E94A030, 0xBF7FB10F]),
    v([0x3E47C5C8, 0x3F676BD8, 0xBF226799, 0xBF187FBE]),
    v([0x3F6C835F, 0x3E164085, 0xBF7EC46D, 0x3EAC7CD5]),
    v([0x3F54DB31, 0xBF3DAEFB, 0xBEF15AE8, 0x3F7853F8]),
    v([0x3F3504F3, 0xBEAC7CD6, 0xBF45E402, 0x3F5B941B]),
    v([0x3E47C5BC, 0xBF7D3AAC, 0x3EF15AE9, 0x3F2BEB49]),
    v([0xBF6C8360, 0xBD48FAD2, 0x3F74FA0B, 0xBF187FC1]),
    v([0xBF0E39D6, 0x3F7853F8, 0xBDC8BD41, 0xBF676BD7]),
    v([0x3F3504F7, 0x3EDAE881, 0xBF7EC46D, 0x3E78CFDB]),
    v([0x3F54DB31, 0xBF4D9F04, 0xBE94A02D, 0x3F7FB10F]),
    v([0xBEC3EF18, 0xBF3DAEF9, 0x3F61C599, 0x3E164085]),
    v([0xBF7B14BE, 0x3F039C3E, 0x3F226799, 0xBF710909]),
    v([0x3F3504F3, 0xBEDAE880, 0xBF22679A, 0x3F7853F8]),
    v([0xBE47C5C2, 0xBF4D9F02, 0x3F61C599, 0x3D48FB29]),
    v([0xBF6C835E, 0x3F3DAEF9, 0x3E94A030, 0xBF7D3AAC]),
    v([0x3F0E39DD, 0x3F039C3C, 0xBF7EC46D, 0x3EAC7CD5]),
    v([0x3F3504F3, 0xBF710909, 0x3DC8BD5D, 0x3F5B941A]),
    v([0xBF54DB32, 0xBE16407F, 0x3F74FA0B, 0xBF2BEB4B]),
    v([0xBEC3EF0B, 0x3F7FB10F, 0xBEF15AED, 0xBF187FBE]),
    v([0x3F7B14BF, 0xBE78CFCD, 0xBF45E402, 0x3F676BDA]),
    v([0x3F3504F3, 0xBF039C3D, 0xBEF15AE8, 0x3F7FB10F]),
    v([0xBF0E39DC, 0xBEDAE87B, 0x3F7EC46D, 0xBF187FC1]),
    v([0xBEC3EF0B, 0x3F7D3AAC, 0xBF226799, 0xBEAC7CD4]),
    v([0x3F7B14BE, 0xBF2BEB4B, 0xBE94A02D, 0x3F7853F8]),
    v([0xBF3504F3, 0xBE78CFBA, 0x3F74FA0B, 0xBF3DAEFB]),
    v([0xBE47C5C6, 0x3F710908, 0xBF45E404, 0xBE16407F]),
    v([0x3F6C835E, 0xBF4D9F04, 0xBDC8BD1A, 0x3F676BD8]),
    v([0xBF54DB32, 0xBD48FAD2, 0x3F61C597, 0xBF5B941A]),
    v([0x3F3504F3, 0xBF187FC1, 0xBE94A02D, 0x3F710908]),
    v([0xBF54DB32, 0x3D48FB58, 0x3F45E403, 0xBF7853F8]),
    v([0x3EC3EF1B, 0x3F039C3C, 0xBF7EC46D, 0x3F2BEB4C]),
    v([0x3E47C5BC, 0xBF676BD7, 0x3F61C599, 0xBE16408A]),
    v([0xBF3504F1, 0x3F7D3AAD, 0xBEF15AED, 0xBEDAE87B]),
    v([0x3F7B14BE, 0xBF3DAEFB, 0xBDC8BD1A, 0x3F5B941A]),
    v([0xBF6C8360, 0x3E78CFDB, 0x3F226799, 0xBF7FB10F]),
    v([0x3F0E39DD, 0x3EAC7CD3, 0xBF74FA0A, 0x3F4D9F05]),
    v([0x3F3504F3, 0xBF2BEB4B, 0xBDC8BD1A, 0x3F4D9F02]),
    v([0xBF7B14BF, 0x3F039C3E, 0x3E94A030, 0xBF676BD7]),
    v([0x3F6C835F, 0xBEAC7CD6, 0xBEF15AE8, 0x3F7853F8]),
    v([0xBF54DB32, 0x3E164080, 0x3F226799, 0xBF7FB10F]),
    v([0x3F3504F7, 0x3D48FB29, 0xBF45E402, 0x3F7D3AAD]),
    v([0xBF0E39DC, 0xBE78CFBA, 0x3F61C597, 0xBF710909]),
    v([0x3EC3EF1B, 0x3EDAE881, 0xBF74FA0A, 0x3F5B941B]),
    v([0xBE47C5C2, 0xBF187FBE, 0x3F7EC46D, 0xBF3DAEFB]),
    v([0x3F3504F3, 0xBF3DAEFB, 0x3DC8BD5D, 0x3F187FC0]),
    v([0xBF7B14BE, 0x3F5B941B, 0xBE94A033, 0xBEDAE87B]),
    v([0x3F6C835E, 0xBF710909, 0x3EF15AE9, 0x3E78CFC8]),
    v([0xBF54DB30, 0x3F7D3AAD, 0xBF226799, 0xBD48FAD2]),
    v([0x3F3504F3, 0xBF7FB10F, 0x3F45E405, 0xBE16408A]),
    v([0xBF0E39D6, 0x3F7853F8, 0xBF61C599, 0x3EAC7CD5]),
    v([0x3EC3EF15, 0xBF676BD7, 0x3F74FA0C, 0xBF039C3D]),
    v([0xBE47C5C6, 0x3F4D9F02, 0xBF7EC46D, 0x3F2BEB4C]),
    v([0x3F3504F3, 0xBF4D9F04, 0x3E94A03D, 0x3EAC7CD3]),
    v([0xBF54DB30, 0x3F7FB10F, 0xBF45E404, 0x3E78CFDB]),
    v([0x3EC3EF15, 0xBF5B9419, 0x3F7EC46E, 0xBF3DAEFB]),
    v([0x3E47C5C8, 0x3EDAE881, 0xBF61C597, 0x3F7D3AAD]),
    v([0xBF3504F3, 0x3E164080, 0x3EF15AE7, 0xBF676BD7]),
    v([0x3F7B14BF, 0xBF2BEB4B, 0x3DC8BD5D, 0x3F039C3C]),
    v([0xBF6C835E, 0x3F7853F8, 0xBF226799, 0x3D48FB58]),
    v([0x3F0E39D9, 0xBF710908, 0x3F74FA0C, 0xBF187FC1]),
    v([0x3F3504F3, 0xBF5B941A, 0x3EF15AE9, 0x3D48FB29]),
    v([0xBF0E39D6, 0x3F676BD8, 0xBF7EC46D, 0x3F4D9F05]),
    v([0xBEC3EF18, 0xBE16407F, 0x3F226799, 0xBF710908]),
    v([0x3F7B14BF, 0xBF3DAEFB, 0x3E94A03D, 0x3E78CFC8]),
    v([0xBF3504F1, 0x3F7853F8, 0xBF74FA0B, 0x3F2BEB4C]),
    v([0xBE47C5C2, 0xBEAC7CD4, 0x3F45E403, 0xBF7D3AAC]),
    v([0x3F6C835F, 0xBF187FC1, 0x3DC8BD5D, 0x3EDAE881]),
    v([0xBF54DB30, 0x3F7FB10F, 0xBF61C599, 0x3F039C3E]),
    v([0x3F3504F3, 0xBF676BD8, 0x3F22679A, 0xBE78CFCD]),
    v([0xBE47C5C6, 0x3F187FC0, 0xBF61C597, 0x3F7FB10F]),
    v([0xBF6C8360, 0x3F2BEB4C, 0xBE94A033, 0xBE16407F]),
    v([0x3F0E39D9, 0xBF5B9419, 0x3F7EC46D, 0xBF710909]),
    v([0x3F3504F7, 0xBEAC7CD6, 0xBDC8BD1A, 0x3F039C3C]),
    v([0xBF54DB30, 0x3F7D3AAC, 0xBF74FA0B, 0x3F3DAEF9]),
    v([0xBEC3EF18, 0xBD48FAD2, 0x3EF15AE7, 0xBF4D9F02]),
    v([0x3F7B14BE, 0xBF7853F8, 0x3F45E405, 0xBEDAE880]),
    v([0x3F3504F3, 0xBF710909, 0x3F45E405, 0xBF039C3D]),
    v([0x3E47C5C8, 0x3E164085, 0xBEF15AE8, 0x3F3DAEF9]),
    v([0xBF6C835E, 0x3F7FB10F, 0xBF74FA0B, 0x3F4D9F05]),
    v([0xBF0E39DC, 0x3E78CFDB, 0x3DC8BD35, 0xBEDAE87B]),
    v([0x3F3504F3, 0xBF676BD7, 0x3F7EC46D, 0xBF7853F8]),
    v([0x3F54DB31, 0xBF187FC1, 0x3E94A03D, 0x3D48FB29]),
    v([0xBEC3EF0B, 0x3F2BEB49, 0xBF61C597, 0x3F7D3AAC]),
    v([0xBF7B14BF, 0x3F5B941B, 0xBF226799, 0x3EAC7CD5]),
    v([0x3F3504F3, 0xBF7853F8, 0x3F61C599, 0xBF3DAEFB]),
    v([0x3F0E39DD, 0xBEAC7CD6, 0x3DC8BD5D, 0x3E164085]),
    v([0xBEC3EF0B, 0x3F187FC0, 0xBF45E402, 0x3F676BD8]),
    v([0xBF7B14BE, 0x3F7FB10F, 0xBF74FA0B, 0x3F5B941B]),
    v([0xBF3504F3, 0x3F039C3E, 0xBE94A033, 0x3D48FB58]),
    v([0x3E47C5BC, 0xBEDAE87B, 0x3F226799, 0xBF4D9F02]),
    v([0x3F6C835E, 0xBF7D3AAC, 0x3F7EC46E, 0xBF710909]),
    v([0x3F54DB31, 0xBF2BEB4B, 0x3EF15AE9, 0xBE78CFCD]),
    v([0x3F3504F3, 0xBF7D3AAC, 0x3F74FA0C, 0xBF676BD8]),
    v([0x3F54DB31, 0xBF3DAEFB, 0x3F22679A, 0xBF039C3D]),
    v([0x3EC3EF1B, 0xBE78CFCD, 0x3DC8BD5D, 0x3D48FB29]),
    v([0xBE47C5C6, 0x3EAC7CD3, 0xBEF15AE8, 0x3F187FC0]),
    v([0xBF3504F1, 0x3F4D9F02, 0xBF61C597, 0x3F710908]),
    v([0xBF7B14BE, 0x3F7FB10F, 0xBF7EC46D, 0x3F7853F8]),
    v([0xBF6C8360, 0x3F5B941B, 0xBF45E404, 0x3F2BEB4C]),
    v([0xBF0E39DC, 0x3EDAE88A, 0xBE94A033, 0x3E164080]),
    v([0x3F3504F3, 0xBF7FB10F, 0x3F7EC46E, 0xBF7D3AAC]),
    v([0x3F7B14BF, 0xBF7853F8, 0x3F74FA0C, 0xBF710909]),
    v([0x3F6C835F, 0xBF676BD8, 0x3F61C599, 0xBF5B941A]),
    v([0x3F54DB31, 0xBF4D9F04, 0x3F45E405, 0xBF3DAEFB]),
    v([0x3F3504F7, 0xBF2BEB4B, 0x3F22679A, 0xBF187FC1]),
    v([0x3F0E39DD, 0xBF039C3D, 0x3EF15AE9, 0xBEDAE880]),
    v([0x3EC3EF1B, 0xBEAC7CD6, 0x3E94A03D, 0xBE78CFCD]),
    v([0x3E47C5C8, 0xBE16408A, 0x3DC8BD5D, 0xBD48FB41]),
];

const WINDOW_TABLE: [V; 128] = [
    v([0x00000000, 0xB7800074, 0xB7800074, 0xB7800074]),
    v([0xB7800074, 0xB7800074, 0xB7800074, 0xB8000074]),
    v([0xB8000074, 0xB8000074, 0xB8000074, 0xB83FFF9B]),
    v([0xB83FFF9B, 0xB87FFFD5, 0xB87FFFD5, 0xB8A00008]),
    v([0xB8A00008, 0xB8C00025, 0xB8E00041, 0xB8E00041]),
    v([0xB8FFFFD5, 0xB90FFFF9, 0xB9200008, 0xB9300016]),
    v([0xB94FFFEE, 0xB95FFFFD, 0xB980000D, 0xB987FFF2]),
    v([0xB9980000, 0xB9A8000F, 0xB9C00002, 0xB9D00011]),
    v([0xB9E80004, 0xB9F7FFF0, 0xBA0C0007, 0xBA180000]),
    v([0xBA23FFFA, 0xBA340008, 0xBA440006, 0xBA540003]),
    v([0xBA680004, 0xBA7C0005, 0xBA880003, 0xBA920003]),
    v([0xBA9DFFFD, 0xBAA9FFFF, 0xBAB60002, 0xBAC20004]),
    v([0xBACFFFFF, 0xBADE0004, 0xBAE9FFFD, 0xBAFA0003]),
    v([0xBB03FFFF, 0xBB0B0001, 0xBB130000, 0xBB1A0002]),
    v([0xBB210000, 0xBB28FFFE, 0xBB300001, 0xBB36FFFE]),
    v([0xBB3E0000, 0xBB440001, 0xBB49FFFE, 0xBB4FFFFF]),
    v([0x3B550000, 0x3B5A0000, 0x3B5DFFFF, 0x3B610002]),
    v([0x3B62FFFF, 0x3B640000, 0x3B640000, 0x3B62FFFF]),
    v([0x3B600001, 0x3B5CFFFE, 0x3B570002, 0x3B4FFFFF]),
    v([0x3B480001, 0x3B3CFFFF, 0x3B310001, 0x3B230002]),
    v([0x3B11FFFF, 0x3AFDFFFE, 0x3AD40003, 0x3AA5FFFC]),
    v([0x3A640000, 0x39E80004, 0xB8000074, 0xBA0FFFF9]),
    v([0xBA900002, 0xBADE0004, 0xBB190001, 0xBB44FFFE]),
    v([0xBB740002, 0xBB930000, 0xBBAD8000, 0xBBC87FFF]),
    v([0xBBE58000, 0xBC01C001, 0xBC114000, 0xBC214000]),
    v([0xBC31C000, 0xBC42C000, 0xBC540000, 0xBC65C000]),
    v([0xBC77C000, 0xBC850000, 0xBC8E2000, 0xBC974000]),
    v([0xBCA06000, 0xBCA98000, 0xBCB28000, 0xBCBB4000]),
    v([0xBCC3E000, 0xBCCC4000, 0xBCD44000, 0xBCDBE000]),
    v([0xBCE32000, 0xBCE9C000, 0xBCEFE000, 0xBCF54000]),
    v([0xBCFA2000, 0xBCFE0000, 0xBD009000, 0xBD01B000]),
    v([0xBD025000, 0xBD027000, 0xBD020000, 0xBD00F000]),
    v([0x3CFEA000, 0x3CFA0000, 0x3CF40000, 0x3CECA000]),
    v([0x3CE3C000, 0x3CD96000, 0x3CCD8000, 0x3CBFE000]),
    v([0x3CB0C000, 0x3CA00000, 0x3C8D6000, 0x3C728000]),
    v([0x3C468001, 0x3C174000, 0x3BC90000, 0x3B390000]),
    v([0xBA340008, 0xBB8FFFFF, 0xBC084000, 0xBC4B8000]),
    v([0xBC88E000, 0xBCAD8000, 0xBCD38000, 0xBCFAC000]),
    v([0xBD11A000, 0xBD267000, 0xBD3BC000, 0xBD517000]),
    v([0xBD679000, 0xBD7DF000, 0xBD8A4800, 0xBD95A000]),
    v([0xBDA10800, 0xBDAC6800, 0xBDB7B800, 0xBDC2E800]),
    v([0xBDCDE800, 0xBDD8B800, 0xBDE33800, 0xBDED6800]),
    v([0xBDF73000, 0xBE004400, 0xBE04AC00, 0xBE08CC00]),
    v([0xBE0C9800, 0xBE100C00, 0xBE132000, 0xBE15C400]),
    v([0xBE17FC00, 0xBE19B800, 0xBE1AF000, 0xBE1B9C00]),
    v([0xBE1BB800, 0xBE1B3C00, 0xBE1A1C00, 0xBE185800]),
    v([0xBE15E000, 0xBE12B400, 0xBE0ECC00, 0xBE0A2000]),
    v([0xBE04B000, 0xBDFCE000, 0xBDEEC000, 0xBDDEF000]),
    v([0x3DCD7000, 0x3DBA3800, 0x3DA54000, 0x3D8E8800]),
    v([0x3D6C0000, 0x3D377000, 0x3CFEA000, 0x3C874000]),
    v([0x3A8BFFFE, 0xBC797FFF, 0xBD04A000, 0xBD4E4000]),
    v([0xBD8DA800, 0xBDB5D000, 0xBDDF9000, 0xBE057000]),
    v([0xBE1BDC00, 0xBE32FC00, 0xBE4AD000, 0xBE635000]),
    v([0xBE7C6C00, 0xBE8B0E00, 0xBE982C00, 0xBEA58A00]),
    v([0xBEB32200, 0xBEC0EC00, 0xBECEE400, 0xBEDD0200]),
    v([0xBEEB4000, 0xBEF99600, 0xBF03FF00, 0xBF0B3800]),
    v([0xBF127100, 0xBF19A800, 0xBF20D800, 0xBF27FE00]),
    v([0xBF2F1500, 0xBF361900, 0xBF3D0600, 0xBF43D900]),
    v([0xBF4A8D00, 0xBF511E00, 0xBF578A00, 0xBF5DCA00]),
    v([0xBF63DD00, 0xBF69BE00, 0xBF6F6900, 0xBF74DC00]),
    v([0xBF7A1300, 0xBF7F0A00, 0xBF81DF00, 0xBF841680]),
    v([0xBF862A00, 0xBF881780, 0xBF89DF00, 0xBF8B7E00]),
    v([0xBF8CF480, 0xBF8E4180, 0xBF8F6380, 0xBF905A00]),
    v([0xBF912480, 0xBF91C300, 0xBF923400, 0xBF927800]),
    v([0x3F928F00, 0x3F927800, 0x3F923400, 0x3F91C300]),
    v([0x3F912480, 0x3F905A00, 0x3F8F6380, 0x3F8E4180]),
    v([0x3F8CF480, 0x3F8B7E00, 0x3F89DF00, 0x3F881780]),
    v([0x3F862A00, 0x3F841680, 0x3F81DF00, 0x3F7F0A00]),
    v([0x3F7A1300, 0x3F74DC00, 0x3F6F6900, 0x3F69BE00]),
    v([0x3F63DD00, 0x3F5DCA00, 0x3F578A00, 0x3F511E00]),
    v([0x3F4A8D00, 0x3F43D900, 0x3F3D0600, 0x3F361900]),
    v([0x3F2F1500, 0x3F27FE00, 0x3F20D800, 0x3F19A800]),
    v([0x3F127100, 0x3F0B3800, 0x3F03FF00, 0x3EF99600]),
    v([0x3EEB4000, 0x3EDD0200, 0x3ECEE400, 0x3EC0EC00]),
    v([0x3EB32200, 0x3EA58A00, 0x3E982C00, 0x3E8B0E00]),
    v([0x3E7C6C00, 0x3E635000, 0x3E4AD000, 0x3E32FC00]),
    v([0x3E1BDC00, 0x3E057000, 0x3DDF9000, 0x3DB5D000]),
    v([0x3D8DA800, 0x3D4E4000, 0x3D04A000, 0x3C797FFF]),
    v([0xBA8BFFFE, 0xBC874000, 0xBCFEA000, 0xBD377000]),
    v([0xBD6C0000, 0xBD8E8800, 0xBDA54000, 0xBDBA3800]),
    v([0x3DCD7000, 0x3DDEF000, 0x3DEEC000, 0x3DFCE000]),
    v([0x3E04B000, 0x3E0A2000, 0x3E0ECC00, 0x3E12B400]),
    v([0x3E15E000, 0x3E185800, 0x3E1A1C00, 0x3E1B3C00]),
    v([0x3E1BB800, 0x3E1B9C00, 0x3E1AF000, 0x3E19B800]),
    v([0x3E17FC00, 0x3E15C400, 0x3E132000, 0x3E100C00]),
    v([0x3E0C9800, 0x3E08CC00, 0x3E04AC00, 0x3E004400]),
    v([0x3DF73000, 0x3DED6800, 0x3DE33800, 0x3DD8B800]),
    v([0x3DCDE800, 0x3DC2E800, 0x3DB7B800, 0x3DAC6800]),
    v([0x3DA10800, 0x3D95A000, 0x3D8A4800, 0x3D7DF000]),
    v([0x3D679000, 0x3D517000, 0x3D3BC000, 0x3D267000]),
    v([0x3D11A000, 0x3CFAC000, 0x3CD38000, 0x3CAD8000]),
    v([0x3C88E000, 0x3C4B8000, 0x3C084000, 0x3B8FFFFF]),
    v([0x3A340008, 0xBB390000, 0xBBC90000, 0xBC174000]),
    v([0xBC468001, 0xBC728000, 0xBC8D6000, 0xBCA00000]),
    v([0xBCB0C000, 0xBCBFE000, 0xBCCD8000, 0xBCD96000]),
    v([0xBCE3C000, 0xBCECA000, 0xBCF40000, 0xBCFA0000]),
    v([0x3CFEA000, 0x3D00F000, 0x3D020000, 0x3D027000]),
    v([0x3D025000, 0x3D01B000, 0x3D009000, 0x3CFE0000]),
    v([0x3CFA2000, 0x3CF54000, 0x3CEFE000, 0x3CE9C000]),
    v([0x3CE32000, 0x3CDBE000, 0x3CD44000, 0x3CCC4000]),
    v([0x3CC3E000, 0x3CBB4000, 0x3CB28000, 0x3CA98000]),
    v([0x3CA06000, 0x3C974000, 0x3C8E2000, 0x3C850000]),
    v([0x3C77C000, 0x3C65C000, 0x3C540000, 0x3C42C000]),
    v([0x3C31C000, 0x3C214000, 0x3C114000, 0x3C01C001]),
    v([0x3BE58000, 0x3BC87FFF, 0x3BAD8000, 0x3B930000]),
    v([0x3B740002, 0x3B44FFFE, 0x3B190001, 0x3ADE0004]),
    v([0x3A900002, 0x3A0FFFF9, 0x38000074, 0xB9E80004]),
    v([0xBA640000, 0xBAA5FFFC, 0xBAD40003, 0xBAFDFFFE]),
    v([0xBB11FFFF, 0xBB230002, 0xBB310001, 0xBB3CFFFF]),
    v([0xBB480001, 0xBB4FFFFF, 0xBB570002, 0xBB5CFFFE]),
    v([0xBB600001, 0xBB62FFFF, 0xBB640000, 0xBB640000]),
    v([0xBB62FFFF, 0xBB610002, 0xBB5DFFFF, 0xBB5A0000]),
    v([0x3B550000, 0x3B4FFFFF, 0x3B49FFFE, 0x3B440001]),
    v([0x3B3E0000, 0x3B36FFFE, 0x3B300001, 0x3B28FFFE]),
    v([0x3B210000, 0x3B1A0002, 0x3B130000, 0x3B0B0001]),
    v([0x3B03FFFF, 0x3AFA0003, 0x3AE9FFFD, 0x3ADE0004]),
    v([0x3ACFFFFF, 0x3AC20004, 0x3AB60002, 0x3AA9FFFF]),
    v([0x3A9DFFFD, 0x3A920003, 0x3A880003, 0x3A7C0005]),
    v([0x3A680004, 0x3A540003, 0x3A440006, 0x3A340008]),
    v([0x3A23FFFA, 0x3A180000, 0x3A0C0007, 0x39F7FFF0]),
    v([0x39E80004, 0x39D00011, 0x39C00002, 0x39A8000F]),
    v([0x39980000, 0x3987FFF2, 0x3980000D, 0x395FFFFD]),
    v([0x394FFFEE, 0x39300016, 0x39200008, 0x390FFFF9]),
    v([0x38FFFFD5, 0x38E00041, 0x38E00041, 0x38C00025]),
    v([0x38A00008, 0x38A00008, 0x387FFFD5, 0x387FFFD5]),
    v([0x383FFF9B, 0x383FFF9B, 0x38000074, 0x38000074]),
    v([0x38000074, 0x38000074, 0x37800074, 0x37800074]),
    v([0x37800074, 0x37800074, 0x37800074, 0x37800074]),
];

