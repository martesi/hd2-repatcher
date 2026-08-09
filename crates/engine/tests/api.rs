//! Ports of the pytest cases in `reference/tests/test_update_unit_mods.py` for
//! the pure engine API, plus an end-to-end aggregation check over the committed
//! golden inputs.

use std::path::{Path, PathBuf};

use engine::{
    find_patch_files, game_data_path, is_valid_game_data_path, is_valid_game_root_path,
    process_patch_folder, PatchFileGroup, UnitData, UnitDataSource, LEGACY_MARKER_FILE,
    SLIM_MARKER_FILE, WWISE_STREAM,
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
fn install_root_resolves_and_validates_its_data_folder() {
    let root = tmp();
    let data = game_data_path(&root);
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join(LEGACY_MARKER_FILE), b"").unwrap();

    assert_eq!(game_data_path(&root), root.join("data"));
    assert!(is_valid_game_root_path(&root));
    assert!(is_valid_game_data_path(&data));
}

#[test]
fn install_root_is_invalid_without_a_data_folder() {
    let root = tmp();
    std::fs::write(root.join(LEGACY_MARKER_FILE), b"").unwrap();
    assert!(!is_valid_game_root_path(&root));
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

fn write_patch_with_sidecars(main: &Path, stream_size: u32, gpu_size: u32) {
    let type_end = 72 + 32;
    let header_end = type_end + 80;
    let data_offset = header_end + 8;
    let mut data = vec![0u8; data_offset + 4];
    data[0..4].copy_from_slice(&0xF000_0011u32.to_le_bytes());
    data[4..8].copy_from_slice(&1u32.to_le_bytes());
    data[8..12].copy_from_slice(&1u32.to_le_bytes());
    data[72 + 8..72 + 16].copy_from_slice(&WWISE_STREAM.to_le_bytes());
    data[72 + 16..72 + 24].copy_from_slice(&1u64.to_le_bytes());
    data[type_end..type_end + 8].copy_from_slice(&1u64.to_le_bytes());
    data[type_end + 8..type_end + 16].copy_from_slice(&WWISE_STREAM.to_le_bytes());
    data[type_end + 16..type_end + 24].copy_from_slice(&(data_offset as u64).to_le_bytes());
    data[type_end + 56..type_end + 60].copy_from_slice(&4u32.to_le_bytes());
    data[type_end + 60..type_end + 64].copy_from_slice(&stream_size.to_le_bytes());
    data[type_end + 64..type_end + 68].copy_from_slice(&gpu_size.to_le_bytes());
    std::fs::write(main, data).unwrap();
    if stream_size > 0 {
        std::fs::write(PatchFileGroup::companion_path(main, ".stream"), [1u8, 2, 3, 4]).unwrap();
    }
    if gpu_size > 0 {
        std::fs::write(
            PatchFileGroup::companion_path(main, ".gpu_resources"),
            [5u8, 6, 7, 8],
        )
        .unwrap();
    }
}

#[test]
fn patch_group_resolves_main_and_both_companions_to_one_group() {
    let d = tmp();
    let main = d.join("chosen.patch_7");
    write_patch_with_sidecars(&main, 4, 4);

    let from_main = PatchFileGroup::resolve(&main).unwrap();
    let from_stream = PatchFileGroup::resolve(&main.with_file_name("chosen.patch_7.stream")).unwrap();
    let from_gpu = PatchFileGroup::resolve(&main.with_file_name("chosen.patch_7.gpu_resources")).unwrap();

    assert_eq!(from_main, from_stream);
    assert_eq!(from_main, from_gpu);
    assert_eq!(from_main.main, main);
    assert!(from_main.stream.is_some());
    assert!(from_main.gpu_resources.is_some());
}

#[test]
fn patch_group_reports_missing_required_companion_before_mutation() {
    let d = tmp();
    let main = d.join("missing.patch_0");
    write_patch_with_sidecars(&main, 4, 0);
    std::fs::remove_file(PatchFileGroup::companion_path(&main, ".stream")).unwrap();

    let error = PatchFileGroup::resolve(&main).unwrap_err().to_string();
    assert!(error.contains("missing required companion"), "{error}");
    assert!(main.is_file());
}

#[test]
fn patch_file_discovery_excludes_companions() {
    let d = tmp();
    let main = d.join("only.patch_0");
    std::fs::write(&main, []).unwrap();
    std::fs::write(PatchFileGroup::companion_path(&main, ".stream"), []).unwrap();
    std::fs::write(PatchFileGroup::companion_path(&main, ".gpu_resources"), []).unwrap();

    assert_eq!(find_patch_files(&d), vec![main]);
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
