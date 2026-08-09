//! Headless CLI mode. Every input is processed as either one selected patch
//! group or all groups beneath a selected directory; no neighbouring group is
//! merged, renamed, or removed.

use std::path::{Path, PathBuf};

use clap::Parser;
use engine::{settings, GameResources, PatchKind};

use crate::patching::{self, GroupOutcome};

pub mod console;

#[derive(Parser, Debug)]
#[command(
    name = "hd2-repatcher",
    about = "Repatch Helldivers II unit and audio patch groups in place.",
    disable_help_subcommand = true
)]
pub struct Args {
    /// path to the Helldivers II install root; its `data` folder is used and cached for future runs
    #[arg(short = 'g', long = "game", value_name = "PATH")]
    pub game: Option<String>,

    /// do not save or overwrite the cached game install root
    #[arg(long = "no-game-path-caching")]
    pub no_game_path_caching: bool,

    /// patch file, companion file, or folder(s) containing patch groups
    #[arg(value_name = "PATCH_PATH")]
    pub patches: Vec<String>,
}

fn absolute(path: &str) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| PathBuf::from(path))
}

/// Runs the CLI and returns a non-zero exit code if any input group failed or
/// the configured game path was invalid.
pub fn run(args: Args) -> i32 {
    let mut game_root = None;
    if let Some(game) = &args.game {
        let path = absolute(game);
        if !engine::is_valid_game_root_path(&path) {
            eprintln!(
                "error: '{}' does not look like a Helldivers II install root \
                 (expected to find a `data` folder containing `{}` or `{}`)",
                game,
                engine::LEGACY_MARKER_FILE,
                engine::SLIM_MARKER_FILE
            );
            return 1;
        }
        if !args.no_game_path_caching {
            let _ = settings::set_cached_game_root(&path.to_string_lossy());
        }
        println!("Game install root set to: {}", path.display());
        game_root = Some(path);
    }

    run_cli(game_root, &args)
}

#[derive(Default)]
struct CliSummary {
    patches_found: usize,
    updated: Vec<String>,
    audio_updated: Vec<String>,
    no_units: Vec<String>,
    corrupted: Vec<String>,
}

fn run_cli(game_root: Option<PathBuf>, args: &Args) -> i32 {
    let game_root = match game_root {
        Some(path) => path,
        None => match settings::get_cached_game_root() {
            Some(cached) if engine::is_valid_game_root_path(Path::new(&cached)) => {
                PathBuf::from(cached)
            }
            _ => {
                eprintln!("error: no game install root configured; pass -g/--game <path>");
                return 1;
            }
        },
    };

    // Recheck immediately before loading and patching. This guards cached
    // paths that became invalid after the argument parse and ensures no patch
    // mutation begins with an invalid game install.
    if !engine::is_valid_game_root_path(&game_root) {
        eprintln!("error: configured Helldivers II install root is no longer valid");
        return 1;
    }
    let game_data = engine::game_data_path(&game_root);
    println!("Loading game resources from: {}", game_data.display());
    let resources = GameResources::load(&game_data);

    let mut exit_code = 0;
    for input in &args.patches {
        let source = absolute(input);
        let mut summary = CliSummary::default();
        if source.is_dir() {
            let candidates = engine::find_patch_files(&source);
            summary.patches_found = candidates.len();
            for candidate in candidates {
                process_candidate(&candidate, &resources, &mut summary);
            }
        } else {
            summary.patches_found = 1;
            process_candidate(&source, &resources, &mut summary);
        }

        if !summary.corrupted.is_empty() {
            exit_code = 1;
        }
        print_cli_result(&source, &summary);
    }
    exit_code
}

fn process_candidate(path: &Path, resources: &GameResources, summary: &mut CliSummary) {
    let group = match engine::PatchFileGroup::resolve(path) {
        Ok(group) => group,
        Err(error) => {
            summary
                .corrupted
                .push(format!("{}: {error}", path.display()));
            return;
        }
    };

    match patching::process_patch_group(&group, resources) {
        Ok(GroupOutcome::Updated(kind)) => {
            let name = group.main.to_string_lossy().into_owned();
            if matches!(kind, PatchKind::Unit | PatchKind::UnitAndAudio) {
                summary.updated.push(name.clone());
            }
            if matches!(kind, PatchKind::Audio | PatchKind::UnitAndAudio) {
                summary.audio_updated.push(name);
            }
        }
        Ok(GroupOutcome::Skipped(_)) => {
            summary
                .no_units
                .push(group.main.to_string_lossy().into_owned());
        }
        Err(error) => summary
            .corrupted
            .push(format!("{}: {error}", group.main.display())),
    }
}

fn print_cli_result(source: &Path, summary: &CliSummary) {
    println!("\n{}", source.display());
    if summary.patches_found == 0 {
        println!("  No patch files found.");
        return;
    }
    println!("  Checked {} patch group(s)", summary.patches_found);
    println!(
        "  Updated {} patch group(s) containing unit resources",
        summary.updated.len()
    );
    if !summary.audio_updated.is_empty() {
        println!(
            "  Patched {} audio patch group(s)",
            summary.audio_updated.len()
        );
    }
    if !summary.no_units.is_empty() {
        println!(
            "  Skipped {} patch group(s) with no unit or audio resources",
            summary.no_units.len()
        );
    }
    if !summary.corrupted.is_empty() {
        eprintln!(
            "  Failed to patch {} group(s):",
            summary.corrupted.len()
        );
        for name in &summary.corrupted {
            eprintln!("    {name}");
        }
    }
}
