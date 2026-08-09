//! Native Rust port of the HD2 Repatcher patching engine.
//!
//! This is a behaviour-preserving translation of the original Python engine
//! (`reference/update_unit_mods.py` + `reference/slim.py`). Structure, constants
//! and offset arithmetic are kept as close to the source as practical so the two
//! can be diffed, and a differential golden-fixture test suite
//! (`tests/golden.rs`) pins the byte-exact output against the Python oracle.

mod audio_resources;
mod memstream;
mod patch;
mod patch_group;
mod resources;
mod slim;
pub mod wwise;

pub mod settings;

pub use audio_resources::AudioIndex;
pub use patch::{update_patch_file, PatchOutcome, UnitData, UnitDataSource};
pub use patch_group::{
    find_patch_file_groups, find_patch_files, is_main_patch_file, resolve_patch_file_group,
    PatchFileGroup, PatchFileGroupError,
};
pub use resources::GameResources;
pub use slim::Slim;

use rayon::prelude::*;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Marker file present in legacy Helldivers II `data` installs.
pub const LEGACY_MARKER_FILE: &str = "9ba626afa44a3aa3";
/// Marker file present in "slim" (bundled) Helldivers II `data` installs.
pub const SLIM_MARKER_FILE: &str = "bundles.nxa";
/// Type id that identifies a unit resource in a package TOC.
pub const UNIT_TYPE_ID: u64 = 16187218042980615487;

// Audio resource type ids (from the original audio tool's `const.py`). A patch
// file that carries any of these is handled by the native audio merger.
/// Wwise soundbank resource.
pub const WWISE_BANK: u64 = 6006249203084351385;
/// Wwise streamed-audio resource.
pub const WWISE_STREAM: u64 = 5785811756662211598;
/// Wwise dependency resource.
pub const WWISE_DEP: u64 = 12624162998411505776;
/// Text/string bank resource (`TEXT_BANK` == `STRING` in the audio tool).
pub const TEXT_BANK: u64 = 979299457696010195;
/// Bink video resource.
pub const BINK_VIDEO: u64 = 6838244362054241717;

/// True when `type_id` is one of the audio resource types.
pub fn is_audio_type_id(type_id: u64) -> bool {
    matches!(
        type_id,
        WWISE_BANK | WWISE_STREAM | WWISE_DEP | TEXT_BANK | BINK_VIDEO
    )
}

/// What kind of resources a patch file carries, from a type-table scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchKind {
    /// Unit resources only.
    Unit,
    /// Audio resources only.
    Audio,
    /// Both unit and audio resources.
    UnitAndAudio,
    /// Neither unit nor audio resources.
    Other,
    /// The type table could not be read (malformed header / I/O error).
    Corrupted,
}

fn u32_le(buf: &[u8], off: usize) -> Option<u32> {
    buf.get(off..off + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}

fn u64_le(buf: &[u8], off: usize) -> Option<u64> {
    buf.get(off..off + 8)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
}

/// Classifies a patch file by scanning its type table for unit / audio type ids.
///
/// This does its own read + type-table walk (the same 72-byte header → 32-byte
/// type-entry stride as [`update_patch_file`]) rather than sharing that hot path,
/// so the byte-exact golden tests stay untouched. Unlike `update_patch_file` it
/// scans **every** type entry (no early break) so a file carrying both unit and
/// audio types is reported as [`PatchKind::UnitAndAudio`].
pub fn classify_patch_file(path: &Path) -> PatchKind {
    let Ok(data) = std::fs::read(path) else {
        return PatchKind::Corrupted;
    };
    let Some(num_types) = u32_le(&data, 4) else {
        return PatchKind::Corrupted;
    };
    let mut has_unit = false;
    let mut has_audio = false;
    let mut pos = 72;
    for _ in 0..num_types as usize {
        pos += 8; // padding before the type id (matches update_patch_file)
        let Some(type_id) = u64_le(&data, pos) else {
            return PatchKind::Corrupted;
        };
        pos += 24; // type id (8) + count (8) + trailing padding (8)
        if type_id < (1u64 << 32) {
            return PatchKind::Corrupted;
        }
        if type_id == UNIT_TYPE_ID {
            has_unit = true;
        } else if is_audio_type_id(type_id) {
            has_audio = true;
        }
    }
    match (has_unit, has_audio) {
        (true, true) => PatchKind::UnitAndAudio,
        (true, false) => PatchKind::Unit,
        (false, true) => PatchKind::Audio,
        (false, false) => PatchKind::Other,
    }
}

/// True when `path` looks like a Helldivers II `data` folder (contains either
/// the legacy or the slim marker file).
pub fn is_valid_game_data_path(path: &Path) -> bool {
    path.is_dir()
        && (path.join(LEGACY_MARKER_FILE).exists() || path.join(SLIM_MARKER_FILE).exists())
}

/// Resolves the install root selected by users to Helldivers II's data folder.
pub fn game_data_path(game_root: &Path) -> PathBuf {
    game_root.join("data")
}

/// True when `path` is a Helldivers II install root containing a valid `data`
/// folder. The lower-level [`is_valid_game_data_path`] remains available for
/// callers that already operate on the derived data directory.
pub fn is_valid_game_root_path(path: &Path) -> bool {
    path.is_dir() && is_valid_game_data_path(&game_data_path(path))
}

/// Aggregated result of processing a folder of patch files. Field names mirror
/// the Python `PatchResult` dataclass.
#[derive(Debug, Default, Clone, Serialize, PartialEq, Eq)]
pub struct PatchResult {
    pub directory: String,
    pub patches_found: usize,
    pub updated: Vec<String>,
    pub no_units: Vec<String>,
    /// Patch files that carry audio resources; they no longer land in
    /// `no_units`.
    pub audio: Vec<String>,
    /// Retained for compatibility with the lower-level aggregation API. The
    /// application-level one-group-at-a-time workflow fills its own summaries
    /// after transactional processing.
    pub audio_updated: Vec<String>,
    /// Audio-carrying patch files that failed to merge, paired with the
    /// error message. See [`Self::audio_updated`]'s note on wiring.
    pub audio_failed: Vec<(String, String)>,
    pub corrupted_files: Vec<String>,
}

/// Processes a list of patch files in parallel and buckets them by outcome,
/// preserving input order within each bucket (matching the Python engine, which
/// collects futures in submission order).
pub fn process_patch_files(patches: &[PathBuf], source: &(impl UnitDataSource + Sync)) -> PatchResult {
    // Classify each file (for the audio bucket) alongside unit patching. A file
    // with no unit type makes `update_patch_file` return `NoUnits` without
    // writing, so running it on an audio/other file is a harmless no-op.
    let outcomes: Vec<(PatchKind, PatchOutcome, &PathBuf)> = patches
        .par_iter()
        .map(|p| (classify_patch_file(p), update_patch_file(p, source), p))
        .collect();

    let mut result = PatchResult {
        patches_found: patches.len(),
        ..Default::default()
    };
    for (kind, outcome, path) in outcomes {
        let path = path.to_string_lossy().into_owned();
        let is_audio = matches!(kind, PatchKind::Audio | PatchKind::UnitAndAudio);
        match outcome {
            // A UnitAndAudio file is patched *and* delegated: it lands in both.
            PatchOutcome::Updated => {
                if is_audio {
                    result.audio.push(path.clone());
                }
                result.updated.push(path);
            }
            PatchOutcome::NoUnits => {
                if is_audio {
                    result.audio.push(path);
                } else {
                    result.no_units.push(path);
                }
            }
            PatchOutcome::Corrupted => result.corrupted_files.push(path),
        }
    }
    result
}

/// Finds and processes every patch file under `directory`.
pub fn process_patch_folder(directory: &Path, source: &(impl UnitDataSource + Sync)) -> PatchResult {
    let patches = find_patch_files(directory);
    let mut result = process_patch_files(&patches, source);
    result.directory = directory.to_string_lossy().into_owned();
    result
}

/// Native implementation of the original audio tool's headless `run_patch_cli`
/// (`hd2-audio-modder/audio_modder.py:3647-3737`). The output keeps the first
/// selected main filename. Patch operations use one main file per call; the
/// slice remains for compatibility with lower-level multi-input callers.
///
/// Resolves each touched soundbank's containing base archive via
/// `resources.audio_index()` (this port's replacement for the external
/// friendlynames db real upstream's headless CLI downloads); a soundbank
/// with no indexed archive fails the whole call, matching upstream's
/// "cannot build a correct patch" early return. A patch carrying any
/// text-bank strings also pulls in the base `9ba626afa44a3aa3` archive
/// (upstream hardcodes the same archive for text banks).
pub fn process_audio_patches(dir: &Path, patches: &[PathBuf], resources: &GameResources) -> Result<(), String> {
    let output_filename = patches
        .first()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .ok_or("audio patch group has no valid main filename")?;
    process_audio_patches_to(dir, patches, resources, output_filename)
}

/// Merges the supplied audio patch files and writes the result using exactly
/// `output_filename`. Callers should pass one [`PatchFileGroup::main`] at a
/// time when performing an in-place repatch; the function accepts a slice so
/// the lower-level multi-input audio behavior remains available to fixtures
/// and library users.
pub fn process_audio_patches_to(
    dir: &Path,
    patches: &[PathBuf],
    resources: &GameResources,
    output_filename: &str,
) -> Result<(), String> {
    let mut sorted: Vec<&PathBuf> = patches.iter().collect();
    sorted.sort();

    let mut archives_to_load: BTreeSet<String> = BTreeSet::new();
    for path in &sorted {
        let Some(archive) = wwise::GameArchive::from_patch_file(path) else {
            return Err(format!("failed to read/parse patch file '{}'", path.display()));
        };
        if !archive.text_banks.is_empty() {
            archives_to_load.insert(LEGACY_MARKER_FILE.to_string());
        }
        for bank_id in archive.wwise_banks.keys() {
            match resources.audio_index().archive_name(*bank_id) {
                Some(name) => {
                    archives_to_load.insert(name.to_string());
                }
                None => {
                    return Err(format!(
                        "unable to locate base archive for soundbank {bank_id} (from '{}'); cannot build a correct patch",
                        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                    ));
                }
            }
        }
    }

    let mut mod_ = wwise::Mod::new();
    for archive_name in &archives_to_load {
        // Every name here is distinct (`archives_to_load` is a set) and
        // `mod_` starts empty, so the only way `load_base_archive` returns
        // false is a genuine load failure — an indexed package that's gone
        // from disk, or one whose toc/stream data won't parse. Bail rather
        // than merge into a pool that's missing the resources this patch
        // targets.
        if !mod_.load_base_archive(resources.slim(), archive_name) {
            return Err(format!(
                "failed to load base game archive '{archive_name}'; cannot build a correct patch"
            ));
        }
    }
    for path in &sorted {
        if !mod_.import_patch(path, true) {
            return Err(format!("failed to import patch file '{}'", path.display()));
        }
    }

    // A patch group can carry audio types whose base archive this port
    // can't resolve — `AudioIndex` maps soundbank ids only, so a group of
    // `WWISE_STREAM`-only files (still audio per `is_audio_type_id`) loads
    // no base archive at all, swaps nothing, and would write a
    // resource-less patch. Videos are the one legitimately archive-less
    // case and they do flag `modified`, so this guard leaves them alone.
    if !mod_.has_modified_resources() {
        return Err(format!(
            "no audio was replaced by {} patch file(s) (loaded base archive(s): {}); \
             refusing to write an empty patch",
            sorted.len(),
            if archives_to_load.is_empty() {
                "none".to_string()
            } else {
                archives_to_load.iter().cloned().collect::<Vec<_>>().join(", ")
            }
        ));
    }

    mod_.write_patch(dir, Some(output_filename))
        .map_err(|e| format!("failed to write patch to '{}': {e}", dir.display()))
}

/// Convenience wrapper for a single audio patch whose output keeps its main
/// filename. The caller is responsible for staging the group when atomic
/// replacement is required.
pub fn process_audio_patch_file(path: &Path, resources: &GameResources) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| format!("patch '{}' has no parent directory", path.display()))?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("patch '{}' has a non-UTF-8 filename", path.display()))?;
    process_audio_patches_to(dir, &[path.to_path_buf()], resources, filename)
}
