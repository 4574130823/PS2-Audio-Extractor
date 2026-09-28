//! Ubisoft DARE banks and maps, PS2 versions (vgmstream meta/ubi_sb.c): .SB1 banks, .SM1/.LM1
//! maps (sets of banks) and the earlier PS2 .BNM ("2xsp") banks [Rayman 2 Revolution,
//! Batman: Vengeance, Splinter Cell 1-4, Prince of Persia trilogy, Rainbow Six 3, Ghost
//! Recon 2/GRAW, Myst III, TMNT, Open Season...]. Sounds are PS-ADPCM (or PCM) inside the
//! bank or in external stream files; sequences (chains of sounds) are joined like vgmstream
//! does when all their parts are in one file.
//!
//! Not handled: other platforms' banks (.sb0/.sb2...), .BLK maps, sequences spanning several
//! files, and a few layer variants (listed with a note).

use std::io;
use std::sync::Arc;

use super::{Ctx, Found, Parser, sane_rate};
use crate::codecs::{Codec, ea_xa};
use crate::track::{Data, Track};

pub const PARSER: Parser = Parser { name: "Ubi SB", magics: &[], magic_at: 0, exts: &["sb1", "sm1", "lm1", "bnm"], locate: None, parse };

const MAX_SUBSONGS: u64 = 128000;
const MAX_CHAIN: u64 = 256;

#[derive(Clone, Default)]
struct Cfg {
    map_version: u32,
    map_entry_size: u64,
    map_name: u64,
    s1_entry: u64,
    s2_entry: u64,
    s3_entry: u64,
    resource_name_size: u64,
    audio_extra_offset: u64,
    audio_stream_size: u64,
    audio_stream_offset: u64,
    audio_stream_type: u64,
    audio_software_flag: u64,
    audio_hwmodule_flag: u64,
    audio_streamed_flag: u64,
    audio_cd_streamed_flag: u64,
    audio_loop_flag: u64,
    audio_loc_flag: u64,
    audio_stereo_flag: u64,
    audio_ram_streamed_flag: u64,
    audio_internal_flag: u64,
    audio_num_samples: u64,
    audio_num_samples2: u64,
    audio_sample_rate: u64,
    audio_channels: u64,
    audio_stream_name: u64,
    audio_extra_name: u64,
    audio_pitch: u64,
    streamed_and: u32,
    cd_streamed_and: u32,
    loop_and: u32,
    software_and: u32,
    hwmodule_and: u32,
    loc_and: u32,
    stereo_and: u32,
    ram_streamed_and: u32,
    audio_interleave: u64,
    has_rs_files: bool,
    seq_extra_offset: u64,
    seq_loop_start: u64,
    seq_num_loops: u64,
    seq_count: u64,
    seq_entry_size: u64,
    layer_count: u64,
    layer_stream_size: u64,
    layer_stream_offset: u64,
    layer_pitch: u64,
    layer_loc_flag: u64,
    layer_loc_and: u32,
    layer_extra_offset: u64,
    layer_stream_name: u64,
    layer_extra_name: u64,
    layer_entry_size: u64,
    layer_sample_rate: u64,
    layer_channels: u64,
    layer_stream_type: u64,
    layer_num_samples: u64,
    layer_hijack: u32,
    silence_int: u64,
    silence_float: u64,
    padded_s1: bool,
    padded_s2: bool,
    padded_s3: bool,
    padded_sx: bool,
    padded_sounds: bool,
}

#[derive(Clone, Default)]
struct Sb {
    be: bool,
    version: u32,
    is_map: bool,
    is_ps2_bnm: bool,
    is_ps2_old: bool,
    bank_number: u32,
    s1_off: u64,
    s1_num: u64,
    s2_off: u64,
    s2_num: u64,
    s3_off: u64,
    s3_num: u64,
    sx_off: u64,
    sx_size: u64,
    map_name: String,
    cfg: Cfg,
}

#[derive(Clone, Copy, PartialEq, Default, Debug)]
enum Kind {
    #[default]
    None,
    Audio,
    Layer,
    Sequence,
    Silence,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum UCodec {
    Psx,
    Pcm,
    Other(&'static str),
}

#[derive(Clone, Default)]
struct Hdr {
    kind: Kind,
    header_index: u64,
    stream_offset: u64,
    stream_size: u64,
    stream_type: u32,
    subblock_id: u32,
    loop_flag: bool,
    loop_start: i64,
    num_samples: i64,
    sample_rate: u32,
    channels: u32,
    is_streamed: bool,
    is_cd_streamed: bool,
    is_ram_streamed: bool,
    is_external: bool,
    is_localized: bool,
    resource_name: String,
    codec: Option<UCodec>,
    layer_count: u64,
    layer_channels: Vec<u32>,
    seq: Vec<u64>,
    seq_loop_start: i64,
    seq_num_loops: u32,
    duration: f32,
}

/// The header file in memory, read with the bank's endianness (0xFFFFFFFF past the end,
/// like vgmstream's reads).
struct B<'a> {
    d: &'a [u8],
    be: bool,
}

impl B<'_> {
    fn u32(&self, o: u64) -> u32 {
        let o = o as usize;
        match self.d.get(o..o.wrapping_add(4)) {
            Some(b) => {
                let a: [u8; 4] = b.try_into().unwrap();
                if self.be { u32::from_be_bytes(a) } else { u32::from_le_bytes(a) }
            }
            None => 0xFFFFFFFF,
        }
    }
    fn u16(&self, o: u64) -> u16 {
        let o = o as usize;
        match self.d.get(o..o.wrapping_add(2)) {
            Some(b) => {
                if self.be { u16::from_be_bytes([b[0], b[1]]) } else { u16::from_le_bytes([b[0], b[1]]) }
            }
            None => 0xFFFF,
        }
    }
    fn u8(&self, o: u64) -> u8 {
        self.d.get(o as usize).copied().unwrap_or(0xFF)
    }
    fn f32(&self, o: u64) -> f32 {
        f32::from_bits(self.u32(o))
    }
    /// read_string_sz with a buffer of 0x28 (resource names)
    fn name(&self, o: u64, size: u64) -> String {
        let mut s = String::new();
        for pos in 0..0x28u64 {
            if size != 0 && pos == size {
                break;
            }
            let c = self.u8(o + pos);
            if c == 0 || pos == 0x27 || !(0x20..=0xF0).contains(&c) {
                break;
            }
            s.push(c as char);
        }
        s
    }
}

fn align16(v: u64) -> u64 {
    v.next_multiple_of(0x10)
}

// ------------------------------------------------------------------ config

fn config_version(sb: &mut Sb, ctx: &Ctx) -> bool {
    let c = &mut sb.cfg;
    c.resource_name_size = 0x28;
    c.map_version = if sb.version <= 7 {
        1
    } else if sb.version < 0x00150000 {
        2
    } else {
        3
    };
    c.map_entry_size = if c.map_version < 2 { 0x30 } else { 0x34 };
    c.map_name = 0x10;
    if sb.is_ps2_bnm {
        c.audio_stream_size = 0x2c;
        c.audio_stream_offset = 0x30;
        c.seq_extra_offset = 0x10;
    } else if sb.version <= 7 {
        c.audio_internal_flag = 0x08;
        c.audio_stream_size = 0x0c;
        c.audio_extra_offset = 0x10;
        c.audio_stream_offset = 0x14;
        c.seq_extra_offset = 0x10;
        c.layer_extra_offset = 0x10;
    } else {
        c.audio_stream_size = 0x08;
        c.audio_extra_offset = 0x0c;
        c.audio_stream_offset = 0x10;
        c.seq_extra_offset = 0x0c;
        c.layer_extra_offset = 0x0c;
    }
    let entry = |c: &mut Cfg, s1: u64, s2: u64| {
        c.s1_entry = s1;
        c.s2_entry = s2;
        c.s3_entry = 8;
    };
    let fb_ps2 = |c: &mut Cfg, flags: u64, st: u32, sw: u32, lp: u32, hw: u32| {
        c.audio_streamed_flag = flags;
        c.audio_software_flag = flags;
        c.audio_loop_flag = flags;
        c.audio_hwmodule_flag = flags;
        c.streamed_and = st;
        c.software_and = sw;
        c.loop_and = lp;
        c.hwmodule_and = hw;
    };
    let fb = |c: &mut Cfg, flags: u64, st: u32, sw: u32, lp: u32| {
        c.audio_streamed_flag = flags;
        c.audio_software_flag = flags;
        c.audio_loop_flag = flags;
        c.streamed_and = st;
        c.software_and = sw;
        c.loop_and = lp;
    };
    let hs = |c: &mut Cfg, ch: u64, rate: u64, ns: u64, ns2: u64, name: u64, st: u64| {
        c.audio_channels = ch;
        c.audio_sample_rate = rate;
        c.audio_num_samples = ns;
        c.audio_num_samples2 = ns2;
        c.audio_stream_name = name;
        c.audio_stream_type = st;
    };
    let he = |c: &mut Cfg, ch: u64, rate: u64, ns: u64, ns2: u64, name: u64, st: u64| {
        c.audio_channels = ch;
        c.audio_sample_rate = rate;
        c.audio_num_samples = ns;
        c.audio_num_samples2 = ns2;
        c.audio_extra_name = name;
        c.audio_stream_type = st;
    };
    let is_bnm = sb.is_ps2_bnm;
    let seq = |c: &mut Cfg, count: u64, size: u64| {
        c.seq_loop_start = count - 0x10;
        c.seq_num_loops = count - 0x0c;
        c.seq_count = count;
        c.seq_entry_size = size;
        if is_bnm {
            c.seq_loop_start = count - 0x0c;
            c.seq_num_loops = count - 0x08;
        }
    };
    let layer_hs = |c: &mut Cfg, count: u64, size: u64, offset: u64, name: u64| {
        c.layer_count = count;
        c.layer_stream_size = size;
        c.layer_stream_offset = offset;
        c.layer_stream_name = name;
    };
    let layer_he = |c: &mut Cfg, count: u64, size: u64, offset: u64, name: u64| {
        c.layer_count = count;
        c.layer_stream_size = size;
        c.layer_stream_offset = offset;
        c.layer_extra_name = name;
    };
    let layer_sh = |c: &mut Cfg, entry: u64, rate: u64, channels: u64, stream_type: u64, samples: u64| {
        c.layer_entry_size = entry;
        c.layer_sample_rate = rate;
        c.layer_channels = channels;
        c.layer_stream_type = stream_type;
        c.layer_num_samples = samples;
    };
    let v = sb.version;
    if sb.is_ps2_bnm {
        // Rayman 2: Revolution, Disney's Dinosaur, Hype: The Time Quest
        sb.version = 0;
        entry(c, 0x1c, 0x44);
        c.audio_streamed_flag = 0x18;
        c.audio_cd_streamed_flag = 0x18;
        c.audio_loop_flag = 0x18;
        c.streamed_and = 1 << 5;
        c.cd_streamed_and = 1 << 6;
        c.loop_and = 1 << 7;
        c.audio_channels = 0x20;
        c.audio_sample_rate = 0x22;
        c.audio_interleave = 0x400;
        seq(c, 0x24, 0x14);
        return true;
    }
    match v {
        0x00000003 => {
            // Batman: Vengeance, Disney's Tarzan: Untamed (maps)
            entry(c, 0x30, 0x3c);
            c.audio_streamed_flag = 0x1c;
            c.audio_loop_flag = 0x1c;
            c.audio_loc_flag = 0x1c;
            c.audio_stereo_flag = 0x1c;
            c.streamed_and = 1 << 4;
            c.loop_and = 1 << 5;
            c.loc_and = 1 << 6;
            c.stereo_and = 1 << 7;
            c.audio_pitch = 0x20;
            c.audio_sample_rate = 0x24;
            c.audio_interleave = 0x800;
            sb.is_ps2_old = true;
            seq(c, 0x2c, 0x18);
            c.layer_loc_flag = 0x1c;
            c.layer_loc_and = 1;
            c.layer_count = 0x20;
            c.layer_pitch = 0x24;
        }
        0x00000004 => {
            // Myst III: Exile
            entry(c, 0x34, 0x70);
            fb(c, 0x1c, 1 << 4, 0, 1 << 5);
            hs(c, 0x24, 0x28, 0x34, 0x3c, 0x44, 0x6c);
            seq(c, 0x2c, 0x24);
        }
        0x00000007 => {
            // Splinter Cell, Splinter Cell: Pandora Tomorrow (maps)
            entry(c, 0x40, 0x70);
            fb(c, 0x1c, 1 << 2, 0, 1 << 3);
            hs(c, 0x24, 0x28, 0x34, 0x3c, 0x44, 0x6c);
            seq(c, 0x2c, 0x30);
            layer_hs(c, 0x24, 0x64, 0x5c, 0x34);
            layer_sh(c, 0x18, 0x00, 0x06, 0x08, 0x14);
        }
        0x000A0002 | 0x000A0004 | 0x000A0007 | 0x000A0008 | 0x00100000 | 0x00120009 | 0x0012000c => {
            let bia = v == 0x000A0007 && (ctx.sibling_named("BIAAUDIO.SP1").is_some() || parent_has(ctx, "BIAAUDIO.SP1"));
            if bia {
                // Brothers in Arms: Road to Hill 30 / Earned in Blood
                entry(c, 0x5c, 0x14c);
                fb_ps2(c, 0x18, 1 << 2, 1 << 3, 1 << 4, 1 << 5);
                hs(c, 0x20, 0x24, 0x30, 0x38, 0x40, 0x148);
                seq(c, 0x28, 0x10);
                layer_hs(c, 0x20, 0x140, 0x138, 0x30);
                layer_sh(c, 0x14, 0x00, 0x06, 0x08, 0x10);
                c.padded_s1 = true;
                c.padded_s2 = true;
                c.padded_s3 = true;
                c.padded_sx = true;
                c.padded_sounds = true;
            } else {
                // Prince of Persia: The Sands of Time / Warrior Within, Rainbow Six 3, Ghost Recon 2, Horsez...
                entry(c, 0x48, 0x6c);
                fb_ps2(c, 0x18, 1 << 2, 1 << 3, 1 << 4, 1 << 5);
                hs(c, 0x20, 0x24, 0x30, 0x38, 0x40, 0x68);
                seq(c, 0x28, 0x10);
                layer_hs(c, 0x20, 0x60, 0x58, 0x30);
                layer_sh(c, 0x14, 0x00, 0x06, 0x08, 0x10);
                c.silence_int = 0x18;
            }
        }
        0x00130001 => {
            // Splinter Cell: Chaos Theory (map)
            entry(c, 0x48, 0x4c);
            fb_ps2(c, 0x18, 1 << 2, 1 << 3, 1 << 4, 1 << 5);
            he(c, 0x20, 0x24, 0x30, 0x38, 0x40, 0x44);
            seq(c, 0x28, 0x10);
        }
        0x00130004 => {
            // Ghost Recon Advanced Warfighter
            entry(c, 0x48, 0x50);
            fb_ps2(c, 0x18, 1 << 2, 1 << 3, 1 << 4, 1 << 5);
            he(c, 0x20, 0x24, 0x30, 0x38, 0x40, 0x4c);
            c.audio_interleave = 0x8000;
            c.padded_s1 = true;
            c.padded_sounds = true;
        }
        0x00150000 => {
            // Prince of Persia: The Two Thrones
            entry(c, 0x48, 0x5c);
            fb_ps2(c, 0x20, 1 << 2, 1 << 3, 1 << 4, 1 << 5);
            he(c, 0x2c, 0x30, 0x3c, 0x44, 0x4c, 0x50);
            seq(c, 0x2c, 0x10);
        }
        0x00160002 | 0x00180003 => {
            // Splinter Cell: Double Agent, Open Season, Shaun White Snowboarding (maps)
            entry(c, 0x48, 0x54);
            fb_ps2(c, 0x20, 1 << 2, 1 << 3, 1 << 4, 1 << 5);
            he(c, 0x28, 0x2c, 0x34, 0x3c, 0x44, 0x48);
            seq(c, 0x2c, 0x10);
            layer_he(c, 0x20, 0x2c, 0x30, 0x38);
            layer_sh(c, 0x34, 0x00, 0x08, 0x0c, 0x14);
            c.silence_float = 0x1c;
        }
        0x00190002 => {
            // TMNT
            entry(c, 0x48, 0x5c);
            fb_ps2(c, 0x20, 1 << 2, 1 << 3, 1 << 4, 1 << 5);
            he(c, 0x28, 0x2c, 0x34, 0x3c, 0x44, 0x48);
            seq(c, 0x2c, 0x10);
            layer_he(c, 0x20, 0x2c, 0x30, 0x38);
            layer_sh(c, 0x30, 0x00, 0x04, 0x08, 0x10);
            c.silence_float = 0x1c;
        }
        _ => return false,
    }
    true
}

/// check_project_file's "../name" for localized subfolders.
fn parent_has(ctx: &Ctx, name: &str) -> bool {
    let path = ctx.path();
    let Some((dir, _)) = path.rsplit_once('/') else { return false };
    let parent = dir.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    let want = if parent.is_empty() { name.to_string() } else { format!("{parent}/{name}") };
    ctx.entries.iter().any(|e| e.path.eq_ignore_ascii_case(&want))
}

// ------------------------------------------------------------------ headers

fn pitch_to_freq(pitch: u32) -> u32 {
    ((pitch as f64 / 65536.0) * 48000.0).ceil() as u32
}

fn parse_audio(sb: &Sb, h: &mut Hdr, b: &B, off: u64) -> bool {
    let c = &sb.cfg;
    h.kind = Kind::Audio;
    if sb.is_ps2_bnm {
        h.stream_size = b.u32(off + c.audio_stream_size) as u64;
        h.stream_offset = b.u32(off + c.audio_stream_offset) as u64;
        h.channels = b.u8(off + c.audio_channels) as u32;
        h.sample_rate = b.u16(off + c.audio_sample_rate) as u32;
        if h.stream_size == 0 {
            return false;
        }
        let flags = b.u32(off + c.audio_streamed_flag);
        h.is_streamed = flags & c.streamed_and != 0;
        h.is_cd_streamed = b.u32(off + c.audio_cd_streamed_flag) & c.cd_streamed_and != 0;
        h.loop_flag = b.u32(off + c.audio_loop_flag) & c.loop_and != 0;
        h.num_samples = 0;
        if !h.is_cd_streamed {
            h.stream_size *= h.channels as u64;
        }
        h.resource_name = if h.is_streamed {
            if h.is_cd_streamed { format!("BNK_{}.VSC", sb.bank_number as i32) } else { format!("BNK_{}.VSB", sb.bank_number as i32) }
        } else {
            format!("BNK_{}.VB", sb.bank_number as i32)
        };
        h.is_external = true;
        return true;
    }
    if sb.is_ps2_old {
        h.stream_size = b.u32(off + c.audio_stream_size) as u64;
        h.stream_offset = b.u32(off + c.audio_stream_offset) as u64;
        if h.stream_size == 0 {
            return false;
        }
        h.sample_rate = pitch_to_freq(b.u32(off + c.audio_pitch));
        h.is_streamed = b.u32(off + c.audio_streamed_flag) & c.streamed_and != 0;
        h.loop_flag = b.u32(off + c.audio_loop_flag) & c.loop_and != 0;
        h.is_localized = b.u32(off + c.audio_loc_flag) & c.loc_and != 0;
        let stereo = b.u32(off + c.audio_stereo_flag) & c.stereo_and != 0;
        h.num_samples = 0;
        h.channels = if stereo { 2 } else { 1 };
        h.stream_size *= h.channels as u64;
        h.subblock_id = 0;
        if h.is_streamed {
            h.resource_name = if h.is_localized { "STRM.LM1" } else { "STRM.SM1" }.to_string();
            h.is_external = true;
        }
        return true;
    }
    h.stream_size = b.u32(off + c.audio_stream_size) as u64;
    h.stream_offset = b.u32(off + c.audio_stream_offset) as u64;
    h.channels = if c.audio_channels % 4 != 0 { b.u16(off + c.audio_channels) as u32 } else { b.u32(off + c.audio_channels) };
    h.sample_rate = b.u32(off + c.audio_sample_rate);
    h.stream_type = b.u32(off + c.audio_stream_type);
    if h.stream_size == 0 {
        return false;
    }
    h.is_streamed = b.u32(off + c.audio_streamed_flag) & c.streamed_and != 0;
    h.is_external = h.is_streamed;
    if c.audio_internal_flag != 0 && !h.is_streamed {
        h.is_external = b.u32(off + c.audio_internal_flag) == 0;
    }
    if c.audio_software_flag != 0 && c.software_and != 0 {
        let software = b.u32(off + c.audio_software_flag) & c.software_and != 0;
        let hw = b.u32(off + c.audio_hwmodule_flag) & c.hwmodule_and != 0;
        h.subblock_id = if !software { if !hw { 0 } else { 3 } } else { 1 };
        if !software {
            h.stream_type = 0;
        }
    } else {
        h.subblock_id = if h.stream_type == 1 { 0 } else { 1 };
    }
    if c.has_rs_files && !h.is_external {
        h.is_ram_streamed = b.u32(off + c.audio_ram_streamed_flag) & c.ram_streamed_and != 0;
        h.is_external = h.is_ram_streamed;
    }
    h.loop_flag = b.u32(off + c.audio_loop_flag) & c.loop_and != 0;
    if h.loop_flag {
        h.loop_start = b.u32(off + c.audio_num_samples) as i32 as i64;
        h.num_samples = (b.u32(off + c.audio_num_samples2) as i32 as i64) + h.loop_start;
        if c.audio_num_samples == c.audio_num_samples2 {
            h.num_samples = h.loop_start;
            h.loop_start = 0;
        }
    } else {
        h.num_samples = b.u32(off + c.audio_num_samples) as i32 as i64;
    }
    if c.audio_stream_name != 0 {
        if c.has_rs_files && h.is_ram_streamed {
            h.resource_name = "MAPS.RS1".to_string();
        } else if h.is_external {
            h.resource_name = b.name(off + c.audio_stream_name, c.resource_name_size);
        }
    } else {
        let at = b.u32(off + c.audio_extra_name);
        if at != 0xFFFFFFFF {
            h.resource_name = b.name(sb.sx_off + at as u64, c.resource_name_size);
        }
    }
    true
}

fn parse_sequence(sb: &Sb, h: &mut Hdr, b: &B, off: u64) -> bool {
    let c = &sb.cfg;
    h.kind = Kind::Sequence;
    if c.seq_count == 0 {
        return false;
    }
    let extra = b.u32(off + c.seq_extra_offset) as u64 + sb.sx_off;
    h.seq_loop_start = b.u32(off + c.seq_loop_start) as i32 as i64;
    h.seq_num_loops = b.u32(off + c.seq_num_loops);
    let count = b.u32(off + c.seq_count) as u64;
    if count > MAX_CHAIN {
        return false;
    }
    h.seq.clear();
    let mut t = extra;
    for _ in 0..count {
        let mut n = b.u32(t) as u64;
        if sb.is_ps2_bnm {
            let bank = (n >> 16) & 0xFFFF;
            n &= 0xFFFF;
            if bank as u32 != sb.bank_number {
                return false; // (other banks aren't followed here)
            }
        } else {
            n &= 0x3FFFFFFF;
            if n > sb.s2_num {
                return false;
            }
        }
        h.seq.push(n);
        t += c.seq_entry_size;
    }
    true
}

fn parse_layer(sb: &Sb, h: &mut Hdr, b: &B, off: u64) -> bool {
    let c = &sb.cfg;
    h.kind = Kind::Layer;
    if c.layer_count == 0 {
        return false;
    }
    h.is_streamed = true;
    if sb.is_ps2_old {
        h.layer_count = b.u32(off + c.layer_count) as u64;
        h.stream_size = b.u32(off + c.audio_stream_size) as u64;
        h.stream_offset = b.u32(off + c.audio_stream_offset) as u64;
        if h.stream_size == 0 || h.layer_count > 16 {
            return false;
        }
        h.sample_rate = pitch_to_freq(b.u32(off + c.layer_pitch));
        h.is_localized = b.u32(off + c.layer_loc_flag) & c.layer_loc_and != 0;
        h.num_samples = 0;
        h.channels = h.layer_count as u32 * 2;
        h.stream_size *= h.channels as u64;
        h.resource_name = if h.is_localized { "STRM.LM1" } else { "STRM.SM1" }.to_string();
        h.is_external = true;
        return true;
    }
    h.layer_count = b.u32(off + c.layer_count) as u64;
    h.stream_size = b.u32(off + c.layer_stream_size) as u64;
    h.stream_offset = b.u32(off + c.layer_stream_offset) as u64;
    if h.stream_size == 0 || h.layer_count > 16 {
        return false;
    }
    h.is_external = h.is_streamed;
    let mut t = b.u32(off + c.layer_extra_offset) as u64 + sb.sx_off;
    h.sample_rate = b.u32(t + c.layer_sample_rate);
    h.stream_type = b.u32(t + c.layer_stream_type);
    h.num_samples = b.u32(t + c.layer_num_samples) as i32 as i64;
    h.layer_channels.clear();
    for _ in 0..h.layer_count {
        let ch = if c.layer_channels % 4 != 0 { b.u16(t + c.layer_channels) as u32 } else { b.u32(t + c.layer_channels) };
        let rate = b.u32(t + c.layer_sample_rate);
        let st = b.u32(t + c.layer_stream_type);
        let ns = b.u32(t + c.layer_num_samples) as i32 as i64;
        if rate != h.sample_rate || st != h.stream_type {
            return false;
        }
        h.layer_channels.push(ch);
        if h.num_samples != ns && h.num_samples + 1 == ns {
            h.num_samples -= 1;
        }
        t += c.layer_entry_size;
    }
    if c.layer_stream_name != 0 {
        h.resource_name = b.name(off + c.layer_stream_name, c.resource_name_size);
    } else if c.layer_extra_name != 0 {
        let at = b.u32(off + c.layer_extra_name);
        if at != 0xFFFFFFFF {
            h.resource_name = b.name(sb.sx_off + at as u64, c.resource_name_size);
        }
    }
    true
}

fn parse_silence(sb: &Sb, h: &mut Hdr, b: &B, off: u64) -> bool {
    h.kind = Kind::Silence;
    let c = &sb.cfg;
    if c.silence_int != 0 {
        h.duration = b.u32(off + c.silence_int) as f32 / 65536.0;
    } else if c.silence_float != 0 {
        h.duration = b.f32(off + c.silence_float);
    } else {
        return false;
    }
    true
}

fn parse_codec(sb: &Sb, h: &mut Hdr) -> bool {
    if h.kind != Kind::Audio && h.kind != Kind::Layer {
        return true;
    }
    if sb.is_ps2_bnm || sb.is_ps2_old {
        h.codec = Some(UCodec::Psx);
        return true;
    }
    let hw = UCodec::Psx;
    h.codec = Some(if sb.version < 7 {
        match h.stream_type {
            1 => {
                if h.is_streamed {
                    UCodec::Pcm
                } else {
                    hw
                }
            }
            2 => UCodec::Other("Ubi MPEG"),
            4 => UCodec::Other("Ubi APM IMA"),
            6 => UCodec::Other("Ubi ADPCM"),
            8 => UCodec::Other("Ubi IMA"),
            _ => return false,
        }
    } else if sb.version < 0x000A0000 {
        match h.stream_type {
            1 => {
                if h.is_streamed {
                    UCodec::Pcm
                } else {
                    hw
                }
            }
            2 => UCodec::Other("Ubi ADPCM"),
            4 => UCodec::Other("Ubi IMA"),
            _ => return false,
        }
    } else {
        match h.stream_type {
            0 => hw,
            1 => UCodec::Pcm,
            3 => UCodec::Other("Ubi IMA"),
            4 => UCodec::Other("Ogg Vorbis"),
            6 => UCodec::Psx,
            _ => return false,
        }
    });
    true
}

/// parse_offsets: where an internal sound is.
fn parse_offsets(sb: &Sb, h: &mut Hdr, b: &B) -> bool {
    if h.kind != Kind::Audio && h.kind != Kind::Layer {
        return true;
    }
    if sb.is_ps2_bnm {
        if h.is_cd_streamed {
            h.stream_offset *= 0x800;
        }
        return true;
    }
    if sb.is_map {
        if h.is_external && !h.is_ram_streamed {
            return true;
        }
        for i in 0..sb.s3_num {
            let o = sb.s3_off + 0x14 * i;
            let t1 = b.u32(o + 4) as u64 + sb.s3_off;
            let n1 = b.u32(o + 8) as u64;
            let t2 = b.u32(o + 0x0c) as u64 + sb.s3_off;
            let n2 = b.u32(o + 0x10) as u64;
            if n1 > MAX_SUBSONGS || n2 > MAX_SUBSONGS {
                return false;
            }
            for j in 0..n1 {
                let index = (b.u32(t1 + 8 * j) & 0x3FFFFFFF) as u64;
                if index == h.header_index {
                    h.stream_offset = b.u32(t1 + 8 * j + 4) as u64;
                    if h.is_ram_streamed {
                        break;
                    }
                    let mut k = 0;
                    while k < n2 {
                        if b.u32(t2 + 0x10 * k) == h.subblock_id {
                            h.stream_offset += b.u32(t2 + 0x10 * k + 0x0c) as u64;
                            break;
                        }
                        k += 1;
                    }
                    if k == n2 {
                        return false;
                    }
                    break;
                }
            }
            if h.stream_offset != 0 {
                break;
            }
        }
        if h.stream_offset == 0 && !h.is_external {
            return false;
        }
    } else {
        if h.is_external {
            return true;
        }
        let mut sounds = sb.s3_off + sb.cfg.s3_entry * sb.s3_num;
        if sb.cfg.padded_sounds {
            sounds = align16(sounds);
        }
        h.stream_offset += sounds;
        let mut i = 0;
        while i < sb.s3_num {
            let o = sb.s3_off + sb.cfg.s3_entry * i;
            if b.u32(o) == h.subblock_id {
                break;
            }
            h.stream_offset = h.stream_offset.wrapping_add(b.u32(o + 4) as u64);
            i += 1;
        }
        if i == sb.s3_num {
            return false;
        }
    }
    true
}

fn parse_header(sb: &Sb, h: &mut Hdr, b: &B, off: u64, index: u64) -> bool {
    h.header_index = index;
    let t = b.u32(off + 4);
    let ok = match t {
        0x01 => parse_audio(sb, h, b, off),
        0x05 | 0x0b | 0x0c => parse_sequence(sb, h, b, off),
        0x06 | 0x0d => parse_layer(sb, h, b, off),
        0x08 | 0x0f => parse_silence(sb, h, b, off),
        // 0x0a (random) isn't configured for any PS2 version: vgmstream fails it
        _ => false,
    };
    ok && parse_codec(sb, h) && parse_offsets(sb, h, b)
}

// ------------------------------------------------------------------ building tracks

/// One segment of decodable audio.
struct Seg {
    entry: usize,
    channels: u32,
    rate: u32,
    samples: u64,
    kind: ea_xa::Kind,
    /// Ranges of the file making up the segment's data, read back to back.
    pieces: Vec<(u64, u64)>,
    /// (samples, channel starts in the pieces' data), per interleave row
    rows: Vec<(u32, Vec<u64>)>,
    loop_: Option<(u64, u64)>,
    silence: bool,
}

/// ps_find_loop_offsets (no full-loop detection).
fn ps_find_loop(d: &[u8], start: u64, size: u64, channels: u64, interleave: u64) -> Option<(u64, u64)> {
    if size == 0 || channels == 0 || (channels > 1 && interleave == 0) {
        return None;
    }
    let max = start + size;
    let (mut ns, mut ls, mut le) = (0u64, 0u64, 0u64);
    let (mut lsf, mut lef) = (false, false);
    let mut off = start;
    let mut consumed = 0;
    let rd = |o: u64| d.get(o as usize).copied().unwrap_or(0xFF);
    while off < max {
        let flag = rd(off + 1) & 0x0f;
        if flag == 0x06 && !lsf {
            ls = ns;
            lsf = true;
        }
        if flag == 0x03 && le == 0 {
            le = ns + 28;
            lef = true;
            if channels == 1 && off + 0x10 < max && rd(off + 0x11) & 0x0f == 0x06 {
                le = 0;
                lef = false;
            }
            if lsf && lef {
                break;
            }
        }
        ns += 28;
        off += 0x10;
        consumed += 0x10;
        if consumed == interleave {
            consumed = 0;
            off += interleave * (channels - 1);
        }
    }
    (lsf && lef).then_some((ls, le))
}

/// init_vgmstream_ubi_sb_base for PS-ADPCM/PCM: the segment for an audio header.
fn audio_seg(ctx: &mut Ctx, sb: &Sb, h: &Hdr, own: &[u8]) -> io::Result<Result<Seg, String>> {
    let codec = h.codec.unwrap_or(UCodec::Other("?"));
    if let UCodec::Other(name) = codec {
        return Ok(Err(format!("{name} audio isn't supported")));
    }
    let ch = h.channels;
    if !(1..=8).contains(&ch) || !sane_rate(h.sample_rate) {
        return Ok(Err(String::new()));
    }
    // the data's file
    let (entry, mut ext_reader) = if h.is_external {
        let name = h.resource_name.rsplit(['\\', '/']).next().unwrap_or("").to_string();
        match ctx.sibling_named(&name).or_else(|| parent_named(ctx, &name)) {
            Some((i, r)) => (i, Some(r)),
            None => return Ok(Err(format!("external file {name} not found"))),
        }
    } else {
        (ctx.entry, None)
    };
    let file_size = ext_reader.as_ref().map(|r| r.size).unwrap_or(own.len() as u64);
    let mut fetch = |o: u64, n: u64| -> io::Result<Vec<u8>> {
        match ext_reader.as_mut() {
            Some(r) => r.bytes(o, n as usize),
            None => {
                let a = (o as usize).min(own.len());
                let b = (o.saturating_add(n) as usize).min(own.len());
                let mut v = own[a..b].to_vec();
                v.resize(n as usize, 0);
                Ok(v)
            }
        }
    };
    let mut size = h.stream_size;
    let start = h.stream_offset;
    if codec == UCodec::Pcm {
        let mut samples = h.num_samples.max(0) as u64;
        if samples == 0 {
            samples = size / ch as u64 / 2;
        }
        let rows = vec![(samples as u32, (0..ch as u64).map(|c| 2 * c).collect())];
        let loop_ = if h.loop_flag { vgm_loop(h.loop_start, samples as i64, samples) } else { None };
        return Ok(Ok(Seg {
            entry,
            channels: ch,
            rate: h.sample_rate,
            samples,
            kind: ea_xa::Kind::Pcm16Int { big_endian: false },
            pieces: vec![(start, samples * 2 * ch as u64)],
            rows,
            loop_,
            silence: false,
        }));
    }
    if sb.cfg.has_rs_files {
        size = size.wrapping_sub(0x30);
    }
    let interleave = if sb.is_ps2_bnm {
        if h.is_cd_streamed { sb.cfg.audio_interleave } else { size / ch as u64 }
    } else if sb.cfg.audio_interleave != 0 {
        sb.cfg.audio_interleave
    } else {
        size / ch as u64
    };
    let mut samples = h.num_samples;
    let (mut ls, mut le) = (h.loop_start, h.num_samples);
    if samples == 0 {
        samples = (size / ch as u64 / 16 * 28) as i64;
        le = h.num_samples; // (0 until found below)
        if h.loop_start == 0 && h.loop_flag && size < (64 << 20) {
            let data = fetch(start, size)?;
            if let Some((a, b)) = ps_find_loop(&data, 0, size, ch as u64, interleave) {
                ls = a as i64;
                le = b as i64;
            }
        }
    }
    if samples <= 0 || interleave == 0 || start >= file_size {
        return Ok(Err(String::new()));
    }
    let samples = samples as u64;
    // one row per interleave block (the last one with full spacing, like the layout)
    let per_row = if ch == 1 { u64::MAX } else { interleave / 16 * 28 };
    let mut rows = Vec::new();
    let mut left = samples;
    let mut row_base = start;
    let mut hi = start;
    while left > 0 {
        if per_row == 0 {
            return Ok(Err(String::new()));
        }
        let n = left.min(per_row).min(u32::MAX as u64);
        let starts: Vec<u64> = (0..ch as u64).map(|c| row_base - start + c * interleave).collect();
        hi = hi.max(start + starts[ch as usize - 1] + n.div_ceil(28) * 16);
        rows.push((n as u32, starts));
        left -= n;
        row_base += interleave * ch as u64;
    }
    let probe = fetch(start, 0x100.min(file_size - start))?;
    if !crate::codecs::psx::plausible(&probe) {
        return Ok(Err(String::new()));
    }
    let loop_ = if h.loop_flag { vgm_loop(ls, le, samples) } else { None };
    Ok(Ok(Seg {
        entry,
        channels: ch,
        rate: h.sample_rate,
        samples,
        kind: ea_xa::Kind::Psx,
        pieces: vec![(start, hi.min(file_size).max(start + 1) - start)],
        rows,
        loop_,
        silence: false,
    }))
}

/// setup_ubi_sb_streamfile: the file ranges holding layer `n` of the layered stream at
/// `offset` (`size` bytes), read back to back.
fn layer_pieces(r: &mut crate::disc::Reader, be: bool, offset: u64, size: u64, n: u64, count: u64, hijack: u32) -> io::Result<Option<Vec<(u64, u64)>>> {
    let mut rd = |o: u64| -> io::Result<u64> {
        let v: [u8; 4] = r.bytes(o, 4)?.try_into().unwrap();
        Ok((if be { u32::from_be_bytes(v) } else { u32::from_le_bytes(v) }) as u64)
    };
    let mut version = rd(offset)?;
    if hijack == 1 && version == 0x000B0008 {
        version = 0xFFFF0007;
    }
    // (layer max, header next, header sizes, header data base, block next, block sizes, block data base)
    let (max_at, hn, hs, hd, bn, bs, bd): (u64, u64, u64, u64, u64, u64, u64) = match version {
        0x00000002 => (0x04, 0x10, 0, 0x18, 0, 0x08, 0x08),
        0x00000003 => (0x04, 0x10, 0x1c, 0x1c, 0, 0x08, 0x08),
        0x00000004 => (0x04, 0x14, 0x20, 0x20, 0, 0x0c, 0x0c),
        0x00000007 => (0x08, 0x18, 0x40, 0x40, 0, 0x0c, 0x0c),
        0xFFFF0007 => (0x08, 0x18, 0x4c, 0x4c, 0, 0x0c, 0x0c),
        0x00040008 | 0x000B0008 | 0x000C0008 | 0x00100008 => (0x08, 0x18, 0x1c, 0x1c, 0x04, 0x08, 0x08),
        0x00100009 => (0x08, 0x18, 0x5c, 0x5c, 0x04, 0x08, 0x08),
        _ => return Ok(None),
    };
    let layer_max = rd(offset + max_at)?;
    if count > layer_max || layer_max > 64 {
        return Ok(None);
    }
    // data starts after the per-layer size tables (not for version 2's header)
    let hd = if version == 0x00000002 { hd } else { hd + layer_max * 4 };
    let bd = bd + layer_max * 4;
    let mut header_size = hd;
    if hs != 0 {
        for i in 0..layer_max {
            header_size += rd(offset + hs + i * 4)?;
        }
    }
    let end = offset + size;
    let mut pieces = Vec::new();
    let mut phys = offset;
    let mut next = rd(phys + hn)?;
    if hs != 0 {
        let mut skip = hd;
        for i in 0..n {
            skip += rd(phys + hs + i * 4)?;
        }
        let data = rd(phys + hs + n * 4)?;
        if data > 0 {
            pieces.push((phys + skip, data));
        }
    }
    phys += header_size;
    let mut guard = 0;
    while phys < end {
        guard += 1;
        if guard > 1_000_000 {
            return Ok(None);
        }
        let block_size = next;
        if bn != 0 {
            next = rd(phys + bn)?;
        }
        let mut skip = bd;
        for i in 0..n {
            skip += rd(phys + bs + i * 4)?;
        }
        let data = rd(phys + bs + n * 4)?;
        if data > 0 {
            pieces.push((phys + skip, data));
        }
        if block_size == 0 || block_size == 0xFFFFFFFF {
            break;
        }
        phys += block_size;
    }
    Ok(Some(pieces))
}

/// init_vgmstream_ubi_sb_layer: all layers' channels side by side (PS-ADPCM layers).
fn layer_seg(ctx: &mut Ctx, sb: &Sb, h: &Hdr) -> io::Result<Result<Seg, String>> {
    let pcm = match h.codec {
        Some(UCodec::Psx) => false,
        Some(UCodec::Pcm) => true,
        Some(UCodec::Other(n)) => return Ok(Err(format!("Ubi multi-layer {n} streams aren't supported"))),
        None => return Ok(Err(String::new())),
    };
    if pcm && h.layer_channels.iter().any(|&c| c != h.layer_channels[0]) {
        return Ok(Err("Ubi multi-layer PCM with mixed layers isn't supported".into()));
    }
    if !sane_rate(h.sample_rate) || h.num_samples <= 0 || h.layer_count == 0 {
        return Ok(Err(String::new()));
    }
    let name = h.resource_name.rsplit(['/', '\\']).next().unwrap_or("").to_string();
    let Some((entry, mut r)) = ctx.sibling_named(&name).or_else(|| parent_named(ctx, &name)) else {
        return Ok(Err(format!("external file {name} not found")));
    };
    if h.stream_offset + h.stream_size > r.size {
        return Ok(Err(String::new()));
    }
    if sb.cfg.layer_hijack == 2 && r.size >= 0x0080_0000 {
        let g = r.bytes(0x6B00, 8)?;
        if g == [0x60, 0x47, 0xBF, 0x7F, 0x94, 0xFA, 0xCC, 0x01] {
            return Ok(Err("Ubi layered stream with interleaved garbage isn't supported".into()));
        }
    }
    let mut pieces = Vec::new();
    let mut starts = Vec::new();
    let mut base = 0u64;
    let mut channels = 0u32;
    for n in 0..h.layer_count {
        let Some(p) = layer_pieces(&mut r, sb.be, h.stream_offset, h.stream_size, n, h.layer_count, sb.cfg.layer_hijack)? else {
            return Ok(Err(String::new()));
        };
        let ch = h.layer_channels[n as usize];
        if ch == 0 || ch > 8 {
            return Ok(Err(String::new()));
        }
        let mut size: u64 = p.iter().map(|x| x.1).sum();
        if sb.cfg.has_rs_files && !pcm {
            size = size.wrapping_sub(0x30);
        }
        if sb.cfg.audio_interleave != 0 {
            return Ok(Err("Ubi interleaved layers aren't supported".into()));
        }
        let il = if pcm { 2 } else { size / ch as u64 };
        for c in 0..ch as u64 {
            starts.push(base + c * il);
        }
        base += p.iter().map(|x| x.1).sum::<u64>();
        pieces.extend(p);
        channels += ch;
    }
    if pieces.is_empty() || channels > 16 {
        return Ok(Err(String::new()));
    }
    let samples = h.num_samples as u64;
    Ok(Ok(Seg {
        entry,
        channels,
        rate: h.sample_rate,
        samples,
        kind: if pcm { ea_xa::Kind::Pcm16Group { big_endian: false, group: h.layer_channels[0] as u16 } } else { ea_xa::Kind::Psx },
        pieces,
        rows: vec![(samples.min(u32::MAX as u64) as u32, starts)],
        loop_: None,
        silence: false,
    }))
}

fn parent_named(ctx: &Ctx, name: &str) -> Option<(usize, crate::disc::Reader)> {
    ctx.sibling_named(&format!("../{name}"))
}

fn vgm_loop(start: i64, end: i64, samples: u64) -> Option<(u64, u64)> {
    super::ea_schl::vgm_loop(start, end, samples)
}

/// A finished track (or a described one) from segments.
fn build(ctx: &Ctx, segs: Vec<Seg>, loop_: Option<(u64, u64)>) -> Result<Track, String> {
    let first = segs.iter().find(|s| !s.silence).ok_or_else(String::new)?;
    let (entry, ch, rate, kind) = (first.entry, first.channels, first.rate, first.kind);
    if segs.iter().any(|s| !s.silence && (s.entry != entry || s.channels != ch || s.rate != rate || s.kind != kind)) {
        return Err("sequence parts in several files or formats aren't supported".into());
    }
    if segs.iter().any(|s| s.silence && (s.channels != ch || s.rate != rate)) {
        return Err("sequence with mixed formats isn't supported".into());
    }
    let mut pieces = Vec::new();
    let mut base = 0u64;
    let mut blocks = Vec::new();
    let mut total = 0;
    for s in &segs {
        let mut first_row = true;
        for (n, starts) in &s.rows {
            blocks.push(ea_xa::Block {
                samples: *n,
                starts: if s.silence { vec![] } else { starts.iter().map(|x| x + base).collect() },
                reset: first_row,
            });
            first_row = false;
        }
        total += s.samples;
        for &p in &s.pieces {
            pieces.push(p);
            base += p.1;
        }
    }
    let codec = Codec::EaXa(ea_xa::Params { kind, blocks: Arc::new(blocks) });
    let mut t = Track::new(ctx.entry, 0, "Ubi SB", ch as u16, rate, total, Data::blocks(entry, pieces), codec);
    if let Some((a, b)) = loop_ {
        t = t.looped(a, b);
    }
    Ok(t)
}

fn silence_seg(h: &Hdr, prev: (u32, u32)) -> Seg {
    let ch = if prev.0 == 0 { 2 } else { prev.0 };
    let rate = if prev.1 == 0 { 48000 } else { prev.1 };
    let n = (h.duration * rate as f32) as i64;
    let n = n.max(0) as u64;
    Seg { entry: 0, channels: ch, rate, samples: n, kind: ea_xa::Kind::Psx, pieces: vec![], rows: vec![(n as u32, vec![])], loop_: None, silence: true }
}

/// The track for subsong header `h` of bank `sb`.
fn make_track(ctx: &mut Ctx, sb: &Sb, h: &Hdr, own: &[u8]) -> io::Result<Result<Track, String>> {
    let b = B { d: own, be: sb.be };
    Ok(match h.kind {
        Kind::Audio => match audio_seg(ctx, sb, h, own)? {
            Ok(s) => {
                let l = s.loop_;
                build(ctx, vec![s], l)
            }
            Err(e) => Err(e),
        },
        Kind::Layer if sb.is_ps2_old => match audio_seg(ctx, sb, h, own)? {
            Ok(s) => {
                let l = s.loop_;
                build(ctx, vec![s], l)
            }
            Err(e) => Err(e),
        },
        Kind::Layer => match layer_seg(ctx, sb, h)? {
            Ok(s) => build(ctx, vec![s], None),
            Err(e) => Err(e),
        },
        Kind::Sequence => {
            let mut segs = Vec::new();
            let mut prev = (0u32, 0u32);
            let mut total = 0u64;
            let mut loop_start = 0u64;
            let mut err = None;
            for (i, &n) in h.seq.iter().enumerate() {
                let mut t = h.clone();
                t.loop_start = loop_start as i64;
                t.num_samples = total as i64;
                t.channels = prev.0;
                t.sample_rate = prev.1;
                if !parse_header(sb, &mut t, &b, sb.s2_off + sb.cfg.s2_entry * n, n) || t.kind == Kind::None || t.kind == Kind::Sequence {
                    err = Some(String::new());
                    break;
                }
                let seg = match t.kind {
                    Kind::Silence => Ok(silence_seg(&t, prev)),
                    Kind::Audio => audio_seg(ctx, sb, &t, own)?,
                    Kind::Layer if sb.is_ps2_old => audio_seg(ctx, sb, &t, own)?,
                    Kind::Layer => layer_seg(ctx, sb, &t)?,
                    _ => Err(String::new()),
                };
                match seg {
                    Ok(s) => {
                        if i as i64 == h.seq_loop_start {
                            loop_start = total;
                        }
                        total += s.samples;
                        if !s.silence {
                            prev = (s.channels, s.rate);
                        } else {
                            prev = (t.channels, t.sample_rate);
                        }
                        segs.push(s);
                    }
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                }
            }
            match err {
                Some(e) => Err(e),
                None if segs.is_empty() => Err(String::new()),
                None => {
                    let l = if h.seq_num_loops == 0 { vgm_loop(loop_start as i64, total as i64, total) } else { None };
                    build(ctx, segs, l)
                }
            }
        }
        _ => Err(String::new()),
    })
}

// ------------------------------------------------------------------ banks and maps

/// parse_sb: the subsong headers of one bank/submap (allowed types only).
fn bank_headers(sb: &Sb, b: &B) -> Option<Vec<(u64, u64)>> {
    if sb.s1_num > MAX_SUBSONGS || sb.s2_num > MAX_SUBSONGS || sb.s3_num > MAX_SUBSONGS {
        return None;
    }
    let mut list = Vec::new();
    for i in 0..sb.s2_num {
        let off = sb.s2_off + sb.cfg.s2_entry * i;
        if off + 8 > b.d.len() as u64 {
            return None;
        }
        let t = b.u32(off + 4);
        if t >= 0x10 {
            return None;
        }
        let allowed = matches!(t, 0x01 | 0x05 | 0x0c | 0x06 | 0x0d) || (sb.is_ps2_bnm && t == 0x0b);
        if allowed {
            list.push((off, i));
        }
    }
    Some(list)
}

fn emit(ctx: &mut Ctx, sb: &Sb, d: &[u8], found: &mut Vec<Found>, size: u64) -> io::Result<()> {
    let b = B { d, be: sb.be };
    let Some(list) = bank_headers(sb, &b) else { return Ok(()) };
    for (off, index) in list {
        let mut h = Hdr::default();
        if !parse_header(sb, &mut h, &b, off, index) {
            continue; // vgmstream fails this subsong
        }
        let label = if sb.is_map { format!("{}-{:04}", sb.map_name, index) } else { format!("{index:04}") };
        match make_track(ctx, sb, &h, d)? {
            Ok(mut t) => {
                t.offset = off;
                found.push(Found::new(t, size).label(Some(label)));
            }
            Err(note) if !note.is_empty() => {
                let mut t = Track::new(ctx.entry, off, "Ubi SB", h.channels.max(1) as u16, h.sample_rate.max(1), h.num_samples.max(0) as u64, Data::at(ctx.entry, 0, 0), Codec::None);
                t.note = Some(note);
                found.push(Found::new(t, size).label(Some(label)));
            }
            Err(_) => {}
        }
    }
    Ok(())
}

/// Splinter Cell: Pandora Tomorrow (PS2): 33 maps (lame autodetection, like vgmstream's).
fn scpt_quirks(sb: &mut Sb, b: &B) {
    if sb.version == 7 && b.u32(0x08) == 0x21 {
        sb.cfg.map_entry_size = 0x38;
        sb.cfg.map_name = 0x18;
        sb.cfg.has_rs_files = true;
        sb.cfg.audio_ram_streamed_flag = 0x1c;
        sb.cfg.ram_streamed_and = 1 << 3;
        sb.cfg.loop_and = 1 << 4;
        sb.cfg.layer_hijack = 2;
    }
}

fn parse(ctx: &mut Ctx, off: u64) -> io::Result<Vec<Found>> {
    if off != 0 || ctx.size() < 0x20 || ctx.size() > 512 << 20 {
        return Ok(vec![]);
    }
    let size = ctx.size();
    let ext = ctx.ext();
    let d = ctx.bytes(0, size as usize)?;
    let mut sb = Sb { be: false, ..Default::default() };
    let b = B { d: &d, be: false };
    let mut found = Vec::new();
    match ext.as_str() {
        "bnm" => {
            if b.u32(0) != u32::from_le_bytes(*b"psx2") {
                return Ok(vec![]);
            }
            sb.is_ps2_bnm = true;
            sb.version = b.u32(0);
            if !config_version(&mut sb, ctx) {
                return Ok(vec![]);
            }
            sb.bank_number = b.u32(0x04);
            sb.s1_off = b.u32(0x08) as u64;
            sb.s1_num = b.u32(0x0c) as u64;
            sb.s2_off = b.u32(0x10) as u64;
            sb.s2_num = b.u32(0x14) as u64;
            sb.sx_off = b.u32(0x18) as u64;
            let bank_size = b.u32(0x1c) as u64;
            sb.sx_size = bank_size.wrapping_sub(sb.sx_off);
            if sb.s2_off > size || sb.sx_off > size {
                return Ok(vec![]);
            }
            emit(ctx, &sb, &d, &mut found, size)?;
        }
        "sb1" => {
            sb.version = b.u32(0);
            if !config_version(&mut sb, ctx) {
                return Ok(vec![]);
            }
            scpt_quirks(&mut sb, &b);
            if sb.version <= 0x0B {
                sb.s1_num = b.u32(0x04) as u64;
                sb.s2_num = b.u32(0x0c) as u64;
                sb.s3_num = b.u32(0x14) as u64;
                sb.sx_size = b.u32(0x1c) as u64;
                sb.s1_off = 0x20;
            } else if sb.version <= 0x000A0000 {
                sb.s1_num = b.u32(0x04) as u64;
                sb.s2_num = b.u32(0x08) as u64;
                sb.s3_num = b.u32(0x0c) as u64;
                sb.sx_size = b.u32(0x10) as u64;
                sb.s1_off = 0x18;
            } else {
                sb.s1_num = b.u32(0x04) as u64;
                sb.s2_num = b.u32(0x08) as u64;
                sb.s3_num = b.u32(0x0c) as u64;
                sb.sx_size = b.u32(0x10) as u64;
                sb.s1_off = 0x1c;
            }
            if sb.s1_num > MAX_SUBSONGS || sb.s2_num > MAX_SUBSONGS || sb.s3_num > MAX_SUBSONGS {
                return Ok(vec![]);
            }
            let c = &sb.cfg;
            if c.padded_s1 {
                sb.s1_off = align16(sb.s1_off);
            }
            sb.s2_off = sb.s1_off + c.s1_entry * sb.s1_num;
            if c.padded_s2 {
                sb.s2_off = align16(sb.s2_off);
            }
            sb.sx_off = sb.s2_off + c.s2_entry * sb.s2_num;
            if c.padded_sx {
                sb.sx_off = align16(sb.sx_off);
            }
            sb.s3_off = sb.sx_off + sb.sx_size;
            if c.padded_s3 {
                sb.s3_off = align16(sb.s3_off);
            }
            if sb.s3_off + sb.cfg.s3_entry * sb.s3_num > size {
                return Ok(vec![]);
            }
            emit(ctx, &sb, &d, &mut found, size)?;
        }
        _ => {
            // .sm1/.lm1 maps
            sb.is_map = true;
            sb.version = b.u32(0);
            let map_start = b.u32(4) as u64;
            let map_num = b.u32(8) as u64;
            if map_num >= 1024 || map_num == 0 {
                return Ok(vec![]);
            }
            if !config_version(&mut sb, ctx) {
                return Ok(vec![]);
            }
            scpt_quirks(&mut sb, &b);
            for i in 0..map_num {
                let o = map_start + i * sb.cfg.map_entry_size;
                if o + sb.cfg.map_entry_size > size {
                    return Ok(found);
                }
                let map_offset = b.u32(o + 8) as u64;
                sb.map_name = b.name(o + sb.cfg.map_name, 0);
                let s1 = b.u32(map_offset + 4) as u64;
                sb.s1_off = s1 + map_offset;
                sb.s1_num = b.u32(map_offset + 8) as u64;
                sb.s2_off = b.u32(map_offset + 0x0c) as u64 + map_offset;
                sb.s2_num = b.u32(map_offset + 0x10) as u64;
                if sb.cfg.map_version < 3 {
                    sb.s3_off = b.u32(map_offset + 0x14) as u64 + map_offset;
                    sb.s3_num = b.u32(map_offset + 0x18) as u64;
                    sb.sx_off = b.u32(map_offset + 0x1c) as u64 + map_offset;
                    sb.sx_size = b.u32(map_offset + 0x20) as u64;
                } else {
                    let s4_off = b.u32(map_offset + 0x14) as u64;
                    let s4_num = b.u32(map_offset + 0x18) as u64;
                    sb.s3_off = b.u32(map_offset + 0x1c) as u64 + map_offset;
                    sb.s3_num = b.u32(map_offset + 0x20) as u64;
                    sb.sx_off = b.u32(map_offset + 0x24) as u64 + map_offset;
                    sb.sx_size = b.u32(map_offset + 0x28) as u64;
                    sb.s2_num += s4_num;
                    sb.sx_off += s4_off;
                }
                if map_offset >= size || sb.s2_off > size {
                    return Ok(found);
                }
                emit(ctx, &sb, &d, &mut found, size)?;
            }
        }
    }
    Ok(found)
}
