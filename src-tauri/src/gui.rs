//! Tauri GUI setup. Only reached when the binary is launched with no CLI
//! arguments.

use crate::commands::{self, AppState};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .setup(|app| {
            // Index the cached game data path (if any) up front so dropped
            // batches can run immediately.
            commands::preload_cached_resources(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::set_game_path,
            commands::set_theme,
            commands::set_accent,
            commands::init_game_resources,
            commands::process_batch,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
