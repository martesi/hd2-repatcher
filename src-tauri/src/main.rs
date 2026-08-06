// In release builds on Windows, run as a GUI (no console window). CLI mode
// attaches or allocates a console explicitly. Debug builds keep a console.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod cli;
mod commands;
mod gui;

use clap::Parser;

fn main() {
    // Only touch the console when launched with arguments, so a plain
    // double-click never flashes one.
    let has_args = std::env::args_os().count() > 1;
    let owns_console = has_args && cli::console::attach_or_alloc();

    let args = cli::Args::parse();

    // Patch folders present -> headless CLI run; the GUI is never created.
    if !args.patches.is_empty() {
        let code = cli::run(args);
        if owns_console {
            cli::console::pause();
        }
        std::process::exit(code);
    }

    // `--game` without a patch folder is an error, matching the Python CLI.
    if args.game.is_some() {
        eprintln!("error: at least one PATCH_FOLDER is required with -g/--game");
        if owns_console {
            cli::console::pause();
        }
        std::process::exit(2);
    }

    // No CLI arguments -> launch the GUI.
    gui::run();
}
