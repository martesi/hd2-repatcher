//! Port of the game-resource indexing from `reference/update_unit_mods.py`
//! (`load_game_resources`, `load_resources_from_file`,
//! `get_data_from_original_file`). Builds the unit-id -> location map and reads
//! original unit data, dispatching between slim and legacy installs.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::audio_resources::AudioIndex;
use crate::patch::{UnitData, UnitDataSource};
use crate::slim::Slim;
use crate::{UNIT_TYPE_ID, WWISE_BANK};

/// Where a unit resource lives inside the game data. Mirrors the Python tuple
/// `(basename, toc_data_offset, toc_data_size)`.
#[derive(Clone)]
struct ResourceLoc {
    name: String,
    toc_data_offset: u64,
    toc_data_size: u32,
}

/// Indexed Helldivers II game data. Passed to the engine as the [`UnitDataSource`].
pub struct GameResources {
    folder: PathBuf,
    slim: Slim,
    mapping: HashMap<u64, ResourceLoc>,
    audio: AudioIndex,
}

impl GameResources {
    /// Points at a game data folder and indexes its unit *and* soundbank
    /// resources in a single pass (one `Slim` init, one directory walk).
    /// Mirrors `init_game_resources`, generalized per
    /// `.ref/native-audio-patch-plan.md`'s "Archive index" section to also
    /// build the [`AudioIndex`] that replaces the external friendlynames db.
    pub fn load(path: &Path) -> Self {
        let slim = Slim::init(path);
        let (mapping, audio_mapping) = load_game_resources(path, &slim);
        GameResources {
            folder: path.to_path_buf(),
            slim,
            mapping,
            audio: AudioIndex::from_mapping(audio_mapping),
        }
    }

    pub fn unit_count(&self) -> usize {
        self.mapping.len()
    }

    /// Soundbank id -> containing base archive lookup, for native audio-patch
    /// loading (`wwise::Mod::load_base_archive`).
    pub fn audio_index(&self) -> &AudioIndex {
        &self.audio
    }

    /// The `Slim` package accessor this game data was indexed through, for
    /// native audio-patch loading (`wwise::Mod::load_base_archive`, which
    /// needs to decompress base archives the same way this index was built).
    pub fn slim(&self) -> &Slim {
        &self.slim
    }
}

impl UnitDataSource for GameResources {
    fn contains(&self, unit_id: u64) -> bool {
        self.mapping.contains_key(&unit_id)
    }

    fn get_unit_data(&self, unit_id: u64) -> UnitData {
        let loc = &self.mapping[&unit_id];
        if self.slim.is_slim {
            let unit_data = self.slim.get_resource_from_package(
                &loc.name,
                loc.toc_data_offset,
                loc.toc_data_size as usize,
            );
            unit_data_from_bytes(&unit_data)
        } else {
            read_unit_data_legacy(&self.folder.join(&loc.name), loc.toc_data_offset)
        }
    }
}

fn u32_at(buf: &[u8], off: usize) -> Option<u32> {
    buf.get(off..off + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}

fn u64_at(buf: &[u8], off: usize) -> Option<u64> {
    buf.get(off..off + 8)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
}

/// Extracts (version, lod_group_data, lod_group_size) from a full unit resource
/// blob (slim path).
fn unit_data_from_bytes(unit_data: &[u8]) -> UnitData {
    let mut version = [0u8; 4];
    if unit_data.len() >= 0x30 {
        version.copy_from_slice(&unit_data[0x2C..0x30]);
    }
    let lod_group_offset = u32_at(unit_data, 0x30).unwrap_or(0) as usize;
    let joint_list_offset = u32_at(unit_data, 0x34).unwrap_or(0) as usize;
    let lod_group_size = joint_list_offset as i64 - lod_group_offset as i64;
    let end = lod_group_offset + lod_group_size.max(0) as usize;
    let lod_group_data = unit_data
        .get(lod_group_offset..end.min(unit_data.len()))
        .unwrap_or(&[])
        .to_vec();
    UnitData {
        version,
        lod_group_data,
        lod_group_size,
    }
}

/// Reads (version, lod_group_data, lod_group_size) directly from a legacy
/// package file at `data_offset`.
fn read_unit_data_legacy(path: &Path, data_offset: u64) -> UnitData {
    let mut version = [0u8; 4];
    let (mut lod_group_offset, mut joint_list_offset) = (0i64, 0i64);
    if let Ok(mut f) = File::open(path) {
        if f.seek(SeekFrom::Start(data_offset + 0x2C)).is_ok() {
            let _ = f.read_exact(&mut version);
            let mut buf = [0u8; 8];
            if f.read_exact(&mut buf).is_ok() {
                lod_group_offset = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as i64;
                joint_list_offset = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as i64;
            }
        }
        let lod_group_size = joint_list_offset - lod_group_offset;
        let mut lod_group_data = vec![0u8; lod_group_size.max(0) as usize];
        if f
            .seek(SeekFrom::Start(data_offset + lod_group_offset as u64))
            .is_ok()
        {
            let _ = f.read_exact(&mut lod_group_data);
        }
        return UnitData {
            version,
            lod_group_data,
            lod_group_size,
        };
    }
    UnitData {
        version,
        lod_group_data: Vec::new(),
        lod_group_size: 0,
    }
}

/// One row from a package's (header-only) TOC file-header table. Shared
/// return shape for [`scan_package_toc`] — see its doc comment.
pub(crate) struct TocRow {
    pub type_id: u64,
    pub file_id: u64,
    pub toc_data_offset: u64,
    pub toc_data_size: u32,
}

/// Parses a package's header-only TOC ([`Slim::get_package_toc`]) and returns
/// its basename display name alongside every file-header row whose
/// `type_id` is in `want`. Shared by [`load_resources_from_file`] (filters
/// for `UNIT_TYPE_ID`) and [`crate::audio_resources::AudioIndex`]'s build
/// step (filters for `WWISE_BANK`) — same 72-byte header + 80-byte-stride
/// file-header walk, generalized over which type ids the caller cares about
/// per `.ref/native-audio-patch-plan.md`'s "Archive index" section.
pub(crate) fn scan_package_toc(display_name: &str, slim: &Slim, want: &[u64]) -> (String, Vec<TocRow>) {
    let name = Path::new(display_name)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| display_name.to_string());
    let toc = slim.get_package_toc(display_name);
    if toc.len() < 72 {
        return (name, Vec::new());
    }
    let num_types = u32_at(&toc, 4).unwrap_or(0) as usize;
    let num_files = u32_at(&toc, 8).unwrap_or(0) as usize;
    let toc_start = 72 + 32 * num_types;
    let mut out = Vec::new();
    for n in 0..num_files {
        let hpos = toc_start + n * 80;
        let (Some(file_id), Some(type_id), Some(toc_data_offset), Some(toc_data_size)) = (
            u64_at(&toc, hpos),
            u64_at(&toc, hpos + 8),
            u64_at(&toc, hpos + 16),
            u32_at(&toc, hpos + 56),
        ) else {
            break;
        };
        if want.contains(&type_id) {
            out.push(TocRow {
                type_id,
                file_id,
                toc_data_offset,
                toc_data_size,
            });
        }
    }
    (name, out)
}

/// Parses a package's TOC and records every unit resource and every
/// soundbank it contains.
fn load_resources_from_file(display_name: &str, slim: &Slim) -> (Vec<(u64, ResourceLoc)>, Vec<(u64, String)>) {
    let (name, rows) = scan_package_toc(display_name, slim, &[UNIT_TYPE_ID, WWISE_BANK]);
    let mut units = Vec::new();
    let mut banks = Vec::new();
    for row in rows {
        if row.type_id == UNIT_TYPE_ID {
            units.push((
                row.file_id,
                ResourceLoc {
                    name: name.clone(),
                    toc_data_offset: row.toc_data_offset,
                    toc_data_size: row.toc_data_size,
                },
            ));
        } else if row.type_id == WWISE_BANK {
            banks.push((row.file_id, name.clone()));
        }
    }
    (units, banks)
}

/// Builds the full unit-id -> location map and soundbank-id -> archive-name
/// map for the whole game data folder, in one pass.
fn load_game_resources(folder: &Path, slim: &Slim) -> (HashMap<u64, ResourceLoc>, HashMap<u64, String>) {
    let names: Vec<String> = if slim.is_slim {
        slim_package_names(folder)
    } else {
        let mut files = Vec::new();
        collect_extensionless_files(folder, &mut files);
        files
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    };

    // Index packages in parallel, then fold in order so duplicate ids resolve
    // deterministically (last package wins, matching a serial dict build).
    let per_file: Vec<(Vec<(u64, ResourceLoc)>, Vec<(u64, String)>)> = names
        .par_iter()
        .map(|name| load_resources_from_file(name, slim))
        .collect();

    let mut mapping = HashMap::new();
    let mut audio_mapping = HashMap::new();
    for (units, banks) in per_file {
        for (id, loc) in units {
            mapping.insert(id, loc);
        }
        for (id, name) in banks {
            audio_mapping.insert(id, name);
        }
    }
    (mapping, audio_mapping)
}

/// Package names for a slim install, read from `bundle_database.data`.
fn slim_package_names(folder: &Path) -> Vec<String> {
    let Ok(data) = std::fs::read(folder.join("bundle_database.data")) else {
        return Vec::new();
    };
    let num_packages = match u32_at(&data, 4) {
        Some(n) => n as usize,
        None => return Vec::new(),
    };
    let mut names = Vec::with_capacity(num_packages);
    for i in 0..num_packages {
        let off = 0x10 + 0x33 * i;
        let Some(raw) = data.get(off..off + 0x33) else {
            break;
        };
        let decoded = String::from_utf8_lossy(raw);
        let name = decoded.split('\u{17}').next().unwrap_or("").to_string();
        names.push(name);
    }
    names
}

fn collect_extensionless_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_extensionless_files(&path, out);
        } else if path.extension().is_none() {
            out.push(path);
        }
    }
}
