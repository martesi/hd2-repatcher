//! Differential golden tests for phase 7: `engine::process_audio_patches`,
//! the orchestration entry point wiring `AudioIndex`-based archive
//! resolution (phase 6) into the `Mod::import_patch`/`write_patch` control
//! flow (phase 5). Replays fixtures produced by
//! `tools/gen_audio_golden.py`'s `build_phase7_cases` and asserts
//! byte-for-byte identical output against the reference Python engine
//! (`reference/audio_core.py`, driven directly rather than through
//! `run_patch_cli` since that needs a friendlynames db this port doesn't
//! have — see the fixture generator's doc comment). Regenerate fixtures
//! with `python tools/gen_audio_golden.py`.

use std::path::{Path, PathBuf};

use engine::{process_audio_patches, GameResources};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Copies every file in `dir` into a freshly created `dest`.
fn stage_dir(dir: &Path, dest: &Path) {
    std::fs::create_dir_all(dest).unwrap();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_file() {
            std::fs::copy(&path, dest.join(path.file_name().unwrap())).unwrap();
        }
    }
}

fn run_case(dir: &Path) {
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    let meta: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
    let patch_names: Vec<String> =
        meta["patch_names"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();

    let work_dir = std::env::temp_dir().join(format!("hd2-audio-orchestration-golden-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work_dir);

    let gamedata_dir = work_dir.join("gamedata");
    stage_dir(&dir.join("gamedata"), &gamedata_dir);

    let patches_dir = work_dir.join("patches");
    stage_dir(&dir.join("patches"), &patches_dir);
    let patch_paths: Vec<PathBuf> = patch_names.iter().map(|n| patches_dir.join(n)).collect();

    let resources = GameResources::load(&gamedata_dir);
    process_audio_patches(&patches_dir, &patch_paths, &resources)
        .unwrap_or_else(|e| panic!("[{name}] process_audio_patches failed: {e}"));

    let got_path = patches_dir.join("9ba626afa44a3aa3.patch_0");
    let expected_path = dir.join("expected/9ba626afa44a3aa3.patch_0");
    let got = std::fs::read(&got_path).unwrap_or_else(|e| panic!("[{name}] failed to read {}: {e}", got_path.display()));
    let expected = std::fs::read(&expected_path).unwrap();
    assert!(
        got == expected,
        "[{name}] combined patch differs from Python oracle (len {} vs {}, first diff at {:?})",
        got.len(),
        expected.len(),
        got.iter().zip(&expected).position(|(a, b)| a != b)
    );

    let _ = std::fs::remove_dir_all(&work_dir);
}

#[test]
fn process_audio_patches_golden_fixtures() {
    let base = fixtures_dir();
    for case in ["process_audio_patches_single_archive_and_text_bank", "process_audio_patches_multi_archive"] {
        run_case(&base.join(case));
    }
}
