//! Port of `BankParser`, `DidxEntry`, and `MediaIndex` from the Python source:
//! the RIFF-style chunk reader for a `.bnk` blob (BKHD/DIDX/DATA/HIRC/...) and
//! the audio-source index derived from a bank's DIDX+DATA chunks.

use indexmap::IndexMap;

use crate::memstream::MemoryStream;

/// Splits a bank blob into `tag -> payload` chunks (`BKHD`, `DIDX`, `DATA`,
/// `HIRC`, plus whatever else trails them, kept opaque). Order is preserved
/// since unrecognized trailing chunks are re-emitted verbatim in original
/// order (`WwiseBank.bank_misc_data`).
#[derive(Debug, Default, Clone)]
pub struct BankParser {
    pub chunks: IndexMap<String, Vec<u8>>,
}

impl BankParser {
    pub fn load(bank_data: &[u8]) -> Self {
        let mut chunks = IndexMap::new();
        let mut reader = MemoryStream::new(bank_data.to_vec());
        reader.seek(0);
        loop {
            if reader.tell() + 4 > bank_data.len() {
                break;
            }
            let tag_bytes = reader.read(4).to_vec();
            let Ok(tag) = String::from_utf8(tag_bytes) else {
                break;
            };
            let size = reader.read_u32();
            let payload = reader.read(size as usize).to_vec();
            chunks.insert(tag, payload);
        }
        BankParser { chunks }
    }

    pub fn get_chunk(&self, tag: &str) -> &[u8] {
        self.chunks.get(tag).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// A single `DIDX` chunk entry: `<III>` = (media id, offset into `DATA`, size).
#[derive(Debug, Clone, Copy)]
pub struct DidxEntry {
    pub id: u32,
    pub offset: u32,
    pub size: u32,
}

impl DidxEntry {
    pub fn from_bytes(b: &[u8]) -> Self {
        DidxEntry {
            id: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            offset: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            size: u32::from_le_bytes(b[8..12].try_into().unwrap()),
        }
    }

    pub fn get_data(&self) -> [u8; 12] {
        let mut out = [0u8; 12];
        out[0..4].copy_from_slice(&self.id.to_le_bytes());
        out[4..8].copy_from_slice(&self.offset.to_le_bytes());
        out[8..12].copy_from_slice(&self.size.to_le_bytes());
        out
    }
}

/// Port of `MediaIndex`: decodes a bank's `DIDX`+`DATA` chunk pair into a
/// media-id -> raw-audio-bytes map.
#[derive(Debug, Default, Clone)]
pub struct MediaIndex {
    pub entries: IndexMap<u32, DidxEntry>,
    pub data: IndexMap<u32, Vec<u8>>,
}

impl MediaIndex {
    pub fn load(&mut self, didx_chunk: &[u8], data_chunk: &[u8]) {
        for chunk in didx_chunk.chunks_exact(12) {
            let entry = DidxEntry::from_bytes(chunk);
            let start = entry.offset as usize;
            let end = start + entry.size as usize;
            self.data.insert(entry.id, data_chunk[start..end].to_vec());
            self.entries.insert(entry.id, entry);
        }
    }
}
