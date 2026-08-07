//! Pure-Rust tests (no Python oracle needed) for phase 6: `AudioIndex`
//! map-building alongside `GameResources`'s unit index in a single TOC scan,
//! and `wwise::Mod::load_base_archive`'s `Slim`-backed loading of a genuine
//! (here: legacy-layout) base game archive by the name `AudioIndex` resolves.
//! Follows `classify.rs`'s style of hand-building minimal synthetic files
//! rather than relying on `tools/gen_audio_golden.py` fixtures, since this is
//! new plumbing with no Python-side equivalent to diff against (real upstream
//! resolves the same soundbank -> archive mapping via an external
//! friendlynames db this port doesn't have).

use std::path::{Path, PathBuf};

use engine::wwise::archive::TocHeader;
use engine::wwise::{Mod, BANK_VERSION_KEY};
use engine::{GameResources, UnitDataSource, LEGACY_MARKER_FILE, UNIT_TYPE_ID, WWISE_BANK};

fn tmp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("hd2-audio-index-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Builds a minimal legacy-layout package: a 72-byte main header (no type
/// table) followed by one 80-byte `TocHeader` per row, each backed by
/// whatever bytes `contents` provides at that row's own `toc_data_offset`
/// (placed contiguously right after the header table, in `rows` order).
fn build_legacy_package(rows: &[(u64, u64, Vec<u8>)]) -> Vec<u8> {
    // (file_id, type_id, toc_data)
    let header_table_end = 72 + 80 * rows.len();
    let mut offset = header_table_end as u64;
    let mut headers = Vec::new();
    let mut bodies = Vec::new();
    for (file_id, type_id, data) in rows {
        headers.push(TocHeader {
            file_id: *file_id,
            type_id: *type_id,
            toc_data_offset: offset,
            toc_data_size: data.len() as u32,
            ..Default::default()
        });
        bodies.extend_from_slice(data);
        offset += data.len() as u64;
    }

    let mut out = vec![0u8; 72];
    out[0..4].copy_from_slice(&0xF000_0011u32.to_le_bytes()); // magic (num_types stays 0)
    out[8..12].copy_from_slice(&(rows.len() as u32).to_le_bytes()); // num_files
    for header in &headers {
        out.extend_from_slice(&header.get_data());
    }
    out.extend_from_slice(&bodies);
    out
}

/// A minimal but valid `WWISE_BANK` TOC entry body: the 16-byte wrapper
/// `GameArchive::load` skips, then a bank blob containing only a `BKHD`
/// chunk (empty hierarchy, no DIDX/DATA) — enough for `BankParser`/
/// `GameArchive::load` to parse into a real (if content-free) `WwiseBank`.
fn build_bank_toc_data() -> Vec<u8> {
    let wrapper = vec![0u8; 16];
    let mut bkhd_payload = BANK_VERSION_KEY.to_le_bytes().to_vec(); // xor -> 0, so BankVersion::V140
    bkhd_payload.extend_from_slice(&[0u8; 4]);
    let mut bank_data = Vec::new();
    bank_data.extend_from_slice(b"BKHD");
    bank_data.extend_from_slice(&(bkhd_payload.len() as u32).to_le_bytes());
    bank_data.extend_from_slice(&bkhd_payload);

    let mut out = wrapper;
    out.extend_from_slice(&bank_data);
    out
}

fn write_legacy_install(dir: &Path) {
    std::fs::write(dir.join(LEGACY_MARKER_FILE), []).unwrap();
}

#[test]
fn audio_index_and_unit_map_are_built_from_the_same_scan() {
    let dir = tmp_dir("scan");
    write_legacy_install(&dir);

    let unit_id = 0x1111_2222_3333_4444u64;
    let bank_id = 0x5555_6666_7777_8888u64;
    let pkg_name = "onepkg";
    let rows = vec![
        (unit_id, UNIT_TYPE_ID, vec![0u8; 4]),
        (bank_id, WWISE_BANK, build_bank_toc_data()),
    ];
    std::fs::write(dir.join(pkg_name), build_legacy_package(&rows)).unwrap();

    let resources = GameResources::load(&dir);
    assert!(resources.contains(unit_id), "unit id should be indexed alongside the bank id");
    assert_eq!(resources.audio_index().bank_count(), 1);
    assert_eq!(resources.audio_index().archive_name(bank_id), Some(pkg_name));
    assert_eq!(resources.audio_index().archive_name(unit_id), None, "a unit id must not show up as a bank id");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn audio_index_resolves_the_right_archive_across_multiple_packages() {
    let dir = tmp_dir("multi");
    write_legacy_install(&dir);

    let bank_a = 0xAAAA_1111_1111_1111u64;
    let bank_b = 0xBBBB_2222_2222_2222u64;
    std::fs::write(dir.join("pkg_a"), build_legacy_package(&[(bank_a, WWISE_BANK, build_bank_toc_data())])).unwrap();
    std::fs::write(dir.join("pkg_b"), build_legacy_package(&[(bank_b, WWISE_BANK, build_bank_toc_data())])).unwrap();

    let resources = GameResources::load(&dir);
    assert_eq!(resources.audio_index().bank_count(), 2);
    assert_eq!(resources.audio_index().archive_name(bank_a), Some("pkg_a"));
    assert_eq!(resources.audio_index().archive_name(bank_b), Some("pkg_b"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_base_archive_loads_a_real_bank_through_slim() {
    let dir = tmp_dir("load-base");
    write_legacy_install(&dir);

    let bank_id = 0x9999_8888_7777_6666u64;
    let pkg_name = "soundpkg";
    std::fs::write(dir.join(pkg_name), build_legacy_package(&[(bank_id, WWISE_BANK, build_bank_toc_data())])).unwrap();

    let resources = GameResources::load(&dir);
    let archive_name = resources.audio_index().archive_name(bank_id).expect("bank should be indexed").to_string();
    assert_eq!(archive_name, pkg_name);

    let mut m = Mod::new();
    assert!(m.load_base_archive(resources.slim(), &archive_name), "load_base_archive should succeed");
    assert!(m.wwise_banks.contains_key(&bank_id), "the loaded archive's bank should be pooled onto Mod");

    // loading the same archive name twice is a no-op, matching load_archive_file
    assert!(!m.load_base_archive(resources.slim(), &archive_name));

    let _ = std::fs::remove_dir_all(&dir);
}
