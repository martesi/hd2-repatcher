//! The installed game's resource wrappers must survive old-mod imports.
//! Python parity fixtures use one fixed tag, so they cannot catch this regression.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use engine::wwise::archive::TocHeader;
use engine::wwise::{GameArchive, Mod};
use engine::{process_audio_patches, GameResources, WWISE_BANK, WWISE_STREAM};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn work_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("hd2-audio-headers-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn headers(data: &[u8]) -> Vec<TocHeader> {
    let u32_at = |p| u32::from_le_bytes(data[p..p + 4].try_into().unwrap());
    let u64_at = |p| u64::from_le_bytes(data[p..p + 8].try_into().unwrap());
    let start = 72 + u32_at(4) as usize * 32;
    (0..u32_at(8) as usize)
        .map(|i| {
            let p = start + i * 80;
            TocHeader {
                file_id: u64_at(p),
                type_id: u64_at(p + 8),
                toc_data_offset: u64_at(p + 16),
                toc_data_size: u32_at(p + 56),
                ..Default::default()
            }
        })
        .collect()
}

fn set_tags(path: &Path, expected: &mut HashMap<(u64, u64), Vec<u8>>) {
    let mut data = std::fs::read(path).unwrap();
    for h in headers(&data) {
        let prefix_len = match h.type_id {
            WWISE_BANK => 4,
            WWISE_STREAM => 8,
            _ => continue,
        };
        // Different tags per resource prove there is no archive-wide constant.
        let tag = 0x78A3_F4A5u32.wrapping_add(h.file_id as u32);
        let start = h.toc_data_offset as usize;
        data[start..start + 4].copy_from_slice(&tag.to_le_bytes());
        if h.type_id == WWISE_STREAM {
            data[start + 4..start + 8].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        }
        expected.insert((h.type_id, h.file_id), data[start..start + prefix_len].to_vec());
    }
    std::fs::write(path, data).unwrap();
}

fn assert_tags(path: &Path, expected: &HashMap<(u64, u64), Vec<u8>>) -> usize {
    let data = std::fs::read(path).unwrap();
    let mut checked = 0;
    for h in headers(&data) {
        if !matches!(h.type_id, WWISE_BANK | WWISE_STREAM) {
            continue;
        }
        let prefix = expected.get(&(h.type_id, h.file_id)).expect("resource came from the base game");
        let start = h.toc_data_offset as usize;
        assert_eq!(&data[start..start + prefix.len()], prefix, "wrong wrapper for {:016x}", h.file_id);
        if h.type_id == WWISE_BANK {
            assert_eq!(u32::from_le_bytes(data[start + 4..start + 8].try_into().unwrap()) + 16, h.toc_data_size);
        }
        checked += 1;
    }
    checked
}

fn copy_dir(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let path = entry.unwrap().path();
        std::fs::copy(&path, target.join(path.file_name().unwrap())).unwrap();
    }
}

#[test]
fn roundtrip_keeps_each_bank_and_stream_wrapper() {
    let dir = work_dir("roundtrip");
    let case = fixtures().join("audio_roundtrip_basic");
    let input = dir.join("audio.patch_0");
    std::fs::copy(case.join("input.bin"), &input).unwrap();
    std::fs::copy(case.join("input.bin.stream"), dir.join("audio.patch_0.stream")).unwrap();
    let mut expected = HashMap::new();
    set_tags(&input, &mut expected);
    let archive = GameArchive::from_patch_file(&input).unwrap();
    archive.to_file(&dir).unwrap();
    assert_eq!(assert_tags(&input, &expected), 2);
    assert_eq!(
        std::fs::read(dir.join("audio.patch_0.stream")).unwrap(),
        std::fs::read(case.join("expected.bin.stream")).unwrap()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn old_mod_import_keeps_base_headers_in_combined_and_separate_output() {
    let dir = work_dir("import");
    let case = fixtures().join("process_audio_patches_multi_archive");
    let game = dir.join("game");
    let patches = dir.join("patches");
    copy_dir(&case.join("gamedata"), &game);
    copy_dir(&case.join("patches"), &patches);
    let mut expected = HashMap::new();
    for entry in std::fs::read_dir(&game).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none() && std::fs::metadata(&path).unwrap().len() > 0 {
            set_tags(&path, &mut expected);
        }
    }
    let resources = GameResources::load(&game);
    let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(case.join("meta.json")).unwrap()).unwrap();
    let inputs: Vec<PathBuf> =
        meta["patch_names"].as_array().unwrap().iter().map(|n| patches.join(n.as_str().unwrap())).collect();
    let mut mod_ = Mod::new();
    for entry in std::fs::read_dir(&game).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none() && std::fs::metadata(&path).unwrap().len() > 0 {
            assert!(mod_.load_base_archive(resources.slim(), path.file_name().unwrap().to_str().unwrap()));
        }
    }
    for input in &inputs {
        assert!(mod_.import_patch(input, true));
    }
    let separate = dir.join("separate");
    std::fs::create_dir_all(&separate).unwrap();
    mod_.write_separate_patches(&separate).unwrap();
    let count: usize = std::fs::read_dir(separate)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "patch_0"))
        .map(|p| assert_tags(&p, &expected))
        .sum();
    assert!(count > 0);
    process_audio_patches(&patches, &inputs, &resources).unwrap();
    assert!(assert_tags(&inputs[0], &expected) > 0);
    // Audio and hierarchy output must still match the established oracle;
    // only the resource prefixes intentionally diverge from Python.
    let oracle = std::fs::read(case.join("expected/9ba626afa44a3aa3.patch_0")).unwrap();
    let mut output = std::fs::read(&inputs[0]).unwrap();
    for h in headers(&oracle) {
        let len = match h.type_id {
            WWISE_BANK => 4,
            WWISE_STREAM => 8,
            _ => continue,
        };
        let start = h.toc_data_offset as usize;
        output[start..start + len].copy_from_slice(&oracle[start..start + len]);
    }
    assert_eq!(output, oracle, "audio payload or hierarchy changed beyond the resource wrapper");
    std::fs::remove_dir_all(dir).unwrap();
}
