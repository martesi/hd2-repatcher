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
mod resources;
mod slim;
pub mod wwise;

pub mod settings;

pub use audio_resources::AudioIndex;
pub use patch::{update_patch_file, PatchOutcome, UnitData, UnitDataSource};
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

// Audio resource type ids (from the audio tool's `const.py`). A patch file that
// carries any of these is an audio mod and is delegated to the external audio
// tool rather than unit-patched here.
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
fn is_audio_type_id(type_id: u64) -> bool {
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

/// Recursively collects patch files under `directory`. Mirrors the Python rule:
/// a file counts when its extension contains the substring `patch` (e.g.
/// `.patch`, `.patch_0`).
pub fn find_patch_files(directory: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_patch_files(directory, &mut out);
    out
}

fn collect_patch_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_patch_files(&path, out);
        } else if path
            .extension()
            .map(|e| e.to_string_lossy().contains("patch"))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
}

/// Walks `root` and returns, for every directory that *directly* contains at
/// least one audio-carrying patch file, that directory paired with its sorted
/// audio patch files. Each group is one [`process_audio_patches`] call: the
/// native merge, like the external audio tool it replaces, is non-recursive
/// and combines everything in a directory into one output patch.
pub fn find_audio_dirs(root: &Path) -> Vec<(PathBuf, Vec<PathBuf>)> {
    let mut out = Vec::new();
    collect_audio_dirs(root, &mut out);
    out
}

fn collect_audio_dirs(dir: &Path, out: &mut Vec<(PathBuf, Vec<PathBuf>)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut audio_here: Vec<PathBuf> = Vec::new();
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
        } else if path
            .extension()
            .map(|e| e.to_string_lossy().contains("patch"))
            .unwrap_or(false)
            && matches!(classify_patch_file(&path), PatchKind::Audio | PatchKind::UnitAndAudio)
        {
            audio_here.push(path);
        }
    }
    if !audio_here.is_empty() {
        audio_here.sort();
        out.push((dir.to_path_buf(), audio_here));
    }
    subdirs.sort();
    for sub in subdirs {
        collect_audio_dirs(&sub, out);
    }
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
    /// Audio-carrying patch files successfully merged into the game data by
    /// [`process_audio_patches`]. Populated by the `src-tauri` layer (`cli.rs`,
    /// `commands.rs`), which groups `audio`/`no_units`-adjacent files via
    /// [`find_audio_dirs`] and calls [`process_audio_patches`] per group —
    /// not by [`process_patch_files`]/[`process_patch_folder`] themselves,
    /// since those stay generic over [`UnitDataSource`] alone (audio merging
    /// needs a concrete [`GameResources`] for its archive index).
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

/// Native replacement for the external audio tool's headless `run_patch_cli`
/// (`hd2-audio-modder/audio_modder.py:3647-3737`): merges every audio-carrying
/// `.patch_N` file in `patches` (a same-directory group — see
/// `find_audio_dirs`-style grouping, not yet wired into
/// [`process_patch_files`]/[`process_patch_folder`], see [`PatchResult`]'s
/// doc) into the base game archives they touch, then writes one combined
/// `9ba626afa44a3aa3.patch_0` into `dir`. `patches` need not be pre-sorted —
/// this sorts by path itself, matching Python's `sorted(os.listdir(...))`.
///
/// Resolves each touched soundbank's containing base archive via
/// `resources.audio_index()` (this port's replacement for the external
/// friendlynames db real upstream's headless CLI downloads); a soundbank
/// with no indexed archive fails the whole call, matching upstream's
/// "cannot build a correct patch" early return. A patch carrying any
/// text-bank strings also pulls in the base `9ba626afa44a3aa3` archive
/// (upstream hardcodes the same archive for text banks).
pub fn process_audio_patches(dir: &Path, patches: &[PathBuf], resources: &GameResources) -> Result<(), String> {
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
        mod_.load_base_archive(resources.slim(), archive_name);
    }
    for path in &sorted {
        if !mod_.import_patch(path, true) {
            return Err(format!("failed to import patch file '{}'", path.display()));
        }
    }

    mod_.write_patch(dir, Some("9ba626afa44a3aa3.patch_0"))
        .map_err(|e| format!("failed to write patch to '{}': {e}", dir.display()))
}
