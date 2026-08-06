//! Audio-mod delegation. The repatcher cannot repatch audio itself; it hands
//! each mod folder that contains audio patches to an external audio tool via its
//! headless CLI (`<tool> patch -i <in> -o <out> [-g <data>]`, see
//! `hd2-audio-modder/docs/cli-patch-plan.md`) and relocates the freshly patched
//! output back into the mod folder.
//!
//! The engine stays subprocess-free and pure; all `std::process` and filesystem
//! relocation lives here in the tauri layer.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use engine::{classify_patch_file, PatchKind};

/// Companion resource extensions that sit beside a `.patch_N` file.
const COMPANION_SUFFIXES: [&str; 2] = [".stream", ".gpu_resources"];

/// True when a file name looks like a patch file (extension contains `patch`),
/// matching the engine's discovery rule.
fn is_patch_file(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().contains("patch"))
        .unwrap_or(false)
}

/// Walks `root` and returns, for every directory that *directly* contains at
/// least one audio patch file, that directory paired with its audio patch files.
///
/// The audio tool is non-recursive, so each such directory becomes one
/// delegation with that directory passed as `-i`.
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
        } else if is_patch_file(&path)
            && matches!(
                classify_patch_file(&path),
                PatchKind::Audio | PatchKind::UnitAndAudio
            )
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

/// Invokes the audio tool: `<tool> patch -i <in_dir> -o <out_dir> -g <game_data>`.
/// Maps a non-zero exit (or launch failure) to a clear error carrying stderr.
pub fn repatch_audio_mod(
    tool: &Path,
    game_data: &Path,
    in_dir: &Path,
    out_dir: &Path,
) -> Result<(), String> {
    let output = Command::new(tool)
        .arg("patch")
        .arg("-i")
        .arg(in_dir)
        .arg("-o")
        .arg(out_dir)
        .arg("-g")
        .arg(game_data)
        .output()
        .map_err(|e| format!("failed to launch audio tool '{}': {e}", tool.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "audio tool failed ({}) for '{}':\n{}",
            output.status,
            in_dir.display(),
            stderr.trim()
        ));
    }
    Ok(())
}

/// Deletes each classified audio patch plus its `.stream` / `.gpu_resources`
/// companions. Called *before* the freshly patched output is moved in, because
/// the tool merges to a single `9ba626afa44a3aa3.patch_0` whose name differs
/// from the original inputs, leaving them orphaned.
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

/// Repatches one audio directory in place: runs the tool into a fresh temp `-o`,
/// removes the stale audio inputs, then moves the patched output into `dir`.
/// Returns the file names moved into `dir`.
pub fn repatch_audio_dir(
    tool: &Path,
    game_data: &Path,
    dir: &Path,
    audio_patches: &[PathBuf],
) -> Result<Vec<String>, String> {
    let temp = make_temp_dir("hd2-audio-out")?;
    let result = (|| {
        repatch_audio_mod(tool, game_data, dir, &temp)?;
        remove_stale_audio_files(audio_patches);
        move_dir_contents(&temp, dir)
    })();
    let _ = std::fs::remove_dir_all(&temp);
    result
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

/// Moves every entry directly under `src` into `dst`. The audio tool writes
/// output files directly (no subfolders), so only files are expected here.
fn move_dir_contents(src: &Path, dst: &Path) -> Result<Vec<String>, String> {
    let mut moved = Vec::new();
    let entries = std::fs::read_dir(src)
        .map_err(|e| format!("failed to read audio output '{}': {e}", src.display()))?;
    for entry in entries.flatten() {
        let from = entry.path();
        let name = entry.file_name();
        let to = dst.join(&name);
        move_path(&from, &to)?;
        moved.push(name.to_string_lossy().into_owned());
    }
    Ok(moved)
}

/// Moves `from` to `to`, falling back to copy+remove across filesystems (the
/// temp dir and the mod folder may live on different mounts).
fn move_path(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)
        .map_err(|e| format!("failed to move '{}' -> '{}': {e}", from.display(), to.display()))?;
    let _ = std::fs::remove_file(from);
    Ok(())
}

/// Creates a fresh, uniquely named temp directory under the system temp dir.
fn make_temp_dir(prefix: &str) -> Result<PathBuf, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create temp dir '{}': {e}", dir.display()))?;
    Ok(dir)
}
