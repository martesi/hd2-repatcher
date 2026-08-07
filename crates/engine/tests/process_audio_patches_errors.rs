//! Pure-Rust test (no Python oracle needed) for phase 7:
//! `process_audio_patches`'s "soundbank has no indexed base archive" error
//! path, matching real upstream `run_patch_cli`'s
//! `"Unable to locate base archive for soundbank ..."` early return
//! (`hd2-audio-modder/audio_modder.py:3712-3721`). Reuses an existing
//! golden-fixture patch file (a real WWISE_BANK-carrying `.patch_0`) rather
//! than hand-building one, pointed at an empty game data folder so its bank
//! id can never resolve.

use std::path::{Path, PathBuf};

use engine::{process_audio_patches, GameResources, LEGACY_MARKER_FILE};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn tmp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("hd2-audio-orchestration-err-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn process_audio_patches_fails_when_soundbank_archive_is_unresolvable() {
    let gamedata_dir = tmp_dir("gamedata");
    std::fs::write(gamedata_dir.join(LEGACY_MARKER_FILE), []).unwrap();
    let resources = GameResources::load(&gamedata_dir);

    let patches_dir = tmp_dir("patches");
    let patch_src = fixtures_dir()
        .join("process_audio_patches_single_archive_and_text_bank/patches/mod_patch_orchestration.patch_0");
    let patch_dst = patches_dir.join("mod_patch_orchestration.patch_0");
    std::fs::copy(&patch_src, &patch_dst).unwrap();

    let err = process_audio_patches(&patches_dir, &[patch_dst], &resources)
        .expect_err("a patch touching an unindexed soundbank must fail, not silently drop it");
    assert!(
        err.contains("unable to locate base archive for soundbank"),
        "unexpected error message: {err}"
    );

    let _ = std::fs::remove_dir_all(&gamedata_dir);
    let _ = std::fs::remove_dir_all(&patches_dir);
}
