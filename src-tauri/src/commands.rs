//! Tauri command handlers and shared app state.

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use engine::{settings, GameResources, PatchKind};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::patching::{self, GroupOutcome};

/// The indexed game data, shared read-only across patch operations.
#[derive(Default)]
pub struct AppState {
    pub resources: Mutex<Option<Arc<GameResources>>>,
    /// Patch paths passed on argv at launch, consumed once by the frontend.
    pub startup_paths: Mutex<Vec<String>>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub game_root_path: Option<String>,
    pub game_root_valid: bool,
    pub language: Option<String>,
    pub theme: Option<String>,
    pub accent: Option<String>,
    pub unit_count: usize,
    /// False while cached data is indexing or no valid index is loaded.
    pub resources_ready: bool,
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

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GameDataPatchResult {
    pub main: String,
    pub kind: String,
}

fn current_unit_count(app: &AppHandle) -> usize {
    app.state::<AppState>()
        .resources
        .lock()
        .unwrap()
        .as_ref()
        .map(|resources| resources.unit_count())
        .unwrap_or(0)
}

#[tauri::command]
pub fn get_config(app: AppHandle) -> Config {
    let settings = settings::load();
    let language = settings.language.filter(|language| language == "en");
    let game_root_valid = settings
        .game_root
        .as_ref()
        .map(|path| engine::is_valid_game_root_path(Path::new(path)))
        .unwrap_or(false);
    let resources_ready = app
        .state::<AppState>()
        .resources
        .lock()
        .unwrap()
        .is_some();
    Config {
        game_root_path: settings.game_root,
        game_root_valid,
        language,
        theme: settings.theme,
        accent: settings.accent,
        unit_count: current_unit_count(&app),
        resources_ready,
    }
}

/// Validates and persists the install root. Changing it invalidates the
/// current index before the frontend starts a new indexing operation.
#[tauri::command]
pub fn set_game_root(app: AppHandle, path: String) -> Result<bool, String> {
    let valid = engine::is_valid_game_root_path(Path::new(&path));
    let mut settings = settings::load();
    settings.game_root = Some(path);
    settings::save(&settings).map_err(|error| error.to_string())?;
    *app.state::<AppState>().resources.lock().unwrap() = None;
    Ok(valid)
}

#[tauri::command]
pub fn set_language(language: String) -> Result<(), String> {
    if language != "en" {
        return Err(format!("Unsupported language: {language}"));
    }
    let mut settings = settings::load();
    settings.language = Some(language);
    settings::save(&settings).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_theme(theme: String) -> Result<(), String> {
    let mut settings = settings::load();
    settings.theme = Some(theme);
    settings::save(&settings).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_accent(accent: String) -> Result<(), String> {
    let mut settings = settings::load();
    settings.accent = Some(accent);
    settings::save(&settings).map_err(|error| error.to_string())
}

/// Indexes the derived game data at `game_root` off the async runtime.
#[tauri::command]
pub async fn init_game_resources(app: AppHandle, game_root: String) -> Result<usize, String> {
    let root = PathBuf::from(&game_root);
    if !engine::is_valid_game_root_path(&root) {
        return Err("Not a valid Helldivers II install root".into());
    }
    *app.state::<AppState>().resources.lock().unwrap() = None;
    let data = engine::game_data_path(&root);
    let resources = tauri::async_runtime::spawn_blocking(move || GameResources::load(&data))
        .await
        .map_err(|error| error.to_string())?;
    // Do not install an index that finished after the user selected a new
    // install root.
    let configured = settings::load().game_root;
    if configured.as_deref() != Some(&game_root) {
        return Err("Game install root changed while indexing".into());
    }
    let count = resources.unit_count();
    *app.state::<AppState>().resources.lock().unwrap() = Some(Arc::new(resources));
    Ok(count)
}

fn patch_resources(app: &AppHandle) -> Result<Arc<GameResources>, String> {
    let settings = settings::load();
    let root = settings
        .game_root
        .ok_or("Game install root is not set. Set it in Settings first.")?;
    if !engine::is_valid_game_root_path(Path::new(&root)) {
        return Err("Configured Helldivers II install root is invalid".into());
    }
    app.state::<AppState>()
        .resources
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "Game resources are still indexing. Please wait and try again.".into())
}

/// Repatches one selected patch group or every group under a selected folder.
#[tauri::command]
pub async fn process_batch(app: AppHandle, id: String, path: String) -> Result<BatchResult, String> {
    let resources = patch_resources(&app)?;
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || run_batch(handle, id, path, resources))
        .await
        .map_err(|error| error.to_string())
}

fn emit_progress(app: &AppHandle, progress: &BatchProgress) {
    let _ = app.emit("batch://progress", progress);
}

fn run_batch(app: AppHandle, id: String, path: String, resources: Arc<GameResources>) -> BatchResult {
    let input = PathBuf::from(&path);
    let candidates = if input.is_dir() {
        engine::find_patch_files(&input)
    } else {
        vec![input.clone()]
    };
    let patches_found = candidates.len();
    let mut checked = 0usize;
    let mut updated = 0usize;
    let mut skipped = 0usize;
    let mut audio = 0usize;
    let mut corrupted = Vec::<String>::new();

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

    for candidate in candidates {
        let group = match engine::PatchFileGroup::resolve(&candidate) {
            Ok(group) => group,
            Err(error) => {
                corrupted.push(format!("{}: {error}", candidate.display()));
                checked += 1;
                emit_batch_progress(
                    &app,
                    &id,
                    patches_found,
                    checked,
                    updated,
                    skipped,
                    audio,
                    &corrupted,
                );
                continue;
            }
        };
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            patching::process_patch_group(&group, &resources)
        }))
        .unwrap_or_else(|_| Err("patching panicked while processing a malformed group".into()));

        match outcome {
            Ok(GroupOutcome::Updated(kind)) => {
                if matches!(kind, PatchKind::Unit | PatchKind::UnitAndAudio) {
                    updated += 1;
                }
                if matches!(kind, PatchKind::Audio | PatchKind::UnitAndAudio) {
                    audio += 1;
                }
            }
            Ok(GroupOutcome::Skipped(_)) => skipped += 1,
            Err(error) => corrupted.push(format!("{}: {error}", group.main.display())),
        }
        checked += 1;
        emit_batch_progress(
            &app,
            &id,
            patches_found,
            checked,
            updated,
            skipped,
            audio,
            &corrupted,
        );
    }

    let status = if corrupted.is_empty() { "done" } else { "error" };
    emit_progress(
        &app,
        &BatchProgress {
            id: id.clone(),
            status: status.into(),
            patches_found,
            checked,
            updated,
            skipped,
            audio,
            corrupted: corrupted.clone(),
        },
    );

    BatchResult {
        id,
        path,
        patches_found,
        updated,
        skipped,
        audio,
        corrupted,
    }
}

fn emit_batch_progress(
    app: &AppHandle,
    id: &str,
    patches_found: usize,
    checked: usize,
    updated: usize,
    skipped: usize,
    audio: usize,
    corrupted: &[String],
) {
    emit_progress(
        app,
        &BatchProgress {
            id: id.to_owned(),
            status: "running".into(),
            patches_found,
            checked,
            updated,
            skipped,
            audio,
            corrupted: corrupted.to_vec(),
        },
    );
}

/// Patches exactly one group selected from the configured game `data` folder.
/// Mod-manager users should patch the mod source through their manager instead.
#[tauri::command]
pub async fn patch_game_data(
    app: AppHandle,
    path: String,
) -> Result<GameDataPatchResult, String> {
    let resources = patch_resources(&app)?;
    let settings = settings::load();
    let root = settings
        .game_root
        .ok_or("Game install root is not set. Set it in Settings first.")?;
    let data = engine::game_data_path(Path::new(&root));
    let group = engine::PatchFileGroup::resolve(Path::new(&path)).map_err(|error| error.to_string())?;
    let data_canonical = std::fs::canonicalize(&data)
        .map_err(|error| format!("failed to resolve configured game data directory: {error}"))?;
    let main_canonical = std::fs::canonicalize(&group.main)
        .map_err(|error| format!("failed to resolve selected patch group: {error}"))?;
    if main_canonical.parent() != Some(data_canonical.as_path()) {
        return Err(format!(
            "selected patch group must be directly inside the configured game data directory ({})",
            data.display()
        ));
    }

    let kind = engine::classify_patch_file(&group.main);
    if kind == PatchKind::Corrupted {
        return Err(format!("patch '{}' is malformed", group.main.display()));
    }
    let main = group.main.to_string_lossy().into_owned();
    let result = tauri::async_runtime::spawn_blocking(move || {
        patching::process_patch_group(&group, &resources)
    })
    .await
    .map_err(|error| error.to_string())??;
    if matches!(result, GroupOutcome::Skipped(_)) {
        return Err(format!("selected patch '{}' has no unit or audio resources", main));
    }
    Ok(GameDataPatchResult {
        main,
        kind: format_patch_kind(kind),
    })
}

fn format_patch_kind(kind: PatchKind) -> String {
    match kind {
        PatchKind::Unit => "unit",
        PatchKind::Audio => "audio",
        PatchKind::UnitAndAudio => "unit-and-audio",
        PatchKind::Other => "other",
        PatchKind::Corrupted => "corrupted",
    }
    .into()
}

/// Returns and clears paths seeded at launch (double-click or drag-drop onto
/// the executable).
#[tauri::command]
pub fn take_startup_paths(app: AppHandle) -> Vec<String> {
    std::mem::take(&mut *app.state::<AppState>().startup_paths.lock().unwrap())
}

/// Loads the cached install root at startup so patch controls become available
/// only after indexing has completed.
pub fn preload_cached_resources(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(path) = settings::load().game_root else {
            return;
        };
        let root = PathBuf::from(&path);
        if !engine::is_valid_game_root_path(&root) {
            return;
        }
        let data = engine::game_data_path(&root);
        if let Ok(resources) = tauri::async_runtime::spawn_blocking(move || GameResources::load(&data)).await {
            let count = resources.unit_count();
            *handle.state::<AppState>().resources.lock().unwrap() = Some(Arc::new(resources));
            let _ = handle.emit("resources://ready", count);
        }
    });
}
