//! Writing tracks out as WAV files.
//!
//! ```text
//! <output>/<game title>/
//!   tracks.csv                  every track: where it came from, format, rate, length, loop
//!   SOUND/BGM/TITLE.wav         a file that was one sound keeps its place and name
//!   SOUND/SE.PAK/001.wav ...    a file holding several becomes a folder of them
//! ```
//!
//! Looping tracks get their loop points in a "smpl" chunk (read by samplers, editors and
//! game-audio tools). They can also be written played out: the loop twice, then a
//! 10-second fade, which is what vgmstream plays by default.

use std::fs::{self, File};
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::codecs;
use crate::disc::Game;
use crate::scan::sanitize;
use crate::track::Track;

/// How looping tracks are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loops {
    /// Once through, with the loop points marked.
    Once,
    /// The loop played twice, then faded out over 10 seconds (vgmstream's default).
    Twice,
}

const FADE_SECONDS: u64 = 10;

/// Streams 16-bit PCM into a WAV file, filling in the sizes when done.
pub struct WavWriter<W: Write + Seek> {
    out: W,
    data_bytes: u64,
    data_size_at: u64,
}

impl<W: Write + Seek> WavWriter<W> {
    pub fn new(mut out: W, channels: u16, rate: u32, loop_points: Option<(u64, u64)>) -> io::Result<Self> {
        let block = channels as u32 * 2;
        out.write_all(b"RIFF\0\0\0\0WAVEfmt ")?;
        out.write_all(&16u32.to_le_bytes())?;
        out.write_all(&1u16.to_le_bytes())?; // PCM
        out.write_all(&channels.to_le_bytes())?;
        out.write_all(&rate.to_le_bytes())?;
        out.write_all(&(rate * block).to_le_bytes())?;
        out.write_all(&(block as u16).to_le_bytes())?;
        out.write_all(&16u16.to_le_bytes())?;
        let mut header = 36u64;
        if let Some((start, end)) = loop_points {
            let mut smpl = Vec::with_capacity(68);
            smpl.extend_from_slice(b"smpl");
            smpl.extend_from_slice(&60u32.to_le_bytes());
            for v in [0u32, 0, 1_000_000_000 / rate.max(1), 60, 0, 0, 0, 1, 0] {
                smpl.extend_from_slice(&v.to_le_bytes());
            }
            // one forward loop; the end is inclusive
            for v in [0u32, 0, start as u32, (end.max(1) - 1) as u32, 0, 0] {
                smpl.extend_from_slice(&v.to_le_bytes());
            }
            out.write_all(&smpl)?;
            header += smpl.len() as u64;
        }
        out.write_all(b"data\0\0\0\0")?;
        Ok(WavWriter { out, data_bytes: 0, data_size_at: header + 4 })
    }

    pub fn samples(&mut self, s: &[i16]) -> io::Result<()> {
        let mut bytes = Vec::with_capacity(s.len() * 2);
        for v in s {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        self.out.write_all(&bytes)?;
        self.data_bytes += bytes.len() as u64;
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<W> {
        let data = self.data_bytes.min(u32::MAX as u64 - self.data_size_at) as u32;
        self.out.seek(SeekFrom::Start(4))?;
        self.out.write_all(&(self.data_size_at as u32 - 4 + data).to_le_bytes())?;
        self.out.seek(SeekFrom::Start(self.data_size_at))?;
        self.out.write_all(&data.to_le_bytes())?;
        self.out.flush()?;
        Ok(self.out)
    }
}

fn loop_points(t: &Track) -> Option<(u64, u64)> {
    Some((t.loop_start?, t.loop_end?))
}

/// Decodes a track into a WAV in memory, at most `max_seconds` long (for previews).
pub fn wav_bytes(game: &Game, track: &Track, max_seconds: u32) -> io::Result<Vec<u8>> {
    let mut w = WavWriter::new(io::Cursor::new(Vec::new()), track.channels, track.sample_rate, None)?;
    let limit = max_seconds as u64 * track.sample_rate as u64 * track.channels as u64;
    let mut written = 0u64;
    codecs::decode_track(game, track, &mut |s| {
        let take = (s.len() as u64).min(limit - written) as usize;
        w.samples(&s[..take])?;
        written += take as u64;
        Ok(written < limit)
    })?;
    Ok(w.finish()?.into_inner())
}

pub struct Summary {
    pub folder: PathBuf,
    pub written: usize,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
}

/// Writes the tracks into `<out>/<game title>/`. `progress` gets (tracks done, total, path).
pub fn extract(
    game: &Game,
    tracks: &[Track],
    out: &Path,
    loops: Loops,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize, usize, &str),
) -> Result<Summary, String> {
    let folder = out.join(sanitize(&game.title));
    fs::create_dir_all(&folder).map_err(|e| format!("Can't create {}: {e}", folder.display()))?;
    let mut summary = Summary { folder: folder.clone(), written: 0, skipped: vec![], failed: vec![] };
    let mut csv = String::from("file,source,offset,format,channels,sample_rate,seconds,loop_start,loop_end,note\n");

    for (i, t) in tracks.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        progress(i, tracks.len(), &t.path);
        let source = &game.entries[t.entry].path;
        let opt = |v: Option<u64>| v.map(|v| v.to_string()).unwrap_or_default();
        csv.push_str(&format!(
            "{},{},0x{:X},{},{},{},{:.3},{},{},{}\n",
            csv_field(&t.path), csv_field(source), t.offset, t.format, t.channels, t.sample_rate, t.duration(),
            opt(t.loop_start), opt(t.loop_end), csv_field(t.note.as_deref().unwrap_or(""))
        ));
        if let Some(note) = &t.note {
            summary.skipped.push(format!("{}: {note}", t.path));
            continue;
        }
        let dest = t.path.split('/').fold(folder.clone(), |p, part| p.join(part));
        match write_track(game, t, &dest, loops, cancel) {
            Ok(()) => summary.written += 1,
            Err(e) if e.to_string() == "cancelled" => return Err("cancelled".into()),
            Err(e) => {
                let _ = fs::remove_file(&dest);
                summary.failed.push(format!("{}: {e}", t.path));
            }
        }
    }
    fs::write(folder.join("tracks.csv"), csv).map_err(|e| e.to_string())?;
    progress(tracks.len(), tracks.len(), "");
    Ok(summary)
}

fn write_track(game: &Game, t: &Track, dest: &Path, loops: Loops, cancel: &AtomicBool) -> io::Result<()> {
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir)?;
    }
    let file = BufWriter::with_capacity(1 << 20, File::create(dest)?);
    let mut w = WavWriter::new(file, t.channels, t.sample_rate, loop_points(t))?;
    let ch = t.channels.max(1) as usize;
    let render = loops == Loops::Twice && loop_points(t).is_some();
    let (ls, le) = loop_points(t).unwrap_or((0, 0));
    // Twice: keep the loop's samples to play again afterwards.
    let mut body: Vec<i16> = Vec::new();
    let mut pos = 0u64; // samples per channel so far
    codecs::decode_track(game, t, &mut |s| {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other("cancelled"));
        }
        if !render {
            w.samples(s)?;
            return Ok(true);
        }
        let frames = (s.len() / ch) as u64;
        let keep = frames.min(le.saturating_sub(pos)) as usize;
        w.samples(&s[..keep * ch])?;
        let from = ls.saturating_sub(pos).min(keep as u64) as usize;
        body.extend_from_slice(&s[from * ch..keep * ch]);
        pos += frames;
        Ok(pos < le)
    })?;
    if render && !body.is_empty() {
        // Second time through the loop, then keep looping while fading out.
        w.samples(&body)?;
        let fade = (FADE_SECONDS * t.sample_rate as u64) as usize;
        let frames = body.len() / ch;
        let mut out = Vec::with_capacity(fade.min(1 << 20) * ch);
        for i in 0..fade {
            let gain = 1.0 - i as f64 / fade as f64;
            let f = i % frames;
            for c in 0..ch {
                out.push((body[f * ch + c] as f64 * gain) as i16);
            }
            if out.len() >= (1 << 20) {
                w.samples(&out)?;
                out.clear();
            }
        }
        w.samples(&out)?;
    }
    w.finish()?;
    Ok(())
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header() {
        let mut w = WavWriter::new(io::Cursor::new(Vec::new()), 2, 48000, None).unwrap();
        w.samples(&[1, -1, 2, -2]).unwrap();
        let b = w.finish().unwrap().into_inner();
        assert_eq!(b.len(), 44 + 8);
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 36 + 8);
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 8);
        assert_eq!(u32::from_le_bytes(b[28..32].try_into().unwrap()), 48000 * 4);
    }

    #[test]
    fn wav_loop_chunk() {
        let mut w = WavWriter::new(io::Cursor::new(Vec::new()), 1, 22050, Some((100, 500))).unwrap();
        w.samples(&[0; 10]).unwrap();
        let b = w.finish().unwrap().into_inner();
        assert_eq!(&b[36..40], b"smpl");
        assert_eq!(u32::from_le_bytes(b[36 + 8 + 0x2c..36 + 8 + 0x30].try_into().unwrap()), 100);
        assert_eq!(u32::from_le_bytes(b[36 + 8 + 0x30..36 + 8 + 0x34].try_into().unwrap()), 499);
        assert_eq!(&b[104..108], b"data");
        assert_eq!(u32::from_le_bytes(b[108..112].try_into().unwrap()), 20);
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 104 + 20);
    }
}
