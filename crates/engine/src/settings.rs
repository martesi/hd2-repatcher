//! Persistent application configuration. The native app owns this TOML file;
//! it is intentionally separate from the legacy Python tool's JSON settings.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const APP_NAME: &str = "hd2-repatcher";

/// Persisted app settings.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
}

/// The config.toml location, matching `platformdirs.user_config_dir(APP_NAME)`.
/// On Windows `appauthor` defaults to the app name, nesting it twice under
/// `%LOCALAPPDATA%`; XDG/macOS use a single app-name directory.
pub fn settings_path() -> PathBuf {
    let base = dirs::config_local_dir().unwrap_or_else(|| PathBuf::from("."));
    #[cfg(windows)]
    let dir = base.join(APP_NAME).join(APP_NAME);
    #[cfg(not(windows))]
    let dir = base.join(APP_NAME);
    dir.join("config.toml")
}

/// Loads settings, returning defaults if the file is missing or unreadable.
pub fn load() -> Settings {
    let path = settings_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Settings::default();
    };
    toml::from_str(&text).unwrap_or_default()
}

/// Writes settings to disk as human-readable TOML.
pub fn save(settings: &Settings) -> std::io::Result<()> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(settings).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

pub fn get_cached_game_root() -> Option<String> {
    load().game_root
}

pub fn set_cached_game_root(path: &str) -> std::io::Result<()> {
    let mut settings = load();
    settings.game_root = Some(path.to_string());
    save(&settings)
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn settings_round_trip_as_toml() {
        let settings = Settings {
            game_root: Some("/games/Helldivers 2".into()),
            language: Some("en".into()),
            theme: Some("dark".into()),
            accent: Some("blue".into()),
        };

        let text = toml::to_string_pretty(&settings).unwrap();
        let parsed: Settings = toml::from_str(&text).unwrap();
        assert_eq!(parsed.game_root.as_deref(), Some("/games/Helldivers 2"));
        assert_eq!(parsed.language.as_deref(), Some("en"));
        assert_eq!(parsed.theme.as_deref(), Some("dark"));
        assert_eq!(parsed.accent.as_deref(), Some("blue"));
    }

    #[test]
    fn malformed_toml_uses_defaults() {
        let parsed: Settings = toml::from_str("not = [valid").unwrap_or_default();
        assert!(parsed.game_root.is_none());
        assert!(parsed.language.is_none());
    }
}
