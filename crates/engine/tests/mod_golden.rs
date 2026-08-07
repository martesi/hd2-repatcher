//! Differential golden tests for phase 5: `wwise::Mod`'s `import_patch`/
//! `write_patch`/`write_separate_patches`/`add_game_archive`/
//! `load_archive_file` orchestration. Replays fixtures produced by
//! `tools/gen_audio_golden.py`'s `build_phase5_cases` through the Rust port
//! and asserts byte-for-byte identical output against the reference Python
//! engine (`reference/audio_core.py`). Regenerate fixtures with
//! `python tools/gen_audio_golden.py`.

use std::path::{Path, PathBuf};

use engine::wwise::Mod;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Copies every file in `dir` (a fixture's `base/` or `patches/` folder,
/// including `.stream` companions) into `dest`, returning the copied paths
/// for the *primary* (non-`.stream`) files, in the order given by `names`.
fn stage(dir: &Path, dest: &Path, names: &[String]) -> Vec<PathBuf> {
    std::fs::create_dir_all(dest).unwrap();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_file() {
            std::fs::copy(&path, dest.join(path.file_name().unwrap())).unwrap();
        }
    }
    names.iter().map(|n| dest.join(n)).collect()
}

/// Asserts every file in `expected_dir` has a byte-identical counterpart in
/// `got_dir`, and that `got_dir` has no extra files.
fn assert_dirs_match(name: &str, label: &str, got_dir: &Path, expected_dir: &Path) {
    let mut expected_files: Vec<PathBuf> = std::fs::read_dir(expected_dir).unwrap().flatten().map(|e| e.path()).collect();
    expected_files.sort();
    let mut got_files: Vec<PathBuf> = std::fs::read_dir(got_dir).unwrap().flatten().map(|e| e.path()).collect();
    got_files.sort();

    let expected_names: Vec<_> = expected_files.iter().map(|p| p.file_name().unwrap().to_owned()).collect();
    let got_names: Vec<_> = got_files.iter().map(|p| p.file_name().unwrap().to_owned()).collect();
    assert_eq!(got_names, expected_names, "[{name}] {label}: output file set differs from Python oracle");

    for expected_path in &expected_files {
        let file_name = expected_path.file_name().unwrap();
        let got_path = got_dir.join(file_name);
        let expected_bytes = std::fs::read(expected_path).unwrap();
        let got_bytes = std::fs::read(&got_path).unwrap();
        assert!(
            got_bytes == expected_bytes,
            "[{name}] {label}: {} differs from Python oracle (len {} vs {}, first diff at {:?})",
            file_name.to_string_lossy(),
            got_bytes.len(),
            expected_bytes.len(),
            got_bytes.iter().zip(&expected_bytes).position(|(a, b)| a != b)
        );
    }
}

fn run_case(dir: &Path) {
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    let meta: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
    let base_names: Vec<String> = meta["base_names"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let patch_names: Vec<String> =
        meta["patch_names"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();

    let work_dir = std::env::temp_dir().join(format!("hd2-mod-golden-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work_dir);
    let base_paths = stage(&dir.join("base"), &work_dir.join("base"), &base_names);
    let patch_paths = stage(&dir.join("patches"), &work_dir.join("patches"), &patch_names);

    let mut m = Mod::new();
    for path in &base_paths {
        assert!(m.load_archive_file(path), "[{name}] failed to load base archive {}", path.display());
    }
    for path in &patch_paths {
        assert!(m.import_patch(path, true), "[{name}] failed to import patch {}", path.display());
    }

    let combined_dir = work_dir.join("combined");
    std::fs::create_dir_all(&combined_dir).unwrap();
    m.write_patch(&combined_dir, None).expect("write_patch should succeed");
    assert_dirs_match(&name, "write_patch", &combined_dir, &dir.join("expected_combined"));

    let separate_dir = work_dir.join("separate");
    std::fs::create_dir_all(&separate_dir).unwrap();
    m.write_separate_patches(&separate_dir).expect("write_separate_patches should succeed");
    assert_dirs_match(&name, "write_separate_patches", &separate_dir, &dir.join("expected_separate"));

    let _ = std::fs::remove_dir_all(&work_dir);
}

#[test]
fn mod_golden_fixtures_match_python_oracle() {
    let mut cases: Vec<PathBuf> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures dir exists; run `python tools/gen_audio_golden.py`")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().unwrap().to_string_lossy().starts_with("mod_"))
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no mod-orchestration fixtures found");

    for dir in cases {
        run_case(&dir);
    }
}
