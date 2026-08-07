//! Filesystem helpers around audio-mod patch files that sit outside the pure
//! engine: staging a working copy for dry-run, and cleaning up the original
//! per-mod patch files after `engine::process_audio_patches` merges them into
//! a combined `9ba626afa44a3aa3.patch_0`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Companion resource extensions that sit beside a `.patch_N` file.
const COMPANION_SUFFIXES: [&str; 2] = [".stream", ".gpu_resources"];

/// Deletes each classified audio patch plus its `.stream` / `.gpu_resources`
/// companions. Called after a successful `engine::process_audio_patches`,
/// which merges to a single `9ba626afa44a3aa3.patch_0` whose name differs
/// from the original inputs, leaving them orphaned otherwise.
pub fn remove_stale_audio_files(audio_patches: &[PathBuf]) {
    for patch in audio_patches {
        let _ = std::fs::remove_file(patch);
        for suffix in COMPANION_SUFFIXES {
            let mut companion: OsString = patch.clone().into_os_string();
            companion.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(companion));
        }
    }
}

/// Recursively copies `src` into `dst` (creating `dst`). Used to stage a working
/// copy for dry-run / `-o` so the originals are never modified.
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst)
        .map_err(|e| format!("failed to create '{}': {e}", dst.display()))?;
    let entries = std::fs::read_dir(src)
        .map_err(|e| format!("failed to read '{}': {e}", src.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let dest = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest)?;
        } else {
            std::fs::copy(&path, &dest)
                .map_err(|e| format!("failed to copy '{}': {e}", path.display()))?;
        }
    }
    Ok(())
}
