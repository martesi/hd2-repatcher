//! Differential golden tests for `WwiseHierarchy::import_hierarchy` (phase
//! 3's field-merge for `Sound`/`MusicTrack`/`MusicSegment`/
//! `RandomSequenceContainer`). Tested directly at the hierarchy level (raw
//! HIRC bytes in, raw HIRC bytes out) rather than through a full
//! `GameArchive`/`Mod`, since `Mod::import_patch` orchestration is ported in
//! a later phase — see `tools/gen_audio_golden.py::write_hierarchy_merge_case`.
//! Regenerate fixtures with `python tools/gen_audio_golden.py`.

use std::path::{Path, PathBuf};

use engine::wwise::hierarchy::{BankVersion, WwiseHierarchy};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn run_case(dir: &Path) {
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    let meta: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
    let version = match meta["version"].as_u64().unwrap() {
        154 => BankVersion::V154,
        140 => BankVersion::V140,
        v => panic!("[{name}] unexpected bank version {v}"),
    };

    let base_bytes = std::fs::read(dir.join("base.bin")).unwrap();
    let patch_bytes = std::fs::read(dir.join("patch.bin")).unwrap();
    let expected = std::fs::read(dir.join("expected.bin")).unwrap();

    let mut base = WwiseHierarchy::load(version, &base_bytes);
    let patch = WwiseHierarchy::load(version, &patch_bytes);

    base.import_hierarchy(&patch);
    let got = base.get_data();

    assert!(
        got == expected,
        "[{name}] merged output bytes differ from Python oracle (len {} vs {}, first diff at {:?})",
        got.len(),
        expected.len(),
        got.iter().zip(&expected).position(|(a, b)| a != b)
    );
}

#[test]
fn hirc_merge_fixtures_match_python_oracle() {
    let mut cases: Vec<PathBuf> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures dir exists; run `python tools/gen_audio_golden.py`")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().unwrap().to_string_lossy().starts_with("hirc_merge_"))
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no hirc_merge fixtures found");

    for dir in cases {
        run_case(&dir);
    }
}
