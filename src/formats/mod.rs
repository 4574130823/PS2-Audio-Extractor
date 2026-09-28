//! Audio formats, one module each, named after the vgmstream parser they follow
//! (github.com/vgmstream/vgmstream, src/meta).
//!
//! A format is a `Parser`: signatures (bytes that mark its header) and/or file
//! extensions (for formats without a usable signature), and a function that reads a
//! header at an offset and returns the track(s) it describes. The scanner finds every
//! signature in every file and asks the matching parsers; each validates strictly (sane
//! sizes, rates, channel counts, plausible data) since hits can be anywhere.

use std::collections::HashMap;
use std::io;

use crate::disc::{Entry, Game, Reader};
use crate::track::Track;

pub mod a2m;
pub mod acx;
pub mod adp_ongakukan;
pub mod adx;
pub mod ahv;
pub mod aix;
pub mod ast_mmv;
pub mod ast_mv;
pub mod audiopkg;
pub mod aus;
pub mod bg00;
pub mod bnk_sony;
pub mod ea_schl;
pub mod ea_schl_abk;
pub mod ea_schl_hdr_dat;
pub mod ea_schl_map_mpf_mus;
pub mod ea_schl_standard;
pub mod ea_swvr;
pub mod exst;
pub mod filp;
pub mod fsb;
pub mod gbts;
pub mod hd_bd;
pub mod hgc1;
pub mod hsf;
pub mod hxd;
pub mod iab;
pub mod iivb;
pub mod ikm;
pub mod ild;
pub mod imc;
pub mod ivb;
pub mod joe;
pub mod jstm;
pub mod lp_ap_lep;
pub mod lpcm_shade;
pub mod mcg;
pub mod mcss;
pub mod mib_mih;
pub mod mic_koei;
pub mod mjh_mjb;
pub mod mpc3;
pub mod msa;
pub mod msh_msb;
pub mod msv;
pub mod mtaf;
pub mod mul;
pub mod musc;
pub mod musx;
pub mod npsf;
pub mod ogg_vorbis;
pub mod omu;
pub mod p2bt_move_visa;
pub mod pcm_kceje;
pub mod pcm_success;
pub mod pfs2;
pub mod ps2_adm;
pub mod ps2p;
pub mod psf;
pub mod pwb;
pub mod raw_int;
pub mod riff;
pub mod rkv;
pub mod rsd;
pub mod rstm_rockstar;
pub mod rws_80d;
pub mod rxws;
pub mod sdf;
pub mod seb;
pub mod sfx0_monster;
pub mod skex;
pub mod sl3;
pub mod smp;
pub mod smpl;
pub mod smss;
pub mod spm;
pub mod sre_pcm;
pub mod sshd;
pub mod ssnd;
pub mod ster;
pub mod stma;
pub mod str_wav;
pub mod svag_kcet;
pub mod svag_snk;
pub mod svgp;
pub mod svs;
pub mod tac;
pub mod ubi_hx;
pub mod ubi_sb;
pub mod vag;
pub mod vas_kceo;
pub mod vbk;
pub mod vds_vdm;
pub mod vgs;
pub mod vgs_ps;
pub mod vgv;
pub mod vig_kces;
pub mod vms;
pub mod voi;
pub mod vpk;
pub mod vs;
pub mod vs_square;
pub mod vs_str;
pub mod vsf;
pub mod vsv;
pub mod wd;
pub mod wmw;
pub mod xa2_acclaim;
pub mod xabp;
pub mod xau;
pub mod xavs;

pub const PARSERS: &[&Parser] = &[
    &hd_bd::PARSER, &sshd::PARSER, &vag::PARSER, &adx::PARSER, &riff::PARSER,
    &a2m::PARSER, &acx::PARSER, &adp_ongakukan::PARSER, &ahv::PARSER, &aix::PARSER, &ast_mmv::PARSER,
    &ast_mv::PARSER, &audiopkg::PARSER, &aus::PARSER, &bg00::PARSER, &bnk_sony::PARSER, &ea_schl::PARSER,
    &ea_schl_abk::PARSER, &ea_schl_hdr_dat::PARSER, &ea_schl_map_mpf_mus::PARSER, &ea_schl_standard::PARSER,
    &ea_swvr::PARSER, &exst::PARSER, &filp::PARSER, &fsb::PARSER, &gbts::PARSER, &hgc1::PARSER, &hsf::PARSER,
    &hxd::PARSER, &iab::PARSER, &iivb::PARSER, &ikm::PARSER, &ild::PARSER, &imc::PARSER, &ivb::PARSER, &joe::PARSER,
    &jstm::PARSER, &lp_ap_lep::PARSER, &lpcm_shade::PARSER, &mcg::PARSER, &mcss::PARSER, &mib_mih::PARSER,
    &mic_koei::PARSER, &mjh_mjb::PARSER, &mpc3::PARSER, &msa::PARSER, &msh_msb::PARSER, &msv::PARSER, &mtaf::PARSER,
    &mul::PARSER, &musc::PARSER, &musx::PARSER, &npsf::PARSER, &ogg_vorbis::PARSER, &omu::PARSER,
    &p2bt_move_visa::PARSER, &pcm_kceje::PARSER, &pcm_success::PARSER, &pfs2::PARSER, &ps2_adm::PARSER,
    &ps2p::PARSER, &psf::PARSER, &pwb::PARSER, &raw_int::PARSER, &rkv::PARSER, &rsd::PARSER, &rstm_rockstar::PARSER,
    &rws_80d::PARSER, &rxws::PARSER, &sdf::PARSER, &seb::PARSER, &sfx0_monster::PARSER, &skex::PARSER, &sl3::PARSER,
    &smp::PARSER, &smpl::PARSER, &smss::PARSER, &spm::PARSER, &sre_pcm::PARSER, &ssnd::PARSER, &ster::PARSER,
    &stma::PARSER, &str_wav::PARSER, &svag_kcet::PARSER, &svag_snk::PARSER, &svgp::PARSER, &svs::PARSER,
    &tac::PARSER, &ubi_hx::PARSER, &ubi_sb::PARSER, &vas_kceo::PARSER, &vbk::PARSER, &vds_vdm::PARSER, &vgs::PARSER,
    &vgs_ps::PARSER, &vgv::PARSER, &vig_kces::PARSER, &vms::PARSER, &voi::PARSER, &vpk::PARSER, &vs::PARSER,
    &vs_square::PARSER, &vs_str::PARSER, &vsf::PARSER, &vsv::PARSER, &wd::PARSER, &wmw::PARSER, &xa2_acclaim::PARSER,
    &xabp::PARSER, &xau::PARSER, &xavs::PARSER,
];

pub type ParseFn = fn(&mut Ctx, u64) -> io::Result<Vec<Found>>;
pub type LocateFn = fn(&mut Ctx, u64) -> io::Result<Option<u64>>;

pub struct Parser {
    /// For reading the code and debugging: the format's short name.
    #[allow(dead_code)]
    pub name: &'static str,
    /// Signatures. A hit at file offset `p` means a header at `p - magic_at`.
    pub magics: &'static [&'static [u8]],
    pub magic_at: u64,
    /// Lowercase extensions: files named so are also tried at offset 0 (for formats
    /// whose header has no usable signature).
    pub exts: &'static [&'static str],
    /// Where the header is for a signature hit, when that isn't `hit - magic_at`.
    pub locate: Option<LocateFn>,
    pub parse: ParseFn,
}

fn parse_nothing(_: &mut Ctx, _: u64) -> io::Result<Vec<Found>> {
    Ok(vec![])
}

/// A parser that finds nothing (for modules that only hold code other formats share).
pub const NONE: Parser = Parser { name: "", magics: &[], magic_at: 0, exts: &[], locate: None, parse: parse_nothing };

/// A track as parsed, before the scanner names it.
pub struct Found {
    pub track: Track,
    /// End of what the header and its data cover in this file: nothing else is looked
    /// for inside it.
    pub end: u64,
    /// Name stored in the header, if any.
    pub label: Option<String>,
}

impl Found {
    pub fn new(track: Track, end: u64) -> Found {
        Found { track, end, label: None }
    }
    pub fn label(mut self, label: Option<String>) -> Found {
        self.label = label;
        self
    }
}

/// What a parser works with: the file being scanned, and the rest of the game (for
/// formats that keep their header and data in two files).
pub struct Ctx<'a> {
    pub game: &'a Game,
    /// All files, including data unpacked while scanning (demuxed video audio, inflated data).
    pub entries: &'a [Entry],
    /// Index of the file being scanned.
    pub entry: usize,
    pub r: Reader,
    /// Names of the sounds in this file by offset, from an index kept elsewhere (loaded
    /// once per file by the parser that needs it; `None` until then).
    pub names: Option<HashMap<u64, String>>,
}

impl<'a> Ctx<'a> {
    pub fn size(&self) -> u64 {
        self.r.size
    }

    /// The file's path in the game ("SOUND/SE.HD").
    pub fn path(&self) -> &str {
        &self.entries[self.entry].path
    }

    /// The file's extension, lowercase, without the dot ("" if none).
    pub fn ext(&self) -> String {
        split_ext(self.path()).1.to_ascii_lowercase()
    }

    pub fn bytes(&mut self, off: u64, len: usize) -> io::Result<Vec<u8>> {
        self.r.bytes(off, len)
    }

    /// Whether `magic` is at `off`.
    pub fn is(&mut self, off: u64, magic: &[u8]) -> io::Result<bool> {
        Ok(off + magic.len() as u64 <= self.size() && self.r.bytes(off, magic.len())? == magic)
    }

    pub fn u8(&mut self, off: u64) -> io::Result<u8> {
        Ok(self.r.bytes(off, 1)?[0])
    }
    pub fn u32le(&mut self, off: u64) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.r.bytes(off, 4)?.try_into().unwrap()))
    }
    pub fn u32be(&mut self, off: u64) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.r.bytes(off, 4)?.try_into().unwrap()))
    }
    pub fn i32le(&mut self, off: u64) -> io::Result<i32> {
        Ok(self.u32le(off)? as i32)
    }

    /// Another file next to this one with the same name and extension `ext` (any case):
    /// its index and a reader for it.
    pub fn sibling(&self, ext: &str) -> Option<(usize, Reader)> {
        let (stem, _) = split_ext(self.path());
        let i = self.entries.iter().position(|e| {
            let (s, x) = split_ext(&e.path);
            s.eq_ignore_ascii_case(stem) && x.eq_ignore_ascii_case(ext)
        })?;
        Some((i, self.game.reader(&self.entries[i]).ok()?))
    }

    /// A file in the same folder with this exact name (any case).
    pub fn sibling_named(&self, name: &str) -> Option<(usize, Reader)> {
        let dir = self.path().rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let want = if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") };
        let i = self.entries.iter().position(|e| e.path.eq_ignore_ascii_case(&want))?;
        Some((i, self.game.reader(&self.entries[i]).ok()?))
    }
}

/// ("SOUND/SE", "HD") for "SOUND/SE.HD"; the extension is empty when there isn't one.
pub fn split_ext(path: &str) -> (&str, &str) {
    let name_start = path.rfind('/').map(|s| s + 1).unwrap_or(0);
    match path[name_start..].rfind('.') {
        Some(p) if p > 0 => (&path[..name_start + p], &path[name_start + p + 1..]),
        _ => (path, ""),
    }
}

/// Sample rates PS2 audio can have.
pub fn sane_rate(r: u32) -> bool {
    (4000..=96000).contains(&r)
}

/// A header-stored name: printable ASCII up to the first NUL.
pub fn label(b: &[u8]) -> Option<String> {
    let s: String = b.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect();
    let s = s.trim().to_string();
    (!s.is_empty() && s.chars().all(|c| c.is_ascii_graphic() || c == ' ')).then_some(s)
}

pub fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}
pub fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}
pub fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
pub fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions() {
        assert_eq!(split_ext("SOUND/SE.HD"), ("SOUND/SE", "HD"));
        assert_eq!(split_ext("SOUND.DIR/FILE"), ("SOUND.DIR/FILE", ""));
        assert_eq!(split_ext(".hidden"), (".hidden", ""));
    }
}
