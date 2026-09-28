//! Ogg Vorbis, decoded with the `lewton` crate. vgmstream uses libvorbis (vorbisfile's
//! `ov_read_float`), then turns each float sample into 16 bits by truncating
//! `sample * 32767.0` and clamping, and reorders channels from Vorbis' order (for 3+
//! channels): both are done the same way here. lewton's float math isn't libvorbis'
//! line by line, so a sample can rarely come out 1 off.

use std::io::{self, Cursor};

use lewton::inside_ogg::OggStreamReader;

use super::{Sink, Stream, clamp16};
use crate::track::Track;

#[derive(Debug, Clone, Default)]
pub struct Params {
    /// Output channels in the stream's own order (some games' encoders pre-ordered them).
    pub disable_reordering: bool,
}

/// Vorbis channel order to vgmstream's (vgmstream's xiph_channel_map).
const XIPH_MAP: [[usize; 8]; 8] = [
    [0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 0, 0, 0, 0, 0, 0],
    [0, 2, 1, 0, 0, 0, 0, 0],
    [0, 1, 2, 3, 0, 0, 0, 0],
    [0, 2, 1, 3, 4, 0, 0, 0],
    [0, 2, 1, 5, 3, 4, 0, 0],
    [0, 2, 1, 6, 5, 3, 4, 0],
    [0, 2, 1, 7, 5, 6, 3, 4],
];

/// vgmstream's float to 16-bit conversion: `(int)(f * 32767.0f)`, clamped.
pub fn to_i16(f: f32) -> i16 {
    clamp16((f * 32767.0) as i32)
}

pub fn decode(track: &Track, s: &mut Stream, p: &Params, sink: Sink) -> io::Result<()> {
    let data = s.bytes(0, s.len() as usize)?;
    let mut rdr = OggStreamReader::new(Cursor::new(data)).map_err(|e| io::Error::other(format!("Vorbis: {e:?}")))?;
    let ch = rdr.ident_hdr.audio_channels as usize;
    if ch == 0 || ch != track.channels as usize {
        return Err(io::Error::other("Vorbis: unexpected channel count"));
    }
    let map: Vec<usize> = (0..ch).map(|c| if ch > 8 || p.disable_reordering { c } else { XIPH_MAP[ch - 1][c] }).collect();
    let mut left = track.samples;
    while left > 0 {
        let pck: Vec<Vec<f32>> = match rdr.read_dec_packet_generic() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e) => return Err(io::Error::other(format!("Vorbis: {e:?}"))),
        };
        let n = pck.first().map(|c| c.len()).unwrap_or(0).min(left as usize);
        if n == 0 {
            continue;
        }
        let mut out = vec![0i16; n * ch];
        for (c, &src) in map.iter().enumerate() {
            for (i, v) in pck[src].iter().take(n).enumerate() {
                out[i * ch + c] = to_i16(*v);
            }
        }
        left -= n as u64;
        if !sink(&out)? {
            return Ok(());
        }
    }
    Ok(())
}
