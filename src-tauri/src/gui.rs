//! Tauri GUI setup. Reached whenever there is no parent console to run a
//! headless CLI pass in (see `main.rs`) - including double-click/drag-drop
//! launches, whose dropped paths arrive here as `cli_paths`.

use std::sync::Mutex;

use tauri::{Emitter, Manager};

use crate::commands::{self, AppState};

/// `cli_paths` are patch folders dropped onto the exe at launch (empty for a
/// plain GUI open). They're stashed in state and pulled once by the frontend
/// via `take_startup_paths` rather than emitted directly, since the frontend
/// isn't listening yet this early in the window's lifecycle.
pub fn run(cli_paths: Vec<String>) {
    tauri::Builder::default()
        // Must be the first plugin registered: it decides before anything
        // else whether this process is a second launch, in which case it
        // forwards argv to the already-running instance and this process
        // exits without ever reaching `.setup()`.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let paths: Vec<String> = argv.into_iter().skip(1).collect();
            if !paths.is_empty() {
                let _ = app.emit("cli://open-paths", paths);
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            startup_paths: Mutex::new(cli_paths),
            ..Default::default()
        })
        .setup(|app| {
            // Index the cached game data path (if any) up front so dropped
            // batches can run immediately.
            commands::preload_cached_resources(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::set_game_path,
            commands::set_audio_tool_path,
            commands::clear_audio_tool_path,
            commands::set_theme,
            commands::set_accent,
            commands::init_game_resources,
            commands::process_batch,
            commands::take_startup_paths,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
