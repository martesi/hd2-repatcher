//! Differential golden tests: replay fixtures produced by the reference Python
//! engine (`tools/gen_golden.py`) through the Rust port and assert byte-for-byte
//! identical output and matching outcome. Regenerate fixtures with
//! `python tools/gen_golden.py`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use engine::{update_patch_file, PatchOutcome, UnitData, UnitDataSource};

struct MockSource {
    units: HashMap<u64, (Vec<u8>, Vec<u8>)>, // id -> (version, lod_group)
}

impl UnitDataSource for MockSource {
    fn contains(&self, unit_id: u64) -> bool {
        self.units.contains_key(&unit_id)
    }

    fn get_unit_data(&self, unit_id: u64) -> UnitData {
        let (version, lod) = &self.units[&unit_id];
        let mut v = [0u8; 4];
        v.copy_from_slice(&version[..4]);
        UnitData {
            version: v,
            lod_group_data: lod.clone(),
            lod_group_size: lod.len() as i64,
        }
    }
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn load_units(json: &str) -> HashMap<u64, (Vec<u8>, Vec<u8>)> {
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    let mut units = HashMap::new();
    if let Some(map) = value.get("units").and_then(|u| u.as_object()) {
        for (id, entry) in map {
            let id: u64 = id.parse().unwrap();
            let version = hex_decode(entry["version"].as_str().unwrap());
            let lod = hex_decode(entry["lod_group"].as_str().unwrap());
            units.insert(id, (version, lod));
        }
    }
    units
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn run_case(dir: &Path) {
    let name = dir.file_name().unwrap().to_string_lossy();
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
    let expected_outcome = meta["outcome"].as_str().unwrap();

    let units = load_units(&std::fs::read_to_string(dir.join("resources.json")).unwrap());
    let source = MockSource { units };

    // work on a private copy so the fixture stays pristine
    let input = std::fs::read(dir.join("input.patch")).unwrap();
    let work = std::env::temp_dir().join(format!("hd2-golden-{name}-{}.patch", std::process::id()));
    std::fs::write(&work, &input).unwrap();

    let outcome = update_patch_file(&work, &source);
    let outcome_str = match outcome {
        PatchOutcome::Updated => "updated",
        PatchOutcome::NoUnits => "no_units",
        PatchOutcome::Corrupted => "corrupted",
    };
    assert_eq!(
        outcome_str, expected_outcome,
        "[{name}] outcome mismatch"
    );

    if expected_outcome == "updated" {
        let got = std::fs::read(&work).unwrap();
        let expected = std::fs::read(dir.join("expected.patch")).unwrap();
        assert_eq!(
            got.len(),
            expected.len(),
            "[{name}] output length {} != expected {}",
            got.len(),
            expected.len()
        );
        assert!(
            got == expected,
            "[{name}] output bytes differ from Python oracle (first diff at {:?})",
            got.iter().zip(&expected).position(|(a, b)| a != b)
        );
    }

    let _ = std::fs::remove_file(&work);
}

#[test]
fn golden_fixtures_match_python_oracle() {
    let mut cases: Vec<PathBuf> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures dir exists; run `python tools/gen_golden.py`")
        .flatten()
        .map(|e| e.path())
        // `audio_*`/`hirc_merge_*` fixtures share this directory but belong
        // to `audio_golden.rs`/`hirc_merge_golden.rs` respectively (see
        // `tools/gen_audio_golden.py`).
        .filter(|p| {
            p.is_dir()
                && !p.file_name().unwrap().to_string_lossy().starts_with("audio_")
                && !p.file_name().unwrap().to_string_lossy().starts_with("hirc_merge_")
        })
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no fixtures found");

    for dir in cases {
        run_case(&dir);
    }
}
