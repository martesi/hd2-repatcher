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
pub mod text_bank;
pub mod video;

pub use archive::GameArchive;

/// Wwise plugin id for Vorbis-encoded sources.
pub const VORBIS: u32 = 0x0004_0001;
/// Wwise plugin id for "Rev Audio" (custom FX) sources.
pub const REV_AUDIO: u32 = 0x01A0_1052;
/// XOR key the BKHD chunk's version field is obfuscated with.
pub const BANK_VERSION_KEY: u32 = 0x9211_BCAC;
