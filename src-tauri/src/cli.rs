//! Headless CLI mode — a faithful port of `reference/cli.py`. Runs when the
//! binary is launched with arguments; the GUI is never created in this path.

use std::path::{Path, PathBuf};

use clap::Parser;
use engine::{settings, GameResources, PatchResult};

pub mod console;

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

    /// folder(s) containing patch files to update
    #[arg(value_name = "PATCH_FOLDER")]
    pub patches: Vec<String>,
}

fn absolute(p: &str) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| PathBuf::from(p))
}

/// Runs the CLI and returns the process exit code (non-zero if any corrupted
/// patch files were found or an argument was invalid).
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

    run_cli(game_path, &args.patches)
}

fn run_cli(game_path: Option<PathBuf>, patch_dirs: &[String]) -> i32 {
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
    for patch_dir in patch_dirs {
        let p = absolute(patch_dir);
        if !p.is_dir() {
            eprintln!("error: '{}' is not a directory", p.display());
            exit_code = 1;
            continue;
        }
        let result = engine::process_patch_folder(&p, &resources);
        print_cli_result(&p, &result);
        if !result.corrupted_files.is_empty() {
            exit_code = 1;
        }
    }
    exit_code
}

fn print_cli_result(directory: &Path, result: &PatchResult) {
    println!("\n{}", directory.display());
    if result.patches_found == 0 {
        println!("  No patch files found.");
        return;
    }
    println!("  Checked {} patch file(s)", result.patches_found);
    println!(
        "  Updated {} patch file(s) containing unit resources",
        result.updated.len()
    );
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
