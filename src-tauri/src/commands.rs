//! Tauri command handlers and shared app state. The GUI talks to the native
//! engine exclusively through these.

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use engine::{settings, GameResources, PatchOutcome};
use rayon::prelude::*;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::audio;

/// The indexed game data, shared read-only across the batch worker threads.
#[derive(Default)]
pub struct AppState {
    pub resources: Mutex<Option<Arc<GameResources>>>,
    /// Patch folder paths passed on argv at launch (double-click/drag-drop
    /// onto the exe), consumed once by the frontend via `take_startup_paths`.
    pub startup_paths: Mutex<Vec<String>>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub game_data_path: Option<String>,
    pub game_data_valid: bool,
    pub audio_tool_path: Option<String>,
    pub audio_tool_valid: bool,
    pub theme: Option<String>,
    pub accent: Option<String>,
    /// Number of indexed unit resources (0 until game data is loaded).
    pub unit_count: usize,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct BatchProgress {
    id: String,
    status: String,
    patches_found: usize,
    checked: usize,
    updated: usize,
    skipped: usize,
    audio: usize,
    corrupted: Vec<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BatchResult {
    id: String,
    path: String,
    patches_found: usize,
    updated: usize,
    skipped: usize,
    audio: usize,
    corrupted: Vec<String>,
}

fn current_unit_count(app: &AppHandle) -> usize {
    app.state::<AppState>()
        .resources
        .lock()
        .unwrap()
        .as_ref()
        .map(|r| r.unit_count())
        .unwrap_or(0)
}

#[tauri::command]
pub fn get_config(app: AppHandle) -> Config {
    let s = settings::load();
    let game_data_valid = s
        .game_data_path
        .as_ref()
        .map(|p| engine::is_valid_game_data_path(Path::new(p)))
        .unwrap_or(false);
    let audio_tool_valid = s
        .audio_tool_path
        .as_ref()
        .map(|p| Path::new(p).is_file())
        .unwrap_or(false);
    Config {
        game_data_path: s.game_data_path,
        game_data_valid,
        audio_tool_path: s.audio_tool_path,
        audio_tool_valid,
        theme: s.theme,
        accent: s.accent,
        unit_count: current_unit_count(&app),
    }
}

/// Validates and persists the game data path. Returns whether it is valid; the
/// caller should follow up with `init_game_resources` to index it.
#[tauri::command]
pub fn set_game_path(path: String) -> Result<bool, String> {
    let valid = engine::is_valid_game_data_path(Path::new(&path));
    let mut s = settings::load();
    s.game_data_path = Some(path);
    settings::save(&s).map_err(|e| e.to_string())?;
    Ok(valid)
}

/// Persists the external audio tool path. Returns whether it points at an
/// existing file, which is the same check the CLI makes before delegating.
#[tauri::command]
pub fn set_audio_tool_path(path: String) -> Result<bool, String> {
    let valid = Path::new(&path).is_file();
    settings::set_cached_audio_tool_path(&path).map_err(|e| e.to_string())?;
    Ok(valid)
}

/// Forgets the configured audio tool, returning to the "not configured" state.
#[tauri::command]
pub fn clear_audio_tool_path() -> Result<(), String> {
    let mut s = settings::load();
    s.audio_tool_path = None;
    settings::save(&s).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_theme(theme: String) -> Result<(), String> {
    let mut s = settings::load();
    s.theme = Some(theme);
    settings::save(&s).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_accent(accent: String) -> Result<(), String> {
    let mut s = settings::load();
    s.accent = Some(accent);
    settings::save(&s).map_err(|e| e.to_string())
}

/// Indexes the game data at `path` (heavy; runs off the async runtime) and
/// stores it in app state. Returns the number of indexed unit resources.
#[tauri::command]
pub async fn init_game_resources(app: AppHandle, path: String) -> Result<usize, String> {
    let p = PathBuf::from(&path);
    if !engine::is_valid_game_data_path(&p) {
        return Err("Not a valid Helldivers II data folder".into());
    }
    let resources = tauri::async_runtime::spawn_blocking(move || GameResources::load(&p))
        .await
        .map_err(|e| e.to_string())?;
    let count = resources.unit_count();
    *app.state::<AppState>().resources.lock().unwrap() = Some(Arc::new(resources));
    Ok(count)
}

/// Repatches every patch file under `path`, emitting `batch://progress` events
/// as it goes. Auto-run when a folder is dropped/added on the Home page.
#[tauri::command]
pub async fn process_batch(app: AppHandle, id: String, path: String) -> Result<BatchResult, String> {
    let resources = app
        .state::<AppState>()
        .resources
        .lock()
        .unwrap()
        .clone()
        .ok_or("Game data path is not set. Set it in Settings first.")?;

    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || run_batch(handle, id, path, resources))
        .await
        .map_err(|e| e.to_string())
}

fn emit_progress(app: &AppHandle, p: &BatchProgress) {
    let _ = app.emit("batch://progress", p);
}

fn run_batch(app: AppHandle, id: String, path: String, resources: Arc<GameResources>) -> BatchResult {
    let dir = PathBuf::from(&path);
    let patches = engine::find_patch_files(&dir);
    let patches_found = patches.len();

    let checked = AtomicUsize::new(0);
    let updated = AtomicUsize::new(0);
    let skipped = AtomicUsize::new(0);
    let corrupted = Mutex::new(Vec::<String>::new());

    emit_progress(
        &app,
        &BatchProgress {
            id: id.clone(),
            status: "running".into(),
            patches_found,
            checked: 0,
            updated: 0,
            skipped: 0,
            audio: 0,
            corrupted: vec![],
        },
    );

    patches.par_iter().for_each(|p| {
        // A malformed patch must not take down the GUI; treat a panic as corrupt.
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            engine::update_patch_file(p, &*resources)
        }))
        .unwrap_or(PatchOutcome::Corrupted);

        match outcome {
            PatchOutcome::Updated => {
                updated.fetch_add(1, Ordering::Relaxed);
            }
            PatchOutcome::NoUnits => {
                skipped.fetch_add(1, Ordering::Relaxed);
            }
            PatchOutcome::Corrupted => {
                corrupted.lock().unwrap().push(p.to_string_lossy().into_owned());
            }
        }
        let done = checked.fetch_add(1, Ordering::Relaxed) + 1;
        emit_progress(
            &app,
            &BatchProgress {
                id: id.clone(),
                status: "running".into(),
                patches_found,
                checked: done,
                updated: updated.load(Ordering::Relaxed),
                skipped: skipped.load(Ordering::Relaxed),
                audio: 0,
                corrupted: corrupted.lock().unwrap().clone(),
            },
        );
    });

    // Delegate every directory that directly holds audio patches to the
    // configured external audio tool, mirroring the CLI's second pass.
    let audio_dirs = audio::find_audio_dirs(&dir);
    let mut audio_delegated = 0usize;
    if !audio_dirs.is_empty() {
        let s = settings::load();
        let tool = s.audio_tool_path.map(PathBuf::from).filter(|p| p.is_file());
        match (tool, s.game_data_path) {
            (Some(tool), Some(game_path)) => {
                let game_path = PathBuf::from(game_path);
                for (adir, patches) in &audio_dirs {
                    match audio::repatch_audio_dir(&tool, &game_path, adir, patches) {
                        Ok(_) => audio_delegated += patches.len(),
                        Err(e) => corrupted.lock().unwrap().push(e),
                    }
                }
            }
            _ => {
                let total: usize = audio_dirs.iter().map(|(_, p)| p.len()).sum();
                corrupted.lock().unwrap().push(format!(
                    "{total} audio patch file(s) across {} folder(s) found but no audio tool is \
                     configured; set it in Settings",
                    audio_dirs.len()
                ));
            }
        }
    }

    let corrupted = corrupted.into_inner().unwrap();
    let status = if corrupted.is_empty() { "done" } else { "error" };
    emit_progress(
        &app,
        &BatchProgress {
            id: id.clone(),
            status: status.into(),
            patches_found,
            checked: patches_found,
            updated: updated.load(Ordering::Relaxed),
            skipped: skipped.load(Ordering::Relaxed),
            audio: audio_delegated,
            corrupted: corrupted.clone(),
        },
    );

    BatchResult {
        id,
        path,
        patches_found,
        updated: updated.load(Ordering::Relaxed),
        skipped: skipped.load(Ordering::Relaxed),
        audio: audio_delegated,
        corrupted,
    }
}

/// Returns and clears the patch folder paths seeded at launch (double-click or
/// drag-drop onto the exe). Called once by the frontend on mount.
#[tauri::command]
pub fn take_startup_paths(app: AppHandle) -> Vec<String> {
    std::mem::take(&mut *app.state::<AppState>().startup_paths.lock().unwrap())
}

/// Loads the cached game data path (if valid) into state at startup so batches
/// work immediately. Emits `resources://ready` with the unit count on success.
pub fn preload_cached_resources(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(path) = settings::load().game_data_path else {
            return;
        };
        let p = PathBuf::from(&path);
        if !engine::is_valid_game_data_path(&p) {
            return;
        }
        if let Ok(resources) =
            tauri::async_runtime::spawn_blocking(move || GameResources::load(&p)).await
        {
            let count = resources.unit_count();
            *handle.state::<AppState>().resources.lock().unwrap() = Some(Arc::new(resources));
            let _ = handle.emit("resources://ready", count);
        }
    });
}
