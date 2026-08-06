//! Tests for `classify_patch_file` — the type-table scan that routes audio mods
//! away from the `no_units` bucket. `classify_patch_file` only reads the main
//! header + type table (never the TOC entries or data), so these build minimal
//! synthetic patches consisting of just those two regions.

use std::path::PathBuf;

use engine::{
    classify_patch_file, PatchKind, BINK_VIDEO, TEXT_BANK, UNIT_TYPE_ID, WWISE_BANK, WWISE_DEP,
    WWISE_STREAM,
};

/// Builds a patch file containing only a main header and the given type entries.
/// Layout matches `update_patch_file`: 72-byte main header (num_types at offset
/// 4), then 32-byte type entries (8 pad, type id, count, 8 pad).
fn build(type_ids: &[u64]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0xF000_0011u32.to_le_bytes()); // magic
    out.extend_from_slice(&(type_ids.len() as u32).to_le_bytes()); // num_types @4
    out.extend_from_slice(&0u32.to_le_bytes()); // num_files @8
    out.resize(72, 0); // rest of the main header
    for &t in type_ids {
        out.extend_from_slice(&[0u8; 8]); // padding
        out.extend_from_slice(&t.to_le_bytes()); // type id
        out.extend_from_slice(&1u64.to_le_bytes()); // count
        out.extend_from_slice(&[0u8; 8]); // trailing padding
    }
    out
}

fn tmp_write(bytes: &[u8]) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("hd2-classify-{}-{nanos}.patch", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    path
}

fn classify(type_ids: &[u64]) -> PatchKind {
    let p = tmp_write(&build(type_ids));
    let kind = classify_patch_file(&p);
    let _ = std::fs::remove_file(&p);
    kind
}

#[test]
fn audio_only_is_audio() {
    assert_eq!(classify(&[WWISE_BANK, WWISE_STREAM]), PatchKind::Audio);
    assert_eq!(classify(&[WWISE_DEP]), PatchKind::Audio);
    assert_eq!(classify(&[TEXT_BANK]), PatchKind::Audio);
    assert_eq!(classify(&[BINK_VIDEO]), PatchKind::Audio);
}

#[test]
fn unit_only_is_unit() {
    assert_eq!(classify(&[UNIT_TYPE_ID]), PatchKind::Unit);
}

#[test]
fn mixed_unit_and_audio_is_unit_and_audio() {
    assert_eq!(
        classify(&[UNIT_TYPE_ID, WWISE_BANK]),
        PatchKind::UnitAndAudio
    );
    // order-independent, and even with an unrelated type present
    assert_eq!(
        classify(&[WWISE_STREAM, 0x1234_5678_9ABCu64, UNIT_TYPE_ID]),
        PatchKind::UnitAndAudio
    );
}

#[test]
fn neither_is_other() {
    assert_eq!(classify(&[0x1234_5678_9ABCu64]), PatchKind::Other);
    assert_eq!(classify(&[]), PatchKind::Other);
}

#[test]
fn small_type_id_is_corrupted() {
    // a type id < 2**32 is a structural inconsistency, same as update_patch_file
    assert_eq!(classify(&[0x42]), PatchKind::Corrupted);
}

/// The committed golden fixtures should classify consistently: the `no_units`
/// fixture is `Other` (its non-unit type is not audio), the unit fixtures are
/// `Unit`, and `corrupted_smalltype` is `Corrupted`.
#[test]
fn golden_fixtures_classify_as_expected() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let expect = [
        ("no_units", PatchKind::Other),
        ("unit_grow", PatchKind::Unit),
        ("two_units", PatchKind::Unit),
        ("corrupted_smalltype", PatchKind::Corrupted),
    ];
    for (case, kind) in expect {
        let path = fixtures.join(case).join("input.patch");
        assert_eq!(
            classify_patch_file(&path),
            kind,
            "fixture {case} classified unexpectedly"
        );
    }
}
