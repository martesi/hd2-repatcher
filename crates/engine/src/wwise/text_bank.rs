//! Port of `TextBank` and `StringEntry`. `StringEntry`'s Python `parent`
//! back-reference is dropped: `TextBank::import_text` (its only mutator in
//! this headless engine) needs no back-pointer since it already holds `self`.

use indexmap::IndexMap;

use crate::memstream::MemoryStream;

#[derive(Debug, Clone)]
pub struct StringEntry {
    pub string_id: u32,
    pub text: String,
    pub text_old: String,
    pub modified: bool,
}

/// Port of `TextBank`.
#[derive(Debug, Default, Clone)]
pub struct TextBank {
    pub file_id: u64,
    pub entries: IndexMap<u32, StringEntry>,
    pub language: u32,
    pub modified: bool,
    modified_count: u32,
}

impl TextBank {
    pub fn new(file_id: u64) -> Self {
        TextBank {
            file_id,
            ..Default::default()
        }
    }

    pub fn id(&self) -> u64 {
        self.file_id
    }

    /// `TextBank.set_data`: parses the raw TOC payload (id table + offset
    /// table + null-terminated UTF-8 strings). Reproduces the Python's
    /// off-by-one null scan verbatim (it starts checking for the terminator
    /// one byte *past* `string_offset`, so the byte at `string_offset` is
    /// never itself tested) for byte-for-byte fidelity.
    pub fn set_data(&mut self, data: &[u8]) {
        self.entries.clear();
        let num_entries = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
        self.language = u32::from_le_bytes(data[12..16].try_into().unwrap());
        let id_start = 16;
        let offset_start = id_start + 4 * num_entries;
        let data_start = offset_start + 4 * num_entries;
        let ids = &data[id_start..offset_start];
        let offsets = &data[offset_start..data_start];
        for n in 0..num_entries {
            let string_id = u32::from_le_bytes(ids[4 * n..4 * n + 4].try_into().unwrap());
            let string_offset = u32::from_le_bytes(offsets[4 * n..4 * n + 4].try_into().unwrap()) as usize;
            let mut stop = string_offset + 1;
            while data[stop] != 0 {
                stop += 1;
            }
            let text = String::from_utf8_lossy(&data[string_offset..stop]).into_owned();
            self.entries.insert(
                string_id,
                StringEntry {
                    string_id,
                    text,
                    text_old: String::new(),
                    modified: false,
                },
            );
        }
    }

    /// `TextBank.import_text`: merges string replacements from a patch's
    /// `TextBank` into this one, matching Python's "only if the incoming text
    /// differs from what would be reverted to" merge condition.
    pub fn import_text(&mut self, other: &TextBank) {
        for new_entry in other.entries.values() {
            let Some(old) = self.entries.get_mut(&new_entry.string_id) else {
                continue;
            };
            let changed = if old.modified {
                new_entry.text != old.text_old
            } else {
                new_entry.text != old.text
            };
            if changed {
                if !old.modified {
                    old.text_old = old.text.clone();
                    self.modified_count += 1;
                    self.modified = true;
                }
                old.modified = true;
                old.text = new_entry.text.clone();
            }
        }
    }

    /// `TextBank.generate`: serializes back to the on-disk format.
    pub fn generate(&self) -> Vec<u8> {
        let mut stream = MemoryStream::new(Vec::new());
        stream.write(&[0xae, 0xf3, 0x85, 0x3e, 0x01, 0x00, 0x00, 0x00]);
        stream.write_u32(self.entries.len() as u32);
        stream.write_u32(self.language);
        let mut offset = 16 + 8 * self.entries.len();
        for entry in self.entries.values() {
            stream.write_u32(entry.string_id);
        }
        for entry in self.entries.values() {
            stream.write_u32(offset as u32);
            let initial_position = stream.tell();
            stream.seek(offset);
            let mut text_bytes = entry.text.as_bytes().to_vec();
            text_bytes.push(0);
            stream.write(&text_bytes);
            offset += text_bytes.len();
            stream.seek(initial_position);
        }
        stream.into_data()
    }
}
