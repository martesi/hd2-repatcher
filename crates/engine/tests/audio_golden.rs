//! Differential golden tests for the audio-patching port (phases 1-2:
//! TOC/archive skeleton with opaque HIRC, plus structured `Sound`/`BaseParam`
//! and DIDX/DATA regeneration). Replays fixtures produced by
//! `tools/gen_audio_golden.py` through the Rust port and asserts byte-for-byte
//! identical output against the reference Python engine
//! (`reference/audio_core.py`). Regenerate fixtures with
//! `python tools/gen_audio_golden.py`.

use std::path::{Path, PathBuf};

use engine::wwise::GameArchive;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn run_case(dir: &Path) {
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    let meta: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
    let archive_name = meta["archive_name"].as_str().unwrap();

    // Work on a private copy so the fixture stays pristine, mirroring how
    // `GameArchive::from_patch_file` expects a real file on disk.
    let work_dir = std::env::temp_dir().join(format!("hd2-audio-golden-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&work_dir).unwrap();
    let input_path = work_dir.join(archive_name);
    std::fs::copy(dir.join("input.bin"), &input_path).unwrap();
    let stream_src = dir.join("input.bin.stream");
    if stream_src.exists() {
        std::fs::copy(&stream_src, format!("{}.stream", input_path.display())).unwrap();
    }

    let mut archive = GameArchive::from_patch_file(&input_path).expect("fixture archive should load");

    if let Some(mutation) = meta.get("mutation").filter(|m| !m.is_null() && m.as_object().is_some_and(|o| !o.is_empty())) {
        let new_data = hex_decode(mutation["new_data_hex"].as_str().unwrap());
        if let Some(stream_id) = mutation.get("stream_id").and_then(|v| v.as_u64()) {
            archive
                .wwise_streams
                .get_mut(&stream_id)
                .expect("mutation targets a stream present in the fixture")
                .audio_source
                .set_data(new_data, true);
        } else if let Some(short_id) = mutation.get("short_id").and_then(|v| v.as_u64()) {
            archive
                .audio_sources
                .get_mut(&short_id)
                .expect("mutation targets an audio source present in the fixture")
                .set_data(new_data, true);
        } else {
            panic!("[{name}] mutation object has neither stream_id nor short_id");
        }
    }

    let output_dir = std::env::temp_dir().join(format!("hd2-audio-golden-out-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&output_dir);
    std::fs::create_dir_all(&output_dir).unwrap();
    archive.to_file(&output_dir).expect("to_file should succeed");

    let got = std::fs::read(output_dir.join(archive_name)).unwrap();
    let expected = std::fs::read(dir.join("expected.bin")).unwrap();
    assert!(
        got == expected,
        "[{name}] output bytes differ from Python oracle (len {} vs {}, first diff at {:?})",
        got.len(),
        expected.len(),
        got.iter().zip(&expected).position(|(a, b)| a != b)
    );

    let got_stream_path = output_dir.join(format!("{archive_name}.stream"));
    let expected_stream_path = dir.join("expected.bin.stream");
    match (got_stream_path.exists(), expected_stream_path.exists()) {
        (true, true) => {
            let got_s = std::fs::read(&got_stream_path).unwrap();
            let expected_s = std::fs::read(&expected_stream_path).unwrap();
            assert!(got_s == expected_s, "[{name}] .stream output bytes differ from Python oracle");
        }
        (false, false) => {}
        (got_exists, expected_exists) => panic!(
            "[{name}] .stream presence mismatch: got {got_exists}, expected {expected_exists}"
        ),
    }

    let _ = std::fs::remove_dir_all(&work_dir);
    let _ = std::fs::remove_dir_all(&output_dir);
}

#[test]
fn audio_golden_fixtures_match_python_oracle() {
    let mut cases: Vec<PathBuf> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures dir exists; run `python tools/gen_audio_golden.py`")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().unwrap().to_string_lossy().starts_with("audio_"))
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no audio fixtures found");

    for dir in cases {
        run_case(&dir);
    }
}
