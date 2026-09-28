//! A piece of audio found on the disc.

use std::sync::Arc;

use serde::Serialize;

use crate::codecs::Codec;

#[derive(Debug, Clone, Serialize)]
pub struct Track {
    pub id: usize,
    /// Index of the file (in `Game::entries`) the track was found in.
    pub entry: usize,
    /// Where the track (its header) starts in that file.
    pub offset: u64,
    /// Short format name shown to the user, e.g. "VAG", "ADS", "HD/BD", "ADX".
    pub format: &'static str,
    /// Name to save it under (without extension); unique within its file.
    pub name: String,
    /// Where it is saved, relative to the game's output folder ("SOUND/SE.PAK/003.wav").
    pub path: String,
    pub channels: u16,
    pub sample_rate: u32,
    /// Samples per channel.
    pub samples: u64,
    /// Loop points in samples (start inclusive, end exclusive), when the format has them.
    pub loop_start: Option<u64>,
    pub loop_end: Option<u64>,
    /// Why it can't be extracted, if it can't.
    pub note: Option<String>,
    /// A mono file that may pair with a file named like it into stereo, as vgmstream's
    /// "dual file stereo" (BGM_L.VAG + BGM_R.VAG, SONG.V0 + SONG.V1, ...).
    #[serde(skip)]
    pub dual_ok: bool,
    /// The right channel, from the paired file (this track is then the left one).
    #[serde(skip)]
    pub dual: Option<Box<Track>>,
    #[serde(skip)]
    pub data: Data,
    #[serde(skip)]
    pub codec: Codec,
}

impl Track {
    /// A track with the basics set; parsers fill in the rest.
    pub fn new(entry: usize, offset: u64, format: &'static str, channels: u16, sample_rate: u32, samples: u64, data: Data, codec: Codec) -> Track {
        Track {
            id: 0,
            entry,
            offset,
            format,
            name: String::new(),
            path: String::new(),
            channels,
            sample_rate,
            samples,
            loop_start: None,
            loop_end: None,
            note: None,
            dual_ok: false,
            dual: None,
            data,
            codec,
        }
    }

    /// Sets loop points (ignored unless they make sense).
    pub fn looped(mut self, start: u64, end: u64) -> Track {
        let end = end.min(self.samples);
        if start < end {
            self.loop_start = Some(start);
            self.loop_end = Some(end);
        }
        self
    }

    pub fn duration(&self) -> f64 {
        self.samples as f64 / self.sample_rate.max(1) as f64
    }
}

/// Where a track's encoded data is. Usually one range of a file; formats that break
/// their data up with block headers list the pieces, which are read back to back.
#[derive(Debug, Clone, Default)]
pub struct Data {
    pub entry: usize,
    pub offset: u64,
    pub size: u64,
    /// (offset, size) pieces in the file, in order; when set, `offset`/`size` are unused.
    pub blocks: Option<Arc<Vec<(u64, u64)>>>,
    /// Positions in the (joined) data where decoding starts afresh, as for streams made of
    /// separately encoded segments (sorted).
    pub resets: Option<Arc<Vec<u64>>>,
}

impl Data {
    pub fn at(entry: usize, offset: u64, size: u64) -> Data {
        Data { entry, offset, size, blocks: None, resets: None }
    }

    pub fn blocks(entry: usize, blocks: Vec<(u64, u64)>) -> Data {
        let size = blocks.iter().map(|b| b.1).sum();
        let offset = blocks.first().map(|b| b.0).unwrap_or(0);
        Data { entry, offset, size, blocks: Some(Arc::new(blocks)), resets: None }
    }

}
