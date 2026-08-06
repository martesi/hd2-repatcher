//! Port of `reference/settings.py`. Reads/writes the same JSON file that the
//! Python tool used (via `platformdirs`), so a previously cached game data path
//! carries over, and adds `theme`/`accent` for the GUI.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const APP_NAME: &str = "hd2-repatcher";

/// Persisted app settings. Unknown keys are ignored on load and existing keys
/// are preserved, so this stays compatible with the Python `settings.json`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_data_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_tool_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
}

/// The settings.json location, matching `platformdirs.user_config_dir(APP_NAME)`.
/// On Windows `appauthor` defaults to the app name, nesting it twice under
/// `%LOCALAPPDATA%`; XDG/macOS use a single app-name directory.
pub fn settings_path() -> PathBuf {
    let base = dirs::config_local_dir().unwrap_or_else(|| PathBuf::from("."));
    #[cfg(windows)]
    let dir = base.join(APP_NAME).join(APP_NAME);
    #[cfg(not(windows))]
    let dir = base.join(APP_NAME);
    dir.join("settings.json")
}

/// Loads settings, returning defaults if the file is missing or unreadable.
pub fn load() -> Settings {
    let path = settings_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Settings::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Writes settings to disk (pretty-printed, like the Python `indent=2`).
pub fn save(settings: &Settings) -> std::io::Result<()> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(settings)?;
    std::fs::write(path, text)
}

pub fn get_cached_game_data_path() -> Option<String> {
    load().game_data_path
}

pub fn set_cached_game_data_path(path: &str) -> std::io::Result<()> {
    let mut settings = load();
    settings.game_data_path = Some(path.to_string());
    save(&settings)
}

pub fn get_cached_audio_tool_path() -> Option<String> {
    load().audio_tool_path
}

pub fn set_cached_audio_tool_path(path: &str) -> std::io::Result<()> {
    let mut settings = load();
    settings.audio_tool_path = Some(path.to_string());
    save(&settings)
}
