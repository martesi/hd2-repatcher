//! Native Rust port of the HD2 Repatcher patching engine.
//!
//! This is a behaviour-preserving translation of the original Python engine
//! (`reference/update_unit_mods.py` + `reference/slim.py`). Structure, constants
//! and offset arithmetic are kept as close to the source as practical so the two
//! can be diffed, and a differential golden-fixture test suite
//! (`tests/golden.rs`) pins the byte-exact output against the Python oracle.

mod memstream;
mod patch;
mod resources;
mod slim;

pub mod settings;

pub use patch::{update_patch_file, PatchOutcome, UnitData, UnitDataSource};
pub use resources::GameResources;

use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Marker file present in legacy Helldivers II `data` installs.
pub const LEGACY_MARKER_FILE: &str = "9ba626afa44a3aa3";
/// Marker file present in "slim" (bundled) Helldivers II `data` installs.
pub const SLIM_MARKER_FILE: &str = "bundles.nxa";
/// Type id that identifies a unit resource in a package TOC.
pub const UNIT_TYPE_ID: u64 = 16187218042980615487;

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

/// Aggregated result of processing a folder of patch files. Field names mirror
/// the Python `PatchResult` dataclass.
#[derive(Debug, Default, Clone, Serialize, PartialEq, Eq)]
pub struct PatchResult {
    pub directory: String,
    pub patches_found: usize,
    pub updated: Vec<String>,
    pub no_units: Vec<String>,
    pub corrupted_files: Vec<String>,
}

/// Processes a list of patch files in parallel and buckets them by outcome,
/// preserving input order within each bucket (matching the Python engine, which
/// collects futures in submission order).
pub fn process_patch_files(patches: &[PathBuf], source: &(impl UnitDataSource + Sync)) -> PatchResult {
    let outcomes: Vec<(PatchOutcome, &PathBuf)> = patches
        .par_iter()
        .map(|p| (update_patch_file(p, source), p))
        .collect();

    let mut result = PatchResult {
        patches_found: patches.len(),
        ..Default::default()
    };
    for (outcome, path) in outcomes {
        let path = path.to_string_lossy().into_owned();
        match outcome {
            PatchOutcome::Updated => result.updated.push(path),
            PatchOutcome::NoUnits => result.no_units.push(path),
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
