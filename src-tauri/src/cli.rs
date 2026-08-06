//! Headless CLI mode — a faithful port of `reference/cli.py`. Runs when the
//! binary is launched with arguments; the GUI is never created in this path.

use std::path::{Path, PathBuf};

use clap::Parser;
use engine::{settings, GameResources, PatchResult};

use crate::audio;

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

    /// path to the external audio tool executable; also cached for future runs
    #[arg(long = "audio-tool", value_name = "PATH")]
    pub audio_tool: Option<String>,

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

    // Resolve + cache the audio tool path (mirrors the -g caching block above).
    let mut audio_tool: Option<PathBuf> = None;
    if let Some(t) = &args.audio_tool {
        let p = absolute(t);
        if !p.is_file() {
            eprintln!("error: '{t}' is not an executable file");
            return 1;
        }
        let _ = settings::set_cached_audio_tool_path(&p.to_string_lossy());
        println!("Audio tool set to: {}", p.display());
        audio_tool = Some(p);
    }

    run_cli(game_path, audio_tool, &args)
}

fn run_cli(game_path: Option<PathBuf>, audio_tool: Option<PathBuf>, args: &Args) -> i32 {
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

    // Fall back to the cached audio tool path when the flag was not passed.
    let audio_tool = audio_tool.or_else(|| {
        settings::get_cached_audio_tool_path()
            .map(PathBuf::from)
            .filter(|p| p.is_file())
    });

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

        // 1. Unit patching on the working dir.
        let result = engine::process_patch_folder(&work_dir, &resources);
        if !result.corrupted_files.is_empty() {
            exit_code = 1;
        }

        // 2. Delegate each directory that directly holds audio patches.
        let audio_dirs = audio::find_audio_dirs(&work_dir);
        if !audio_dirs.is_empty() {
            match &audio_tool {
                Some(tool) => {
                    for (dir, patches) in &audio_dirs {
                        if let Err(e) = audio::repatch_audio_dir(tool, &game_path, dir, patches) {
                            eprintln!("error: {e}");
                            exit_code = 1;
                        }
                    }
                }
                None => {
                    eprintln!(
                        "error: '{}' contains {} audio patch folder(s) but no audio tool is \
                         configured; pass --audio-tool <path>",
                        src.display(),
                        audio_dirs.len()
                    );
                    exit_code = 1;
                }
            }
        }

        let dest = copy.then(|| work_dir.clone());
        let delegated = audio_tool.is_some();
        print_cli_result(&src, dest.as_deref(), &result, &audio_dirs, delegated);
    }
    exit_code
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

fn print_cli_result(
    directory: &Path,
    dest: Option<&Path>,
    result: &PatchResult,
    audio_dirs: &[(PathBuf, Vec<PathBuf>)],
    delegated: bool,
) {
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
    if !result.audio.is_empty() {
        if delegated {
            println!(
                "  Delegated {} audio patch file(s) across {} folder(s) to the audio tool",
                result.audio.len(),
                audio_dirs.len()
            );
        } else {
            println!(
                "  Found {} audio patch file(s) across {} folder(s) but skipped them \
                 (no audio tool configured)",
                result.audio.len(),
                audio_dirs.len()
            );
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
