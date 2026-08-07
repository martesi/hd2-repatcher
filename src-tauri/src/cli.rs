//! Headless CLI mode — a faithful port of `reference/cli.py`. Runs when the
//! binary is launched with arguments; the GUI is never created in this path.

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

use clap::Parser;
use engine::{settings, GameResources, PatchResult};

use crate::audio;

pub mod console;

/// Filename `engine::process_audio_patches` always writes its merged output
/// as. Excluded from stale-file cleanup so a per-mod input that happens to
/// already have this exact name isn't deleted out from under its own output.
const MERGED_AUDIO_PATCH_NAME: &str = "9ba626afa44a3aa3.patch_0";

#[derive(Parser, Debug)]
#[command(
    name = "hd2-repatcher",
    about = "Update unit resources in Helldivers II patch files.",
    disable_help_subcommand = true
)]
pub struct Args {
    /// path to the Helldivers II game data folder; also cached for future runs
    #[arg(short = 'g', long = "game", value_name = "PATH")]
    pub game: Option<String>,

    /// do not save or overwrite the cached game data path
    #[arg(long = "no-game-path-caching")]
    pub no_game_path_caching: bool,

    /// write repatched copies into DIR (one <name> subfolder per input folder)
    /// instead of modifying the originals in place
    #[arg(short = 'o', long = "output", value_name = "DIR")]
    pub output: Option<String>,

    /// operate on a copy, leaving the original mod files untouched; copies land
    /// in --output, or in a sibling `<name>-repatched/` folder when omitted
    #[arg(short = 'n', long = "dry-run")]
    pub dry_run: bool,

    /// folder(s) containing patch files to update
    #[arg(value_name = "PATCH_FOLDER")]
    pub patches: Vec<String>,
}

fn absolute(p: &str) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| PathBuf::from(p))
}

/// Runs the CLI and returns the process exit code (non-zero if any corrupted
/// or audio patch files failed to process, or an argument was invalid).
pub fn run(args: Args) -> i32 {
    let mut game_path: Option<PathBuf> = None;
    if let Some(g) = &args.game {
        let p = absolute(g);
        if !engine::is_valid_game_data_path(&p) {
            eprintln!(
                "error: '{}' does not look like a Helldivers II data folder \
                 (expected to find `{}` or `{}` inside it)",
                g,
                engine::LEGACY_MARKER_FILE,
                engine::SLIM_MARKER_FILE
            );
            return 1;
        }
        if !args.no_game_path_caching {
            let _ = settings::set_cached_game_data_path(&p.to_string_lossy());
        }
        println!("Game data directory set to: {}", p.display());
        game_path = Some(p);
    }

    run_cli(game_path, &args)
}

fn run_cli(game_path: Option<PathBuf>, args: &Args) -> i32 {
    let game_path = match game_path {
        Some(p) => p,
        None => match settings::get_cached_game_data_path() {
            Some(c) if engine::is_valid_game_data_path(Path::new(&c)) => PathBuf::from(c),
            _ => {
                eprintln!("error: no game data directory configured; pass -g/--game <path>");
                return 1;
            }
        },
    };

    println!("Loading game resources from: {}", game_path.display());
    let resources = GameResources::load(&game_path);

    let mut exit_code = 0;
    for patch_dir in &args.patches {
        let src = absolute(patch_dir);
        if !src.is_dir() {
            eprintln!("error: '{}' is not a directory", src.display());
            exit_code = 1;
            continue;
        }

        // Dry-run / -o operate on a copy so the originals are never touched.
        let copy = args.dry_run || args.output.is_some();
        let work_dir = if copy {
            let dest = output_dest(&src, args.output.as_deref());
            if dest == src {
                eprintln!("error: output destination for '{}' resolves to itself", src.display());
                exit_code = 1;
                continue;
            }
            let _ = std::fs::remove_dir_all(&dest);
            if let Err(e) = audio::copy_dir_recursive(&src, &dest) {
                eprintln!("error: {e}");
                exit_code = 1;
                continue;
            }
            dest
        } else {
            src.clone()
        };

        // 1. Unit patching on the working dir (also classifies audio-carrying
        // files into `result.audio`, but doesn't merge them).
        let mut result = engine::process_patch_folder(&work_dir, &resources);
        if !result.corrupted_files.is_empty() {
            exit_code = 1;
        }

        // 2. Natively merge each directory that directly holds audio patches.
        if !merge_audio_dirs(&work_dir, &resources, &mut result) {
            exit_code = 1;
        }

        let dest = copy.then(|| work_dir.clone());
        print_cli_result(&src, dest.as_deref(), &result);
    }
    exit_code
}

/// Merges every audio-patch directory under `work_dir` via
/// `engine::process_audio_patches`, filling `result.audio_updated`/
/// `audio_failed`. Returns `false` if any group failed.
fn merge_audio_dirs(work_dir: &Path, resources: &GameResources, result: &mut PatchResult) -> bool {
    let mut ok = true;
    for (dir, patches) in engine::find_audio_dirs(work_dir) {
        // A malformed/incomplete mod (e.g. a `.patch_N` missing its required
        // `.stream` companion) must not take down the whole run; treat a
        // panic as a per-directory failure, matching how a corrupt unit
        // patch is handled.
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            engine::process_audio_patches(&dir, &patches, resources)
        }))
        .unwrap_or_else(|_| Err("panicked while processing (malformed or incomplete mod?)".to_string()));
        match outcome {
            Ok(()) => {
                let stale: Vec<PathBuf> = patches
                    .iter()
                    .filter(|p| p.file_name().is_none_or(|n| n != MERGED_AUDIO_PATCH_NAME))
                    .cloned()
                    .collect();
                audio::remove_stale_audio_files(&stale);
                result
                    .audio_updated
                    .extend(patches.iter().map(|p| p.to_string_lossy().into_owned()));
            }
            Err(e) => {
                ok = false;
                for p in &patches {
                    result
                        .audio_failed
                        .push((p.to_string_lossy().into_owned(), e.clone()));
                }
            }
        }
    }
    ok
}

/// Computes the copy destination for a mod folder: `<output>/<name>` when
/// `--output` is given, otherwise a sibling `<name>-repatched/`.
fn output_dest(src: &Path, output: Option<&str>) -> PathBuf {
    let name = src.file_name().unwrap_or_default();
    match output {
        Some(o) => absolute(o).join(name),
        None => {
            let mut fname = name.to_os_string();
            fname.push("-repatched");
            src.parent().unwrap_or_else(|| Path::new(".")).join(fname)
        }
    }
}

fn print_cli_result(directory: &Path, dest: Option<&Path>, result: &PatchResult) {
    println!("\n{}", directory.display());
    if let Some(dest) = dest {
        println!("  Repatched copy written to: {}", dest.display());
    }
    if result.patches_found == 0 {
        println!("  No patch files found.");
        return;
    }
    println!("  Checked {} patch file(s)", result.patches_found);
    println!(
        "  Updated {} patch file(s) containing unit resources",
        result.updated.len()
    );
    if !result.audio_updated.is_empty() {
        println!(
            "  Patched {} audio patch file(s)",
            result.audio_updated.len()
        );
    }
    if !result.audio_failed.is_empty() {
        eprintln!(
            "  Failed to patch {} audio patch file(s):",
            result.audio_failed.len()
        );
        for (name, err) in &result.audio_failed {
            eprintln!("    {name}: {err}");
        }
    }
    if !result.no_units.is_empty() {
        println!(
            "  Skipped {} patch file(s) with no unit resources",
            result.no_units.len()
        );
    }
    if !result.corrupted_files.is_empty() {
        eprintln!(
            "  Found {} corrupted patch file(s):",
            result.corrupted_files.len()
        );
        for name in &result.corrupted_files {
            eprintln!("    {}", name);
        }
    }
}
