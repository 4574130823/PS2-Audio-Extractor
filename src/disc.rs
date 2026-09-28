//! Where the game's files come from: a disc image (.iso, or a raw .bin/.img with
//! 2352-byte sectors), an extracted disc folder, or a single file.
//!
//! PS2 discs carry an ISO 9660 file system (DVDs add UDF next to it, but ISO 9660 is
//! always there), so images are read by walking its directories. Files are read in place
//! from the image; nothing is unpacked to disk.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SECTOR: u64 = 2048;

/// One file of the game.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Path inside the game, "/"-separated, e.g. "SOUND/BGM/TITLE.VAG".
    pub path: String,
    pub size: u64,
    loc: Loc,
}

#[derive(Debug, Clone)]
enum Loc {
    File(PathBuf),
    /// Byte offset of the file's first byte in the image's logical (2048-byte) sectors.
    Image(u64),
    /// Data made while scanning: audio demuxed from a video, or unpacked from compressed data.
    Memory(Arc<Vec<u8>>),
}

impl Entry {
    /// An in-memory "file", e.g. "MOVIE/OPEN.PSS#audio".
    pub fn memory(path: String, data: Vec<u8>) -> Entry {
        Entry { path, size: data.len() as u64, loc: Loc::Memory(Arc::new(data)) }
    }
}

/// How logical 2048-byte sectors sit in an image file.
#[derive(Debug, Clone, Copy)]
struct Layout {
    /// Bytes per physical sector: 2048 (.iso) or 2352 (raw .bin).
    raw: u64,
    /// Where the 2048 user bytes start in a physical sector: 0, 16 (mode 1) or 24 (mode 2).
    data: u64,
}

#[derive(Debug, Clone)]
pub struct Game {
    /// The game's name: the disc's volume name, or the folder/file name.
    pub name: String,
    /// Name for the output folder: the name, plus the serial when the name doesn't
    /// already say it, e.g. "MYGAME (SLUS-20312)".
    pub title: String,
    pub serial: Option<String>,
    pub entries: Vec<Entry>,
    image: Option<(PathBuf, Layout)>,
}

/// Reads byte ranges of one entry.
pub struct Reader {
    file: Option<File>,
    loc: ReaderLoc,
    pub size: u64,
}

enum ReaderLoc {
    File,
    Image { start: u64, layout: Layout },
    Memory(Arc<Vec<u8>>),
}

impl Reader {
    /// Fills `buf` from `offset` in the entry. Past the end, the rest is left zeroed and
    /// the number of real bytes is returned.
    pub fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        buf.fill(0);
        if offset >= self.size {
            return Ok(0);
        }
        let want = (buf.len() as u64).min(self.size - offset) as usize;
        let file = self.file.as_mut();
        match &self.loc {
            ReaderLoc::Memory(m) => {
                buf[..want].copy_from_slice(&m[offset as usize..offset as usize + want]);
            }
            ReaderLoc::File => {
                let file = file.ok_or_else(|| io::Error::other("no file"))?;
                file.seek(SeekFrom::Start(offset))?;
                read_fully(file, &mut buf[..want])?;
            }
            &ReaderLoc::Image { start, layout } if layout.raw == SECTOR => {
                let file = file.ok_or_else(|| io::Error::other("no file"))?;
                file.seek(SeekFrom::Start(start + offset))?;
                read_fully(file, &mut buf[..want])?;
            }
            &ReaderLoc::Image { start, layout } => {
                // Raw sectors: copy the user data of each sector in turn.
                let file = file.ok_or_else(|| io::Error::other("no file"))?;
                let mut done = 0;
                while done < want {
                    let logical = start + offset + done as u64;
                    let (sector, within) = (logical / SECTOR, logical % SECTOR);
                    let n = ((SECTOR - within) as usize).min(want - done);
                    file.seek(SeekFrom::Start(sector * layout.raw + layout.data + within))?;
                    read_fully(file, &mut buf[done..done + n])?;
                    done += n;
                }
            }
        }
        Ok(want)
    }

    /// Reads `len` bytes at `offset` into a new buffer (zero-padded past the end).
    pub fn bytes(&mut self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let mut v = vec![0; len];
        self.read_at(offset, &mut v)?;
        Ok(v)
    }
}

fn read_fully(f: &mut File, buf: &mut [u8]) -> io::Result<()> {
    let mut done = 0;
    while done < buf.len() {
        match f.read(&mut buf[done..])? {
            0 => break, // short file; the caller's buffer stays zeroed
            n => done += n,
        }
    }
    Ok(())
}

impl Game {
    /// Opens a disc image, an extracted disc folder, or any single file.
    pub fn open(path: &Path) -> Result<Game, String> {
        if path.is_dir() {
            return Game::from_folder(path);
        }
        let mut file = File::open(path).map_err(|e| format!("Can't open {}: {e}", path.display()))?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        if let Some(layout) = detect_image(&mut file, len) {
            return Game::from_image(path, layout);
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(Game {
            name: stem(path),
            title: stem(path),
            serial: None,
            entries: vec![Entry { path: name, size: len, loc: Loc::File(path.to_path_buf()) }],
            image: None,
        })
    }

    pub fn reader(&self, entry: &Entry) -> io::Result<Reader> {
        Ok(match (&entry.loc, &self.image) {
            (Loc::File(p), _) => Reader { file: Some(File::open(p)?), loc: ReaderLoc::File, size: entry.size },
            (Loc::Memory(m), _) => Reader { file: None, loc: ReaderLoc::Memory(m.clone()), size: entry.size },
            (Loc::Image(start), Some((img, layout))) => Reader {
                file: Some(File::open(img)?),
                loc: ReaderLoc::Image { start: *start, layout: *layout },
                size: entry.size,
            },
            (Loc::Image(_), None) => return Err(io::Error::other("image entry without an image")),
        })
    }

    /// The same game with a different file list (the scan adds in-memory files).
    pub fn with_entries(&self, entries: Vec<Entry>) -> Game {
        Game { entries, ..self.clone() }
    }

    fn from_folder(dir: &Path) -> Result<Game, String> {
        let mut entries = Vec::new();
        walk_folder(dir, dir, &mut entries).map_err(|e| format!("Can't read {}: {e}", dir.display()))?;
        entries.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
        let mut game = Game { name: stem(dir), title: stem(dir), serial: None, entries, image: None };
        game.find_serial();
        Ok(game)
    }

    fn from_image(path: &Path, layout: Layout) -> Result<Game, String> {
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let mut read_sector = |lba: u64| -> io::Result<Vec<u8>> {
            let mut buf = vec![0; SECTOR as usize];
            file.seek(SeekFrom::Start(lba * layout.raw + layout.data))?;
            read_fully(&mut file, &mut buf)?;
            Ok(buf)
        };

        // Primary volume descriptor.
        let pvd = read_sector(16).map_err(|e| e.to_string())?;
        let volume_id = String::from_utf8_lossy(&pvd[40..72]).trim().to_string();
        let root = &pvd[156..156 + 34];
        let root_lba = u32le(root, 2) as u64;
        let root_len = u32le(root, 10) as u64;

        let mut entries = Vec::new();
        let mut stack = vec![(String::new(), root_lba, root_len)];
        let mut seen = std::collections::HashSet::new();
        while let Some((prefix, lba, len)) = stack.pop() {
            if !seen.insert(lba) || len > 64 << 20 {
                continue; // loops or nonsense in a damaged image
            }
            for s in 0..len.div_ceil(SECTOR) {
                let sector = read_sector(lba + s).map_err(|e| e.to_string())?;
                let mut i = 0;
                while i < sector.len() {
                    let rec_len = sector[i] as usize;
                    if rec_len == 0 || i + rec_len > sector.len() || rec_len < 34 {
                        break; // rest of this sector is padding
                    }
                    let rec = &sector[i..i + rec_len];
                    i += rec_len;
                    let name_len = rec[32] as usize;
                    let raw_name = &rec[33..(33 + name_len).min(rec.len())];
                    if raw_name == [0] || raw_name == [1] {
                        continue; // "." and ".."
                    }
                    let mut name = String::from_utf8_lossy(raw_name).into_owned();
                    if let Some(p) = name.find(';') {
                        name.truncate(p); // version, "FILE.VAG;1"
                    }
                    let name = name.trim_end_matches('.').to_string();
                    let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
                    let extent = u32le(rec, 2) as u64;
                    let size = u32le(rec, 10) as u64;
                    if rec[25] & 0x02 != 0 {
                        stack.push((path, extent, size));
                    } else {
                        entries.push(Entry { path, size, loc: Loc::Image(extent * SECTOR) });
                    }
                }
            }
        }
        entries.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));

        let name = if volume_id.is_empty() { stem(path) } else { volume_id };
        let mut game = Game {
            title: name.clone(),
            name,
            serial: None,
            entries,
            image: Some((path.to_path_buf(), layout)),
        };
        game.find_serial();
        Ok(game)
    }

    /// The game's serial from SYSTEM.CNF ("BOOT2 = cdrom0:\SLUS_203.12;1" -> "SLUS-20312"),
    /// added to the title when the title doesn't already say it.
    fn find_serial(&mut self) {
        let Some(entry) = self.entries.iter().find(|e| e.path.eq_ignore_ascii_case("SYSTEM.CNF")).cloned() else {
            return;
        };
        let Ok(mut r) = self.reader(&entry) else { return };
        let Ok(text) = r.bytes(0, entry.size.min(4096) as usize) else { return };
        let text = String::from_utf8_lossy(&text);
        let serial = text.lines().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            if !k.trim().eq_ignore_ascii_case("BOOT2") && !k.trim().eq_ignore_ascii_case("BOOT") {
                return None;
            }
            let file = v.trim().rsplit(['\\', ':', '/']).next()?.split(';').next()?.to_string();
            parse_serial(&file)
        });
        if let Some(s) = &serial {
            let plain = |t: &str| t.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
            if !plain(&self.title).contains(&plain(s)) {
                self.title = format!("{} ({s})", self.title);
            }
        }
        self.serial = serial;
    }
}

/// "SLUS_203.12" -> "SLUS-20312"
fn parse_serial(file: &str) -> Option<String> {
    let (prefix, rest) = file.split_once('_')?;
    let digits: String = rest.chars().filter(char::is_ascii_digit).collect();
    (prefix.len() == 4 && prefix.chars().all(|c| c.is_ascii_alphabetic()) && digits.len() == 5)
        .then(|| format!("{}-{digits}", prefix.to_ascii_uppercase()))
}

fn walk_folder(root: &Path, dir: &Path, out: &mut Vec<Entry>) -> io::Result<()> {
    for e in fs::read_dir(dir)? {
        let e = e?;
        let path = e.path();
        let meta = e.metadata()?;
        if meta.is_dir() {
            walk_folder(root, &path, out)?;
        } else if meta.is_file() {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            out.push(Entry { path: rel, size: meta.len(), loc: Loc::File(path) });
        }
    }
    Ok(())
}

/// Whether the file is an ISO 9660 image, and how its sectors are laid out.
fn detect_image(file: &mut File, len: u64) -> Option<Layout> {
    let candidates = [
        Layout { raw: 2048, data: 0 },
        Layout { raw: 2352, data: 24 }, // CD mode 2 form 1 (PS2 CDs)
        Layout { raw: 2352, data: 16 }, // CD mode 1
    ];
    candidates.into_iter().find(|l| {
        if len % l.raw != 0 && l.raw != 2048 {
            return false;
        }
        let mut id = [0u8; 6];
        file.seek(SeekFrom::Start(16 * l.raw + l.data)).is_ok()
            && file.read_exact(&mut id).is_ok()
            && id == *b"\x01CD001"
    })
}

fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "game".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serials() {
        assert_eq!(parse_serial("SLUS_203.12").as_deref(), Some("SLUS-20312"));
        assert_eq!(parse_serial("SCES_500.51").as_deref(), Some("SCES-50051"));
        assert_eq!(parse_serial("MAIN.ELF"), None);
    }
}
