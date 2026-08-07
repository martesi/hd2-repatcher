//! Native Rust port of the Wwise audio-patching engine (see
//! `.ref/native-audio-patch-plan.md`). A faithful translation of the trimmed
//! `hd2-audio-modder` Python tool vendored at `reference/audio_core.py` +
//! `reference/wwise_hierarchy_140.py` / `_154.py`; `tests/audio_golden.rs`
//! pins output byte-for-byte against that oracle.
//!
//! Ported incrementally, phase by phase (see the plan doc): this module
//! currently covers the TOC/archive skeleton with every HIRC entry treated as
//! an opaque `(type, size, id, misc-bytes)` blob. Structured HIRC types
//! (`Sound`, `MusicTrack`, the container types, ...) are added in later
//! phases.

pub mod archive;
pub mod audio_source;
pub mod bank_parser;
pub mod hierarchy;
pub mod mod_;
pub mod text_bank;
pub mod video;

pub use archive::GameArchive;
pub use mod_::Mod;

/// Wwise plugin id for Vorbis-encoded sources.
pub const VORBIS: u32 = 0x0004_0001;
/// Wwise plugin id for "Rev Audio" (custom FX) sources.
pub const REV_AUDIO: u32 = 0x01A0_1052;
/// XOR key the BKHD chunk's version field is obfuscated with.
pub const BANK_VERSION_KEY: u32 = 0x9211_BCAC;

/// Port of `audio_util.py::murmur64_hash` (a MurmurHash64A variant, seed 0).
/// Used to resolve a `WWISE_STREAM` TOC entry's resource id from a bank
/// dependency path + source id (`GameArchive::load`'s stream-source
/// resolution) — the one Wwise-specific path->id hash this engine needs;
/// unit patching never has to synthesize resource ids from a path, only
/// read ones already present in TOC/patch files.
pub fn murmur64_hash(data: &[u8]) -> u64 {
    const M: u64 = 0xC6A4_A793_5BD1_E995;
    const R: u32 = 47;

    let mut h = M.wrapping_mul(data.len() as u64);

    let tail_start = (data.len() / 8) * 8;
    for chunk in data[..tail_start].chunks_exact(8) {
        let mut k = u64::from_le_bytes(chunk.try_into().unwrap());
        k = k.wrapping_mul(M);
        k ^= k >> R;
        k = k.wrapping_mul(M);
        h ^= k;
        h = h.wrapping_mul(M);
    }

    let tail = &data[tail_start..];
    if tail.len() >= 7 {
        h ^= (tail[6] as u64) << 48;
    }
    if tail.len() >= 6 {
        h ^= (tail[5] as u64) << 40;
    }
    if tail.len() >= 5 {
        h ^= (tail[4] as u64) << 32;
    }
    if tail.len() >= 4 {
        h ^= (tail[3] as u64) << 24;
    }
    if tail.len() >= 3 {
        h ^= (tail[2] as u64) << 16;
    }
    if tail.len() >= 2 {
        h ^= (tail[1] as u64) << 8;
    }
    if !tail.is_empty() {
        h ^= tail[0] as u64;
        h = h.wrapping_mul(M);
    }

    h ^= h >> R;
    h = h.wrapping_mul(M);
    h ^= h >> R;
    h
}
