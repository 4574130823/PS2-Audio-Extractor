//! Finding the audio in a game's files.
//!
//! PS2 games mostly keep sound inside big archive files rather than as loose files, so
//! every file is searched for every format's signatures at once (Aho-Corasick), and each
//! hit is parsed and checked before it counts; standalone files are just hits at offset 0.
//! Whatever a found track covers is skipped, so data inside a sound is never mistaken for
//! another one.
//!
//! Beyond headers, the scan also looks inside:
//! * MPEG program streams (PS2 .PSS and CRI .SFD videos): the audio streams are demuxed
//!   into in-memory files and searched like any other file;
//! * zlib/gzip-compressed data: unpacked and searched;
//! * data no header claimed: runs of PS-ADPCM frames with no header at all ("headerless",
//!   as game banks often store them), when enabled.

use std::collections::{BTreeMap, HashSet};
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};

use aho_corasick::AhoCorasick;

use crate::codecs::{Codec, psx};
use crate::disc::{Entry, Game};
use crate::formats::{Ctx, Found, PARSERS, split_ext};
use crate::track::{Data, Track};

const CHUNK: usize = 8 << 20;

#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Look for PS-ADPCM with no header in data nothing else claimed.
    pub headerless: bool,
    /// Sample rate to give headerless audio (it doesn't say).
    pub headerless_rate: u32,
}

impl Default for Options {
    fn default() -> Self {
        Options { headerless: true, headerless_rate: 22050 }
    }
}

pub struct ScanResult {
    pub tracks: Vec<Track>,
    /// The game's files plus the in-memory ones made while scanning.
    pub entries: Vec<Entry>,
}

/// Extra signatures that aren't formats: containers the scan opens.
const GZIP: &[u8] = b"\x1f\x8b\x08";
const ZLIB: [&[u8]; 4] = [b"\x78\x01", b"\x78\x5e", b"\x78\x9c", b"\x78\xda"];

struct Matcher {
    ac: AhoCorasick,
    /// Pattern index -> what it is.
    kinds: Vec<Kind>,
    longest: usize,
}

#[derive(Clone, Copy)]
enum Kind {
    Format(usize),
    Gzip,
    Zlib,
}

fn matcher() -> Matcher {
    let mut patterns: Vec<&[u8]> = Vec::new();
    let mut kinds = Vec::new();
    for (i, p) in PARSERS.iter().enumerate() {
        for m in p.magics {
            patterns.push(m);
            kinds.push(Kind::Format(i));
        }
    }
    patterns.push(GZIP);
    kinds.push(Kind::Gzip);
    for z in ZLIB {
        patterns.push(z);
        kinds.push(Kind::Zlib);
    }
    let longest = patterns.iter().map(|p| p.len()).max().unwrap_or(1);
    Matcher { ac: AhoCorasick::new(&patterns).expect("signatures"), kinds, longest }
}

/// Searches all of the game's files. `progress` gets (bytes done, file being searched).
pub fn scan(game: &Game, opts: Options, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, &str)) -> Result<ScanResult, String> {
    let m = matcher();
    let mut entries = game.entries.clone();
    let mut tracks = Vec::new();
    let mut done = 0u64;
    let mut i = 0;
    // The list grows as containers are opened; those get scanned too.
    while i < entries.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let path = entries[i].path.clone();
        let original = i < game.entries.len();
        progress(done, &path);
        let depth = path.matches(" [").count();
        let skip = is_bd_of_pair(&entries, &entries[i]);
        let mut extra = Vec::new();
        let found = if skip {
            vec![]
        } else {
            scan_entry(game, &entries, i, &m, opts, depth < 3, &mut extra, cancel, &mut |d| {
                if original {
                    progress(done + d, &path)
                }
            })
            .map_err(|e| format!("{path}: {e}"))?
        };
        if original {
            done += entries[i].size;
        }
        tracks.extend(name_tracks(&entries[i], found));
        entries.extend(extra);
        i += 1;
    }
    let mut tracks = pair_dual(tracks, &entries);
    unique_paths(&mut tracks);
    for (id, t) in tracks.iter_mut().enumerate() {
        t.id = id;
    }
    progress(done, "");
    Ok(ScanResult { tracks, entries })
}

#[allow(clippy::too_many_arguments)]
fn scan_entry(
    game: &Game,
    entries: &[Entry],
    index: usize,
    m: &Matcher,
    opts: Options,
    open_containers: bool,
    extra: &mut Vec<Entry>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<Vec<Found>> {
    let entry = &entries[index];
    let r = game.reader(entry)?;
    if r.size < 0x10 {
        return Ok(vec![]);
    }
    let mut ctx = Ctx { game, entries, entry: index, r, names: None };
    let size = ctx.size();

    // Videos: demux their audio and search that instead.
    if open_containers && ctx.is(0, b"\x00\x00\x01\xba")? {
        if let Some(streams) = demux_mpeg_ps(&mut ctx)? {
            for (id, data) in streams {
                extra.push(Entry::memory(format!("{} [audio {id:02X}]", entry.path), data));
            }
            return Ok(vec![]);
        }
    }

    // 1. Signature hits, as (header offset -> parsers to try), and containers to open.
    let mut hits: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    let mut packed: Vec<(u64, bool)> = Vec::new(); // (offset, gzip)
    let overlap = m.longest - 1;
    let mut buf = vec![0u8; CHUNK + overlap];
    let mut pos = 0u64;
    while pos < size {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other("cancelled"));
        }
        let n = ctx.r.read_at(pos, &mut buf)?;
        let last = pos + CHUNK as u64 >= size;
        for hit in m.ac.find_overlapping_iter(&buf[..n]) {
            if hit.start() >= CHUNK && !last {
                continue; // found again from the next chunk
            }
            let abs = pos + hit.start() as u64;
            match m.kinds[hit.pattern().as_usize()] {
                Kind::Format(p) => {
                    let parser = PARSERS[p];
                    let header = match parser.locate {
                        Some(locate) => locate(&mut ctx, abs)?,
                        None => abs.checked_sub(parser.magic_at),
                    };
                    if let Some(h) = header {
                        let list = hits.entry(h).or_default();
                        if !list.contains(&p) {
                            list.push(p);
                        }
                    }
                }
                Kind::Gzip if open_containers => packed.push((abs, true)),
                Kind::Zlib if open_containers && zlib_header_ok(&buf[hit.start()..n]) => packed.push((abs, false)),
                _ => {}
            }
        }
        progress(pos + n.min(CHUNK) as u64);
        pos += CHUNK as u64;
    }
    // Formats known only by extension are tried at the start of files named so, after any
    // format whose signature is there (a signature is the stronger evidence, and formats
    // like raw .INT would take anything).
    let ext = ctx.ext();
    for (p, parser) in PARSERS.iter().enumerate() {
        if parser.exts.contains(&ext.as_str()) {
            let list = hits.entry(0).or_default();
            if !list.contains(&p) {
                list.push(p);
            }
        }
    }

    // 2. Parse them in order, skipping whatever an earlier track covers.
    let mut found = Vec::new();
    let mut covered: Vec<(u64, u64)> = Vec::new();
    let mut covered_to = 0u64;
    for (&off, parsers) in &hits {
        if off < covered_to {
            continue;
        }
        for &p in parsers {
            let parsed = (PARSERS[p].parse)(&mut ctx, off)?;
            if let Some(end) = parsed.iter().map(|f| f.end).max() {
                covered_to = covered_to.max(end);
                covered.push((off, end));
                found.extend(parsed);
                break;
            }
        }
    }

    // 3. Compressed data outside what's been claimed: unpack and search it too.
    let mut unpacked = 0;
    for (off, gzip) in packed {
        if unpacked >= 4096 || covered.iter().any(|&(a, b)| off >= a && off < b) {
            continue;
        }
        if let Some(data) = inflate(&mut ctx, off, gzip)? {
            let end = off + data.1;
            extra.push(Entry::memory(format!("{} [{}@{off:X}]", entry.path, if gzip { "gz" } else { "zlib" }), data.0));
            covered.push((off, end));
            unpacked += 1;
        }
    }

    // 4. Headerless PS-ADPCM in what's left.
    if opts.headerless {
        covered.sort_unstable();
        found.extend(find_headerless(&mut ctx, &covered, opts.headerless_rate, cancel)?);
    }
    Ok(found)
}

// ---------------------------------------------------------------------------------------
// MPEG program streams (.PSS, .SFD, ...)
// ---------------------------------------------------------------------------------------

/// Audio streams of an MPEG-1/2 program stream: private stream 1 (PS2 .PSS audio, which
/// carries an ADS stream) and MPEG audio stream ids (CRI .SFD carries ADX there). Per
/// packet sub-headers are detected and removed. Returns None if it isn't a clean stream.
fn demux_mpeg_ps(ctx: &mut Ctx) -> io::Result<Option<Vec<(u8, Vec<u8>)>>> {
    let size = ctx.size();
    let mut streams: BTreeMap<u8, Vec<Vec<u8>>> = BTreeMap::new();
    let mut pos = 0u64;
    let mut packets = 0;
    while pos + 6 <= size {
        let h = ctx.bytes(pos, 14)?;
        if h[0..3] != [0, 0, 1] {
            // Resync to the next start code within a little distance, else give up.
            let window = ctx.bytes(pos, 0x800)?;
            match window.windows(3).skip(1).position(|w| w == [0, 0, 1]) {
                Some(p) => {
                    pos += p as u64 + 1;
                    continue;
                }
                None => break,
            }
        }
        let id = h[3];
        match id {
            0xba => {
                // pack header: MPEG-2 (01xx) or MPEG-1 (0010)
                pos += if h[4] >> 6 == 1 { 14 + (h[13] & 7) as u64 } else { 12 };
            }
            0xb9 => break, // end code
            _ if id >= 0xbb => {
                let len = u16::from_be_bytes([h[4], h[5]]) as u64;
                let body = pos + 6;
                if id == 0xbd || (0xc0..=0xdf).contains(&id) {
                    let pkt = ctx.bytes(body, len as usize)?;
                    let skip = pes_header_len(&pkt);
                    if skip < pkt.len() {
                        streams.entry(id).or_default().push(pkt[skip..].to_vec());
                    }
                }
                packets += 1;
                pos = body + len;
            }
            _ => pos += 4,
        }
    }
    if packets < 2 || streams.is_empty() {
        return Ok(None);
    }
    let mut out = Vec::new();
    for (id, pkts) in streams {
        // PS2 private streams may prefix each packet with a small sub-header: find how
        // long by where the ADS header sits in the first packet.
        let strip = if id == 0xbd {
            pkts[0].windows(4).take(16).position(|w| w == b"SShd").unwrap_or(0)
        } else {
            0
        };
        let data: Vec<u8> = pkts.iter().flat_map(|p| p.get(strip..).unwrap_or(&[]).iter().copied()).collect();
        if !data.is_empty() {
            out.push((id, data));
        }
    }
    Ok(Some(out))
}

/// Length of a PES packet's header (after the 6-byte start code and length).
fn pes_header_len(p: &[u8]) -> usize {
    if p.first().map(|b| b >> 6) == Some(2) {
        // MPEG-2: flags, flags, header data length
        return 3 + p.get(2).copied().unwrap_or(0) as usize;
    }
    // MPEG-1: stuffing, optional STD buffer, timestamps
    let mut i = 0;
    while i < p.len() && p[i] == 0xff {
        i += 1;
    }
    if i < p.len() && p[i] >> 6 == 1 {
        i += 2;
    }
    match p.get(i).map(|b| b >> 4) {
        Some(2) => i + 5,
        Some(3) => i + 10,
        _ => i + 1,
    }
}

// ---------------------------------------------------------------------------------------
// Compressed data
// ---------------------------------------------------------------------------------------

fn zlib_header_ok(b: &[u8]) -> bool {
    b.len() >= 2 && (u16::from_be_bytes([b[0], b[1]]) % 31 == 0) && b[1] & 0x20 == 0
}

/// Unpacks zlib or gzip data at `off`: (data, compressed length). Only keeps results
/// that are plausibly real (a clean end, and not tiny).
fn inflate(ctx: &mut Ctx, off: u64, gzip: bool) -> io::Result<Option<(Vec<u8>, u64)>> {
    const MAX_OUT: u64 = 512 << 20;
    let avail = ctx.size() - off;
    // Read the compressed stream in pieces through a small adapter.
    struct Src<'a, 'b> {
        ctx: &'a mut Ctx<'b>,
        pos: u64,
        end: u64,
    }
    impl Read for Src<'_, '_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = (buf.len() as u64).min(self.end - self.pos).min(1 << 16) as usize;
            if n == 0 {
                return Ok(0);
            }
            let got = self.ctx.r.read_at(self.pos, &mut buf[..n])?;
            self.pos += got as u64;
            Ok(got)
        }
    }
    let src = Src { ctx, pos: off, end: off + avail };
    let mut out = Vec::new();
    let result = if gzip {
        let mut d = flate2::read::GzDecoder::new(src);
        let r = (&mut d).take(MAX_OUT).read_to_end(&mut out);
        r.map(|_| d.into_inner().pos - off)
    } else {
        let mut d = flate2::read::ZlibDecoder::new(src);
        let r = (&mut d).take(MAX_OUT).read_to_end(&mut out);
        let used = d.total_in();
        r.map(|_| used)
    };
    match result {
        Ok(used) if out.len() >= 64 && (out.len() as u64) < MAX_OUT => Ok(Some((out, used.max(1)))),
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------------------
// Headerless PS-ADPCM
// ---------------------------------------------------------------------------------------

/// Runs of PS-ADPCM with no header, in the parts of the file not `covered`: a silent
/// frame (the SPU's usual lead-in) followed by at least `MIN_FRAMES` valid, non-silent
/// frames and an end flag. Aligned to 16 bytes, like the SPU needs.
fn find_headerless(ctx: &mut Ctx, covered: &[(u64, u64)], rate: u32, cancel: &AtomicBool) -> io::Result<Vec<Found>> {
    const MIN_FRAMES: u64 = 64; // ~1800 samples; shorter runs are too likely by chance
    let size = ctx.size() / 16 * 16;
    let mut out = Vec::new();
    let mut gaps = Vec::new();
    let mut at = 0u64;
    for &(a, b) in covered {
        if a > at {
            gaps.push((at, a));
        }
        at = at.max(b);
    }
    if at < size {
        gaps.push((at, size));
    }
    for (a, b) in gaps {
        let mut pos = a.next_multiple_of(16);
        while pos + 16 * (MIN_FRAMES + 1) <= b {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::other("cancelled"));
            }
            let chunk_end = (pos + (4 << 20)).min(b);
            let buf = ctx.bytes(pos, (chunk_end - pos) as usize)?;
            let mut i = 0usize;
            let mut advanced = false;
            while i + 32 <= buf.len() {
                if buf[i..i + 16].iter().all(|&x| x == 0) && valid_frame(&buf[i + 16..i + 32]) && !silent(&buf[i + 16..i + 32]) {
                    // Walk the run (may continue past this chunk).
                    let start = pos + i as u64;
                    let (frames, ended) = run_length(ctx, start + 16, b)?;
                    if ended && frames >= MIN_FRAMES {
                        let bytes = (frames + 1) * 16;
                        let data = Data::at(ctx.entry, start, bytes);
                        let t = Track::new(ctx.entry, start, "RAW", 1, rate, psx::bytes_to_samples(bytes, 1), data, Codec::Psx(psx::Params::default()));
                        out.push(Found::new(t, start + bytes));
                        pos = start + bytes;
                        advanced = true;
                        break;
                    }
                }
                i += 16;
            }
            if !advanced {
                pos = pos + (buf.len() as u64 / 16 * 16).saturating_sub(16).max(16);
            }
        }
    }
    Ok(out)
}

fn valid_frame(f: &[u8]) -> bool {
    f[0] >> 4 <= 4 && f[0] & 0x0f <= 12 && matches!(f[1], 0 | 1 | 2 | 3 | 4 | 6 | 7)
}

fn silent(f: &[u8]) -> bool {
    f[2..].iter().all(|&x| x == 0)
}

/// Frames in a PS-ADPCM run starting at `pos`, and whether it ended with an end flag.
fn run_length(ctx: &mut Ctx, mut pos: u64, limit: u64) -> io::Result<(u64, bool)> {
    let mut frames = 0u64;
    let mut real = 0u64;
    while pos + 16 <= limit {
        let buf = ctx.bytes(pos, ((limit - pos).min(0x10000)) as usize / 16 * 16)?;
        for f in buf.chunks_exact(16) {
            if !valid_frame(f) {
                return Ok((frames, false));
            }
            frames += 1;
            if !silent(f) {
                real += 1;
            }
            if f[1] == 1 || f[1] == 3 || f[1] == 7 {
                // Mostly silence isn't worth keeping.
                return Ok((frames, real * 2 > frames));
            }
        }
        pos += buf.len() as u64;
        if buf.is_empty() {
            break;
        }
    }
    Ok((frames, false))
}

// ---------------------------------------------------------------------------------------
// Dual-file stereo
// ---------------------------------------------------------------------------------------

/// Name suffixes of left/right file pairs, as vgmstream's `find_dual_file`: (left, right,
/// whether it replaces the extension too).
const DUAL_PAIRS: &[(&str, &str, bool)] = &[
    ("L", "R", false),
    ("l", "r", false),
    ("left", "right", false),
    ("Left", "Right", false),
    (".V0", ".V1", true),
    (".L", ".R", true),
];

/// Files that would pair with `name` (a file name, no folders): (partner name, whether
/// `name` is the left channel), in vgmstream's order.
fn dual_candidates(name: &str) -> Vec<(String, bool)> {
    let ext_start = name.rfind('.').unwrap_or(name.len());
    let mut out = Vec::new();
    for &(l, r, with_ext) in DUAL_PAIRS {
        for (this, that, is_left) in [(l, r, true), (r, l, false)] {
            if with_ext {
                if name.len() > this.len() && name.ends_with(this) {
                    out.push((format!("{}{that}", &name[..name.len() - this.len()]), is_left));
                }
            } else if ext_start > this.len() && name[..ext_start].ends_with(this) {
                out.push((format!("{}{that}{}", &name[..ext_start - this.len()], &name[ext_start..]), is_left));
            }
        }
    }
    out
}

/// Joins mono files that pair into stereo (vgmstream's "dual file stereo"): same format,
/// rate, codec and length (and loops, except SMPL's right channel, which has none). The
/// pair becomes one track, left channel from the left file.
fn pair_dual(tracks: Vec<Track>, entries: &[Entry]) -> Vec<Track> {
    let mut tracks: Vec<Option<Track>> = tracks.into_iter().map(Some).collect();
    for i in 0..tracks.len() {
        let Some(t) = &tracks[i] else { continue };
        if !t.dual_ok || t.channels != 1 || t.note.is_some() || t.offset != 0 {
            continue;
        }
        let path = &entries[t.entry].path;
        let (dir, name) = path.rsplit_once('/').map(|(d, n)| (format!("{d}/"), n)).unwrap_or((String::new(), path.as_str()));
        for (partner, is_left) in dual_candidates(name) {
            let want = format!("{dir}{partner}");
            let Some(e) = entries.iter().position(|x| x.path.eq_ignore_ascii_case(&want)) else { continue };
            let Some(j) = tracks.iter().position(|o| o.as_ref().is_some_and(|o| o.entry == e && o.offset == 0 && o.dual_ok)) else {
                break; // the partner file isn't the same kind of sound: no pair
            };
            let (a, b) = (tracks[i].as_ref().unwrap(), tracks[j].as_ref().unwrap());
            let same = a.format == b.format
                && a.channels == b.channels
                && a.sample_rate == b.sample_rate
                && a.samples == b.samples
                && format!("{:?}", a.codec) == format!("{:?}", b.codec)
                && (a.format == "SMPL" || (a.loop_start, a.loop_end) == (b.loop_start, b.loop_end));
            if !same {
                break;
            }
            let (li, ri) = if is_left { (i, j) } else { (j, i) };
            let right = tracks[ri].take().unwrap();
            // Loops are the left file's (they match, except SMPL's, which only .V0 has).
            let mut left = tracks[li].take().unwrap();
            left.channels = 2;
            left.dual_ok = false;
            left.dual = Some(Box::new(Track { dual_ok: false, ..right }));
            tracks[li] = Some(left);
            break;
        }
    }
    tracks.into_iter().flatten().collect()
}

// ---------------------------------------------------------------------------------------
// Naming
// ---------------------------------------------------------------------------------------

/// A .BD whose .HD is next to it holds raw samples only: the .HD describes them.
fn is_bd_of_pair(entries: &[Entry], entry: &Entry) -> bool {
    let (stem, ext) = split_ext(&entry.path);
    ext.eq_ignore_ascii_case("bd")
        && entries.iter().any(|e| {
            let (s, x) = split_ext(&e.path);
            s.eq_ignore_ascii_case(stem) && x.eq_ignore_ascii_case("hd")
        })
}

/// Names tracks and gives them their output path. A file that is one sound becomes
/// `DIR/NAME.wav`; a file holding several becomes a folder of numbered sounds
/// (`DIR/FILE.EXT/003 NAME.wav`), using names stored in the headers where there are some.
fn name_tracks(entry: &Entry, found: Vec<Found>) -> Vec<Track> {
    let (stem, _) = split_ext(&entry.path);
    let single = found.len() == 1 && found[0].track.offset == 0;
    let mut used = HashSet::new();
    found
        .into_iter()
        .enumerate()
        .map(|(k, f)| {
            let mut t = f.track;
            let base = if single {
                stem.rsplit('/').next().unwrap_or(stem).to_string()
            } else {
                match &f.label {
                    Some(l) => format!("{:03} {}", k + 1, l),
                    None => format!("{:03}", k + 1),
                }
            };
            let mut name = sanitize(&base);
            let mut n = 2;
            while !used.insert(name.to_lowercase()) {
                name = format!("{} ({n})", sanitize(&base));
                n += 1;
            }
            t.name = name.clone();
            let dir = if single {
                entry.path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default()
            } else {
                entry.path.clone()
            };
            let dir = dir.split('/').filter(|p| !p.is_empty()).map(sanitize).collect::<Vec<_>>().join("/");
            t.path = if dir.is_empty() { format!("{name}.wav") } else { format!("{dir}/{name}.wav") };
            t
        })
        .collect()
}

/// Makes output paths unique across the game ("BGM/TITLE.VAG" and "BGM/TITLE.ADS" would
/// both be "BGM/TITLE.wav"): later ones get their format added.
fn unique_paths(tracks: &mut [Track]) {
    let mut used = HashSet::new();
    for t in tracks.iter_mut() {
        if used.insert(t.path.to_lowercase()) {
            continue;
        }
        let stem = t.path.trim_end_matches(".wav").to_string();
        let mut n = 1;
        loop {
            let candidate = if n == 1 {
                format!("{stem} ({}).wav", t.format.replace('/', "-"))
            } else {
                format!("{stem} ({} {n}).wav", t.format.replace('/', "-"))
            };
            if used.insert(candidate.to_lowercase()) {
                t.name = candidate.rsplit('/').next().unwrap_or(&candidate).trim_end_matches(".wav").to_string();
                t.path = candidate;
                break;
            }
            n += 1;
        }
    }
}

/// A name that is safe as a Windows file name.
pub fn sanitize(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    out = out.trim().trim_end_matches(['.', ' ']).to_string();
    if out.is_empty() {
        out = "untitled".into();
    }
    let upper = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || ((upper.starts_with("COM") || upper.starts_with("LPT")) && upper.len() == 4 && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        out.insert(0, '_');
    }
    out.chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(sanitize("a/b:c?"), "a_b_c_");
        assert_eq!(sanitize("CON"), "_CON");
        assert_eq!(sanitize(" x. "), "x");
        assert_eq!(sanitize(""), "untitled");
    }

    #[test]
    fn dual_names() {
        assert_eq!(dual_candidates("SONG.V0"), vec![("SONG.V1".to_string(), true)]);
        assert_eq!(dual_candidates("BGM_L.VAG"), vec![("BGM_R.VAG".to_string(), true)]);
        assert_eq!(dual_candidates("BGM_R.VAG"), vec![("BGM_L.VAG".to_string(), false)]);
        assert_eq!(dual_candidates("MUSIC.L"), vec![("MUSIC.R".to_string(), true)]);
        assert!(dual_candidates("TITLE.VAG").is_empty());
    }

    #[test]
    fn pes_headers() {
        // MPEG-2 PES header with 5 bytes of header data
        assert_eq!(pes_header_len(&[0x81, 0x80, 0x05, 0, 0, 0, 0, 0, 0xaa]), 8);
        // MPEG-1 with a PTS
        assert_eq!(pes_header_len(&[0x21, 0, 0, 0, 0, 0xaa]), 5);
    }

    #[test]
    fn zlib_headers() {
        assert!(zlib_header_ok(&[0x78, 0x9c]));
        assert!(!zlib_header_ok(&[0x78, 0x9d]));
    }
}
