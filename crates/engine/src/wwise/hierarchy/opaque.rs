//! Generic passthrough for every HIRC type not yet given a structured parser.
//! Mirrors the Python base `HircEntry` (`wwise_hierarchy_140.py`/`_154.py`,
//! byte-identical between bank versions), which every *unhandled* hierarchy
//! type already round-trips through unmodified.

use crate::memstream::MemoryStream;

/// `(hierarchy_type, size, hierarchy_id, misc bytes)` — `size` is the on-disk
/// size field (covers `hierarchy_id` + `misc`, i.e. `misc.len() + 4`).
#[derive(Debug, Clone)]
pub struct OpaqueEntry {
    pub hierarchy_type: u8,
    pub size: u32,
    pub hierarchy_id: u32,
    pub misc: Vec<u8>,
}

impl OpaqueEntry {
    pub fn read(stream: &mut MemoryStream, hierarchy_type: u8, size: u32) -> Self {
        let hierarchy_id = stream.read_u32();
        let misc = stream.read((size - 4) as usize).to_vec();
        OpaqueEntry {
            hierarchy_type,
            size,
            hierarchy_id,
            misc,
        }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + 4 + 4 + self.misc.len());
        out.push(self.hierarchy_type);
        out.extend_from_slice(&self.size.to_le_bytes());
        out.extend_from_slice(&self.hierarchy_id.to_le_bytes());
        out.extend_from_slice(&self.misc);
        out
    }
}
