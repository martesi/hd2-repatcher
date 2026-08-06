//! Port of `update_patch_file` from `reference/update_unit_mods.py` — the core
//! that rewrites unit `lod_group` data inside a mod `.patch` file at exact byte
//! offsets. Kept structurally faithful to the Python so the two can be diffed;
//! `tests/golden.rs` pins the output byte-for-byte against the Python oracle.

use crate::memstream::MemoryStream;
use crate::UNIT_TYPE_ID;
use std::path::Path;

/// Outcome of attempting to update a single patch file. Mirrors the Python
/// status codes `UPDATE_SUCCESS` / `NO_UNIT_FILES` / `CORRUPTED_FILE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOutcome {
    Updated,
    NoUnits,
    Corrupted,
}

/// The original unit data pulled from the game files, used to overwrite the unit
/// in a patch. Mirrors the tuple returned by `get_data_from_original_file`.
pub struct UnitData {
    pub version: [u8; 4],
    pub lod_group_data: Vec<u8>,
    /// `joint_list_offset - lod_group_offset` in the original game file. Equals
    /// `lod_group_data.len()`, but tracked separately to match the Python.
    pub lod_group_size: i64,
}

/// Supplies original unit data by unit id. Implemented by [`crate::GameResources`]
/// for real game data and by a mock in the golden tests.
pub trait UnitDataSource {
    /// Whether the game data indexes this unit id (`unit_id in game_resource_mapping`).
    fn contains(&self, unit_id: u64) -> bool;
    /// Fetches the original unit version + lod-group data. Only called when
    /// [`UnitDataSource::contains`] is true.
    fn get_unit_data(&self, unit_id: u64) -> UnitData;
}

fn u32_at(buf: &[u8], off: usize) -> Option<u32> {
    buf.get(off..off + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}

fn u64_at(buf: &[u8], off: usize) -> Option<u64> {
    buf.get(off..off + 8)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
}

/// A parsed patch TOC header entry, plus its byte position in the TOC. We only
/// keep the fields the algorithm touches; the remaining header bytes stay put in
/// the stream buffer.
struct Header {
    file_id: u64,
    type_id: u64,
    toc_data_offset: i64,
    /// Position of this 80-byte header within the stream.
    offset: i64,
}

/// Updates a single patch file in place. Returns [`PatchOutcome::Corrupted`] on
/// any structural inconsistency or I/O failure, matching the Python status codes.
pub fn update_patch_file(file_path: &Path, source: &impl UnitDataSource) -> PatchOutcome {
    let Ok(original) = std::fs::read(file_path) else {
        return PatchOutcome::Corrupted;
    };
    let file_size = original.len() as i64;

    // Main header: <IIII56s> = magic, numTypes, numFiles, unknown, 56 bytes.
    let (Some(num_types), Some(orig_num_files)) = (u32_at(&original, 4), u32_at(&original, 8)) else {
        return PatchOutcome::Corrupted;
    };
    let num_types = num_types as usize;
    let orig_num_files = orig_num_files as usize;

    // Scan the type table for the unit type, mirroring the incremental reads.
    let mut total_resources: u64 = 0;
    let mut num_resources: i64 = 0;
    let mut type_offset: usize = 0;
    let mut found_unit = false;
    let mut pos = 72;
    for _ in 0..num_types {
        pos += 8; // skip 8 bytes of padding before the entry's type id
        let (Some(resource_type), Some(count)) = (u64_at(&original, pos), u64_at(&original, pos + 8))
        else {
            return PatchOutcome::Corrupted;
        };
        total_resources = total_resources.wrapping_add(count);
        pos += 16;
        type_offset = pos - 8; // position of this entry's count field
        pos += 8; // skip trailing 8 bytes of the 32-byte entry
        if resource_type < (1u64 << 32) {
            return PatchOutcome::Corrupted;
        }
        if resource_type == UNIT_TYPE_ID {
            num_resources = count as i64;
            found_unit = true;
            break;
        }
    }
    if !found_unit {
        return PatchOutcome::NoUnits;
    }
    if total_resources < orig_num_files as u64 {
        return PatchOutcome::Corrupted;
    }

    let toc_start = 72 + 32 * num_types;
    let mut size_offset: i64 = 0;
    let mut stream = MemoryStream::new(original.clone());

    // Read every header, bailing out if any points past the end of the file.
    let mut headers: Vec<Header> = Vec::with_capacity(orig_num_files);
    for n in 0..orig_num_files {
        let hpos = toc_start + n * 80;
        let (Some(file_id), Some(type_id), Some(toc_data_offset)) = (
            u64_at(&original, hpos),
            u64_at(&original, hpos + 8),
            u64_at(&original, hpos + 16),
        ) else {
            return PatchOutcome::Corrupted;
        };
        if toc_data_offset as i64 > file_size {
            return PatchOutcome::Corrupted;
        }
        headers.push(Header {
            file_id,
            type_id,
            toc_data_offset: toc_data_offset as i64,
            offset: hpos as i64,
        });
    }

    // Drop unit headers the game data no longer knows about, deleting their
    // 80-byte TOC entries and tracking how far everything after them shifts.
    let mut header_offset_adjustment: i64 = 0;
    let mut kept: Vec<Header> = Vec::with_capacity(headers.len());
    for mut header in headers {
        header.offset += header_offset_adjustment;
        if !source.contains(header.file_id) && header.type_id == UNIT_TYPE_ID {
            stream.seek(header.offset as usize);
            stream.delete(80);
            num_resources -= 1;
            header_offset_adjustment -= 80;
        } else {
            kept.push(header);
        }
    }
    let num_files = orig_num_files as i64 + header_offset_adjustment / 80;

    // Removing header bytes shifts all data (which follows the headers) up by
    // the same amount.
    for header in &mut kept {
        header.toc_data_offset += header_offset_adjustment;
    }
    kept.sort_by_key(|h| h.toc_data_offset);

    stream.seek(8);
    stream.write_u32(num_files as u32);
    stream.seek(type_offset);
    stream.write_u64(num_resources as u64);

    for header in &kept {
        stream.seek((header.offset + 16) as usize);
        stream.write_u64((header.toc_data_offset + size_offset) as u64);

        if header.type_id == UNIT_TYPE_ID && source.contains(header.file_id) {
            let unit_start = (header.toc_data_offset + size_offset) as usize;

            // Legacy layout fix-up for old unit versions.
            stream.seek(unit_start + 0x2C);
            let v = stream.read_u32();
            if v < 0xA4CD36 {
                stream.seek(unit_start + 0x5C);
                let layout_list_offset = stream.read_u32() as usize;
                stream.seek(unit_start + layout_list_offset);
                let num_layouts = stream.read_u32();
                let layout_offsets: Vec<u32> = (0..num_layouts).map(|_| stream.read_u32()).collect();
                for layout_offset in layout_offsets {
                    stream.seek(unit_start + layout_list_offset + layout_offset as usize);
                    stream.advance(8);
                    for _ in 0..16 {
                        let _item_type = stream.read_u32();
                        let item_format = stream.read_u32();
                        if item_format > 16 {
                            stream.advance(-4);
                            stream.write_u32(item_format + 4);
                        }
                        stream.advance(12);
                    }
                }
            }

            let ud = source.get_unit_data(header.file_id);

            stream.seek(unit_start);
            stream.advance(0x2C);
            stream.write(&ud.version);
            let lod_group_offset = stream.read_u32() as usize;
            let joint_list_offset = stream.read_u32() as usize;
            let group_size = joint_list_offset as i64 - lod_group_offset as i64;

            stream.seek(unit_start + lod_group_offset);
            let size_difference = ud.lod_group_size - group_size;
            if size_difference > 0 {
                stream.insert(size_difference as usize);
            } else {
                stream.delete((-size_difference) as usize);
            }

            // Shift every offset in the 16-entry table that sits after the lod group.
            stream.seek(unit_start + 0x34);
            for _ in 0..16 {
                let offset = stream.read_u32();
                if offset != 0 && offset as usize > lod_group_offset {
                    stream.advance(-4);
                    stream.write_u32((offset as i64 + size_difference) as u32);
                }
            }

            stream.seek(unit_start + lod_group_offset);
            stream.write(&ud.lod_group_data);
            size_offset += size_difference;
        }
    }

    // Write back. Python re-writes from offset 0 without truncating, so if the
    // buffer shrank, the original tail beyond it survives — reproduce that.
    let mut newdata = stream.into_data();
    if original.len() > newdata.len() {
        newdata.extend_from_slice(&original[newdata.len()..]);
    }
    if std::fs::write(file_path, &newdata).is_err() {
        return PatchOutcome::Corrupted;
    }
    PatchOutcome::Updated
}
