//! Pure-Rust tests (no Python oracle needed) for phase 7:
//! `process_audio_patches`'s refusal paths. The first mirrors real upstream
//! `run_patch_cli`'s `"Unable to locate base archive for soundbank ..."`
//! early return (`hd2-audio-modder/audio_modder.py:3712-3721`); the other
//! two are this port's own, and ensure an audio run that replaces nothing
//! cannot stage or commit a resource-less patch over the selected group.
//! All three reuse existing golden-fixture archives rather than hand-building
//! ones.

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

/// Copies every file in `src` into a fresh temp dir tagged `tag`.
fn stage(src: &Path, tag: &str) -> PathBuf {
    let dir = tmp_dir(tag);
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let path = entry.path();
        if path.is_file() {
            std::fs::copy(&path, dir.join(path.file_name().unwrap())).unwrap();
        }
    }
    dir
}

#[test]
fn process_audio_patches_fails_when_an_indexed_base_archive_cannot_be_loaded() {
    let case = fixtures_dir().join("process_audio_patches_single_archive_and_text_bank");
    let gamedata_dir = stage(&case.join("gamedata"), "gamedata-vanished");

    // Index the archive first, then delete it: the same state a user reaches
    // by moving/verifying game files between scans, and the only way
    // `load_base_archive` returns false with a name `AudioIndex` resolved.
    let resources = GameResources::load(&gamedata_dir);
    std::fs::remove_file(gamedata_dir.join("1111111111111111")).unwrap();

    let patches_dir = stage(&case.join("patches"), "patches-vanished");
    let patch = patches_dir.join("mod_patch_orchestration.patch_0");

    let err = process_audio_patches(&patches_dir, &[patch], &resources)
        .expect_err("a base archive that disappeared between scan and load must fail the run");
    assert!(err.contains("failed to load base game archive"), "unexpected error message: {err}");

    let _ = std::fs::remove_dir_all(&gamedata_dir);
    let _ = std::fs::remove_dir_all(&patches_dir);
}

#[test]
fn process_audio_patches_fails_when_nothing_was_replaced() {
    let case = fixtures_dir().join("process_audio_patches_multi_archive");
    let gamedata_dir = stage(&case.join("gamedata"), "gamedata-noop");
    let resources = GameResources::load(&gamedata_dir);

    // A "mod" that is byte-identical to the base archive it targets: every
    // resource resolves, nothing changes, and `write_patch` would emit a
    // resource-less archive over the input's own name.
    let patches_dir = tmp_dir("patches-noop");
    let patch = patches_dir.join("mod_patch_identical.patch_0");
    std::fs::copy(case.join("gamedata/aaaaaaaaaaaaaaaa"), &patch).unwrap();

    let err = process_audio_patches(&patches_dir, &[patch], &resources)
        .expect_err("a patch that replaces no audio must fail rather than write an empty patch");
    assert!(err.contains("no audio was replaced"), "unexpected error message: {err}");
    assert!(
        !patches_dir.join("9ba626afa44a3aa3.patch_0").exists(),
        "the empty patch must not have been written"
    );

    let _ = std::fs::remove_dir_all(&gamedata_dir);
    let _ = std::fs::remove_dir_all(&patches_dir);
}
