//! Decoding tracks to 16-bit PCM.
//!
//! Most codecs here are "frame" codecs (a fixed number of bytes decodes to a fixed number
//! of samples) laid out with an interleave: `interleave` bytes of channel 0, then of
//! channel 1, and so on. `run_interleaved` does that layout once for all of them; a codec
//! only decodes single frames (`FrameDecoder`). Codecs with their own framing implement
//! `decode` directly.
//!
//! The math of each codec follows vgmstream (github.com/vgmstream/vgmstream, src/coding)
//! closely enough to produce identical samples, which the tests check.

pub mod adx;
pub mod aica;
pub mod ea_xa;
pub mod ima;
pub mod mpc3;
pub mod mtaf;
pub mod ongakukan;
pub mod pcm;
pub mod psx;
pub mod tac;
pub mod ubi;
pub mod vorbis;

use std::io;

use crate::disc::{Game, Reader};
use crate::track::{Data, Track};

#[derive(Debug, Clone, Default)]
pub enum Codec {
    Psx(psx::Params),
    Adx(adx::Params),
    Pcm(pcm::Params),
    Ima(ima::Params),
    EaXa(ea_xa::Params),
    Mtaf(mtaf::Params),
    Tac(tac::Params),
    Vorbis(vorbis::Params),
    Aica(aica::Params),
    Mpc3(mpc3::Params),
    Ubi(ubi::Params),
    Ongakukan(ongakukan::Params),
    /// Parts played one after another, each decoded on its own (AIX segments).
    Segmented(Vec<Track>),
    /// Can't be decoded; the track's `note` says why.
    #[default]
    None,
}

/// Receives decoded, channel-interleaved samples; returns `false` to stop early.
pub type Sink<'a> = &'a mut dyn FnMut(&[i16]) -> io::Result<bool>;

/// Decodes a track, opening what it needs from the game (both files of a dual-file
/// stereo pair).
pub fn decode_track(game: &Game, track: &Track, sink: Sink) -> io::Result<()> {
    let mut r = game.reader(&game.entries[track.data.entry])?;
    let Some(right) = &track.dual else { return decode(track, &mut r, sink) };
    // Two mono files: decode each, then interleave.
    let mut left_track = track.clone();
    left_track.dual = None;
    left_track.channels = 1;
    let channel = |t: &Track, r: &mut Reader| -> io::Result<Vec<i16>> {
        let mut v = Vec::new();
        decode(t, r, &mut |s| {
            v.extend_from_slice(s);
            Ok(true)
        })?;
        Ok(v)
    };
    let l = channel(&left_track, &mut r)?;
    let r2 = channel(right, &mut game.reader(&game.entries[right.data.entry])?)?;
    let n = l.len().min(r2.len());
    for start in (0..n).step_by(1 << 16) {
        let end = (start + (1 << 16)).min(n);
        let out: Vec<i16> = (start..end).flat_map(|i| [l[i], r2[i]]).collect();
        if !sink(&out)? {
            break;
        }
    }
    Ok(())
}

/// Decodes a track. `r` reads the file holding the track's data (`track.data.entry`).
pub fn decode(track: &Track, r: &mut Reader, sink: Sink) -> io::Result<()> {
    if let Codec::Segmented(parts) = &track.codec {
        for part in parts {
            let mut more = true;
            decode(part, r, &mut |b| {
                more = sink(b)?;
                Ok(more)
            })?;
            if !more {
                break;
            }
        }
        return Ok(());
    }
    let mut s = Stream::new(r, &track.data);
    match &track.codec {
        Codec::Psx(p) => psx::decode(track, &mut s, p, sink),
        Codec::Adx(p) => adx::decode(track, &mut s, p, sink),
        Codec::Pcm(p) => pcm::decode(track, &mut s, p, sink),
        Codec::Ima(p) => ima::decode(track, &mut s, p, sink),
        Codec::EaXa(p) => ea_xa::decode(track, &mut s, p, sink),
        Codec::Mtaf(p) => mtaf::decode(track, &mut s, p, sink),
        Codec::Tac(p) => tac::decode(track, &mut s, p, sink),
        Codec::Vorbis(p) => vorbis::decode(track, &mut s, p, sink),
        Codec::Aica(p) => aica::decode(track, &mut s, p, sink),
        Codec::Mpc3(p) => mpc3::decode(track, &mut s, p, sink),
        Codec::Ubi(p) => ubi::decode(track, &mut s, p, sink),
        Codec::Ongakukan(p) => ongakukan::decode(track, &mut s, p, sink),
        Codec::Segmented(_) => unreachable!("handled above"),
        Codec::None => Err(io::Error::other(track.note.clone().unwrap_or_else(|| "can't be decoded".into()))),
    }
}

/// A track's data as one continuous run of bytes (joining its blocks, if it has any).
pub struct Stream<'a> {
    r: &'a mut Reader,
    /// (file offset, size) pieces.
    pieces: Vec<(u64, u64)>,
    len: u64,
    /// Where decoders start afresh (see `Data::resets`).
    pub resets: Vec<u64>,
}

impl<'a> Stream<'a> {
    pub fn new(r: &'a mut Reader, data: &Data) -> Stream<'a> {
        let pieces = match &data.blocks {
            Some(b) => b.to_vec(),
            None => vec![(data.offset, data.size)],
        };
        let len = pieces.iter().map(|p| p.1).sum();
        let resets = data.resets.as_ref().map(|r| r.to_vec()).unwrap_or_default();
        Stream { r, pieces, len, resets }
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    /// Fills `buf` from position `pos` of the data (zeros past the end).
    pub fn read(&mut self, pos: u64, buf: &mut [u8]) -> io::Result<()> {
        buf.fill(0);
        let mut want = pos;
        let mut done = 0usize;
        let mut base = 0u64;
        for &(off, size) in &self.pieces {
            if done == buf.len() {
                break;
            }
            if want < base + size {
                let within = want - base;
                let n = ((size - within) as usize).min(buf.len() - done);
                self.r.read_at(off + within, &mut buf[done..done + n])?;
                done += n;
                want += n as u64;
            }
            base += size;
        }
        Ok(())
    }

    pub fn bytes(&mut self, pos: u64, len: usize) -> io::Result<Vec<u8>> {
        let mut v = vec![0; len];
        self.read(pos, &mut v)?;
        Ok(v)
    }
}

/// Decodes one channel's frames.
pub trait FrameDecoder {
    /// Bytes per frame.
    fn frame_bytes(&self) -> usize;
    /// Samples per frame.
    fn frame_samples(&self) -> usize;
    /// Decodes one frame into `out` (`frame_samples` long).
    fn decode(&mut self, frame: &[u8], out: &mut [i16]);
    /// Starts afresh (sample history cleared) at a segment boundary.
    fn reset(&mut self) {}
}

/// Runs frame decoders (one per channel) over interleaved data: `interleave` bytes per
/// channel in turn, the last row shorter if the data is. The first block of each channel
/// starts `first_skip` bytes in. Mono data ignores the interleave.
pub fn run_interleaved<D: FrameDecoder>(
    s: &mut Stream,
    mut decoders: Vec<D>,
    interleave: u64,
    first_skip: u64,
    total_samples: u64,
    sink: Sink,
) -> io::Result<()> {
    let ch = decoders.len().max(1);
    let fb = decoders[0].frame_bytes() as u64;
    let fs = decoders[0].frame_samples();
    // Mono reads big rows, except when decoding restarts mid-stream (checked per row).
    let il = if ch == 1 && s.resets.is_empty() { (0x10000 / fb).max(1) * fb } else if ch == 1 { fb } else { interleave.max(fb) };
    let row = il * ch as u64;
    // Read several rows at a time.
    let rows_per_read = (0x40000 / row).max(1);
    let end = s.len();
    let mut left = total_samples;
    let mut pos = 0u64;
    let mut first = first_skip > 0 && ch > 1;
    let mut frame_out = vec![0i16; fs];
    let resets = std::mem::take(&mut s.resets);
    let mut next_reset = 0usize;
    while pos < end && left > 0 {
        let chunk_len = (row * rows_per_read).min(end - pos);
        let buf = s.bytes(pos, chunk_len as usize)?;
        let mut out: Vec<i16> = Vec::with_capacity((chunk_len / fb) as usize * fs);
        let mut rpos = 0u64;
        while rpos < chunk_len && left > 0 {
            while next_reset < resets.len() && resets[next_reset] <= pos + rpos {
                if resets[next_reset] == pos + rpos {
                    decoders.iter_mut().for_each(FrameDecoder::reset);
                }
                next_reset += 1;
            }
            let row_len = (chunk_len - rpos).min(row);
            let block = if row_len < row { row_len / ch as u64 } else { il };
            let skip = if first { first_skip.min(block) } else { 0 };
            let frames = ((block - skip) / fb) as usize;
            if frames == 0 {
                break;
            }
            let n = ((frames * fs) as u64).min(left) as usize;
            let start = out.len();
            out.resize(start + n * ch, 0);
            for (c, dec) in decoders.iter_mut().enumerate() {
                let base = (rpos + c as u64 * block + skip) as usize;
                for f in 0..frames {
                    let at = base + f * fb as usize;
                    dec.decode(&buf[at..at + fb as usize], &mut frame_out);
                    for (k, smp) in frame_out.iter().enumerate() {
                        let i = f * fs + k;
                        if i < n {
                            out[start + i * ch + c] = *smp;
                        }
                    }
                }
            }
            left -= n as u64;
            rpos += block * ch as u64;
            first = false;
        }
        if !out.is_empty() && !sink(&out)? {
            return Ok(());
        }
        if rpos == 0 {
            break; // only a partial frame left
        }
        pos += rpos;
    }
    Ok(())
}

pub fn clamp16(s: i32) -> i16 {
    s.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

