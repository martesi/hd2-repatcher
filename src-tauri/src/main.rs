// In release builds on Windows, run as a GUI (no console window). CLI mode
// only runs when a parent console is actually attached to; otherwise any
// argv is treated as dropped paths for the GUI. Debug builds keep a console.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod audio;
mod cli;
mod commands;
mod gui;

use clap::Parser;

fn main() {
    if cli::console::has_console() {
        let args = cli::Args::parse();

        // Patch folders present -> headless CLI run; the GUI is never created.
        if !args.patches.is_empty() {
            std::process::exit(cli::run(args));
        }

        // `--game` without a patch folder is an error, matching the Python CLI
        // (config-only invocations still need a folder to act on).
        if args.game.is_some() {
            eprintln!("error: at least one PATCH_FOLDER is required with -g/--game");
            std::process::exit(2);
        }

        // No CLI arguments -> launch the GUI; nothing to seed it with.
        gui::run(Vec::new());
        return;
    }

    // No parent console: launched by double-click or drag-and-drop onto the
    // exe. Explorer never passes flags this way, only dropped paths - hand
    // them to the GUI exactly like a live window drop, on this instance or an
    // already-running one (see tauri_plugin_single_instance in gui.rs).
    let dropped: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    gui::run(dropped);
}
