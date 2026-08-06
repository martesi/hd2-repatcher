//! Ports of the pytest cases in `reference/tests/test_update_unit_mods.py` for
//! the pure engine API, plus an end-to-end aggregation check over the committed
//! golden inputs.

use std::path::{Path, PathBuf};

use engine::{
    find_patch_files, is_valid_game_data_path, process_patch_folder, UnitData, UnitDataSource,
    LEGACY_MARKER_FILE, SLIM_MARKER_FILE,
};

/// A source that knows about no units — every unit header is treated as unknown.
struct EmptySource;
impl UnitDataSource for EmptySource {
    fn contains(&self, _unit_id: u64) -> bool {
        false
    }
    fn get_unit_data(&self, _unit_id: u64) -> UnitData {
        unreachable!()
    }
}

fn tmp() -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "hd2-api-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rand_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

#[test]
fn is_valid_false_when_directory_missing() {
    let d = tmp();
    assert!(!is_valid_game_data_path(&d.join("missing")));
}

#[test]
fn is_valid_false_when_neither_marker_present() {
    let d = tmp();
    assert!(!is_valid_game_data_path(&d));
}

#[test]
fn is_valid_true_when_legacy_marker_present() {
    let d = tmp();
    std::fs::write(d.join(LEGACY_MARKER_FILE), b"").unwrap();
    assert!(is_valid_game_data_path(&d));
}

#[test]
fn is_valid_true_when_slim_marker_present() {
    let d = tmp();
    std::fs::write(d.join(SLIM_MARKER_FILE), b"").unwrap();
    assert!(is_valid_game_data_path(&d));
}

#[test]
fn find_patch_files_recursively() {
    let d = tmp();
    std::fs::write(d.join("a.patch_0"), b"").unwrap();
    std::fs::create_dir_all(d.join("sub")).unwrap();
    std::fs::write(d.join("sub/b.patch_1"), b"").unwrap();
    std::fs::write(d.join("c.txt"), b"").unwrap();

    let mut names: Vec<String> = find_patch_files(&d)
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["a.patch_0", "b.patch_1"]);
}

#[test]
fn find_patch_files_empty_when_none() {
    let d = tmp();
    std::fs::write(d.join("c.txt"), b"").unwrap();
    assert!(find_patch_files(&d).is_empty());
}

/// Copies the three golden inputs that cover each outcome into one folder and
/// checks they are discovered and bucketed correctly. With an empty source the
/// `unit_grow` patch still "updates" (its unknown unit header is dropped).
#[test]
fn process_folder_aggregates_by_outcome() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let d = tmp();
    for (case, dest) in [
        ("unit_grow", "u.patch"),
        ("no_units", "n.patch"),
        ("corrupted_smalltype", "c.patch"),
    ] {
        let bytes = std::fs::read(fixtures.join(case).join("input.patch")).unwrap();
        std::fs::write(d.join(dest), bytes).unwrap();
    }

    let result = process_patch_folder(&d, &EmptySource);
    assert_eq!(result.patches_found, 3);
    assert_eq!(result.updated.len(), 1, "updated: {:?}", result.updated);
    assert_eq!(result.no_units.len(), 1, "no_units: {:?}", result.no_units);
    assert_eq!(
        result.corrupted_files.len(),
        1,
        "corrupted: {:?}",
        result.corrupted_files
    );
}
