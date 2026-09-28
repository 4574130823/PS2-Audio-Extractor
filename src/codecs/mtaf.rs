//! MTAF: Konami's ADPCM (Metal Gear Solid 3), a mix of IMA and Yamaha ADPCM. Follows
//! vgmstream's coding/mtaf_decoder.c.
//!
//! Channels come in stereo tracks: each 256-sample frame of a track is a 0x10 header
//! (step index and history per channel) then 0x80 bytes per channel, low nibble first.
//! Tracks' frames follow each other (track 0, track 1, ...) for each 256 samples.

use std::io;

use super::{Sink, Stream, clamp16};
use crate::track::Track;

#[derive(Debug, Clone, Default)]
pub struct Params {}

/// Bytes of one track frame (two channels).
pub const FRAME: u64 = 0x110;
pub const FRAME_SAMPLES: u64 = 256;

const STEP_INDEXES: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

const STEP_SIZES: [[i16; 16]; 32] = [
    [1, 5, 9, 13, 16, 20, 24, 28, -1, -5, -9, -13, -16, -20, -24, -28],
    [2, 6, 11, 15, 20, 24, 29, 33, -2, -6, -11, -15, -20, -24, -29, -33],
    [2, 7, 13, 18, 23, 28, 34, 39, -2, -7, -13, -18, -23, -28, -34, -39],
    [3, 9, 15, 21, 28, 34, 40, 46, -3, -9, -15, -21, -28, -34, -40, -46],
    [3, 11, 18, 26, 33, 41, 48, 56, -3, -11, -18, -26, -33, -41, -48, -56],
    [4, 13, 22, 31, 40, 49, 58, 67, -4, -13, -22, -31, -40, -49, -58, -67],
    [5, 16, 26, 37, 48, 59, 69, 80, -5, -16, -26, -37, -48, -59, -69, -80],
    [6, 19, 31, 44, 57, 70, 82, 95, -6, -19, -31, -44, -57, -70, -82, -95],
    [7, 22, 38, 53, 68, 83, 99, 114, -7, -22, -38, -53, -68, -83, -99, -114],
    [9, 27, 45, 63, 81, 99, 117, 135, -9, -27, -45, -63, -81, -99, -117, -135],
    [10, 32, 53, 75, 96, 118, 139, 161, -10, -32, -53, -75, -96, -118, -139, -161],
    [12, 38, 64, 90, 115, 141, 167, 193, -12, -38, -64, -90, -115, -141, -167, -193],
    [15, 45, 76, 106, 137, 167, 198, 228, -15, -45, -76, -106, -137, -167, -198, -228],
    [18, 54, 91, 127, 164, 200, 237, 273, -18, -54, -91, -127, -164, -200, -237, -273],
    [21, 65, 108, 152, 195, 239, 282, 326, -21, -65, -108, -152, -195, -239, -282, -326],
    [25, 77, 129, 181, 232, 284, 336, 388, -25, -77, -129, -181, -232, -284, -336, -388],
    [30, 92, 153, 215, 276, 338, 399, 461, -30, -92, -153, -215, -276, -338, -399, -461],
    [36, 109, 183, 256, 329, 402, 476, 549, -36, -109, -183, -256, -329, -402, -476, -549],
    [43, 130, 218, 305, 392, 479, 567, 654, -43, -130, -218, -305, -392, -479, -567, -654],
    [52, 156, 260, 364, 468, 572, 676, 780, -52, -156, -260, -364, -468, -572, -676, -780],
    [62, 186, 310, 434, 558, 682, 806, 930, -62, -186, -310, -434, -558, -682, -806, -930],
    [73, 221, 368, 516, 663, 811, 958, 1106, -73, -221, -368, -516, -663, -811, -958, -1106],
    [87, 263, 439, 615, 790, 966, 1142, 1318, -87, -263, -439, -615, -790, -966, -1142, -1318],
    [104, 314, 523, 733, 942, 1152, 1361, 1571, -104, -314, -523, -733, -942, -1152, -1361, -1571],
    [124, 374, 623, 873, 1122, 1372, 1621, 1871, -124, -374, -623, -873, -1122, -1372, -1621, -1871],
    [148, 445, 743, 1040, 1337, 1634, 1932, 2229, -148, -445, -743, -1040, -1337, -1634, -1932, -2229],
    [177, 531, 885, 1239, 1593, 1947, 2301, 2655, -177, -531, -885, -1239, -1593, -1947, -2301, -2655],
    [210, 632, 1053, 1475, 1896, 2318, 2739, 3161, -210, -632, -1053, -1475, -1896, -2318, -2739, -3161],
    [251, 753, 1255, 1757, 2260, 2762, 3264, 3766, -251, -753, -1255, -1757, -2260, -2762, -3264, -3766],
    [299, 897, 1495, 2093, 2692, 3290, 3888, 4486, -299, -897, -1495, -2093, -2692, -3290, -3888, -4486],
    [356, 1068, 1781, 2493, 3206, 3918, 4631, 5343, -356, -1068, -1781, -2493, -3206, -3918, -4631, -5343],
    [424, 1273, 2121, 2970, 3819, 4668, 5516, 6365, -424, -1273, -2121, -2970, -3819, -4668, -5516, -6365],
];

/// Decodes one channel (`ch` 0 or 1) of a track frame into 256 samples.
pub fn frame_into(frame: &[u8], ch: usize, out: &mut [i16]) {
    let mut step = i16::from_le_bytes([frame[4 + ch * 2], frame[5 + ch * 2]]) as i32;
    let mut hist = i16::from_le_bytes([frame[8 + ch * 4], frame[9 + ch * 4]]) as i32;
    step = step.clamp(0, 31);
    for (i, o) in out.iter_mut().take(FRAME_SAMPLES as usize).enumerate() {
        let b = frame[0x10 + 0x80 * ch + i / 2];
        let nibble = ((b >> if i & 1 == 0 { 0 } else { 4 }) & 0x0f) as usize;
        hist = clamp16(hist + STEP_SIZES[step as usize][nibble] as i32) as i32;
        *o = hist as i16;
        step = (step + STEP_INDEXES[nibble]).clamp(0, 31);
    }
}

pub fn decode(track: &Track, s: &mut Stream, _p: &Params, sink: Sink) -> io::Result<()> {
    let ch = track.channels.max(1) as usize;
    let tracks = ch.div_ceil(2);
    let row = FRAME * tracks as u64;
    let mut left = track.samples;
    let mut pos = 0u64;
    let mut buf = [0i16; FRAME_SAMPLES as usize];
    while left > 0 {
        let rows = (0x40000 / row).max(1);
        let data = s.bytes(pos, (row * rows) as usize)?;
        let mut out = Vec::with_capacity(rows as usize * FRAME_SAMPLES as usize * ch);
        for r in 0..rows as usize {
            if left == 0 {
                break;
            }
            let n = FRAME_SAMPLES.min(left) as usize;
            let start = out.len();
            out.resize(start + n * ch, 0);
            for c in 0..ch {
                let at = r * row as usize + (c / 2) * FRAME as usize;
                frame_into(&data[at..at + FRAME as usize], c % 2, &mut buf);
                for (k, v) in buf.iter().take(n).enumerate() {
                    out[start + k * ch + c] = *v;
                }
            }
            left -= n as u64;
        }
        if !sink(&out)? {
            return Ok(());
        }
        pos += row * rows;
    }
    Ok(())
}
