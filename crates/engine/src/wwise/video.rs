//! Port of `VideoSource`.
//!
//! Deviates from the Python, which stores `(filepath, stream_offset,
//! video_size)` and re-reads the backing `.stream` file from disk on every
//! `get_data()` call (a GUI memory-saving lazy load). This engine runs a
//! single load -> mutate -> serialize pass per invocation with no
//! undo/reload session to keep synced with disk, so the unmodified bytes are
//! sliced out once at load time instead — behaviourally identical for that
//! pipeline, without needing a live handle back to the archive's `.stream`
//! buffer.

#[derive(Debug, Default, Clone)]
pub struct VideoSource {
    pub file_id: u64,
    pub video_size: u32,
    pub modified: bool,
    original_data: Vec<u8>,
    replacement_data: Vec<u8>,
}

impl VideoSource {
    pub fn from_original(file_id: u64, video_size: u32, original_data: Vec<u8>) -> Self {
        VideoSource {
            file_id,
            video_size,
            modified: false,
            original_data,
            replacement_data: Vec::new(),
        }
    }

    pub fn id(&self) -> u64 {
        self.file_id
    }

    pub fn get_data(&self) -> &[u8] {
        if self.modified {
            &self.replacement_data
        } else {
            &self.original_data
        }
    }

    pub fn set_data(&mut self, data: Vec<u8>) {
        self.replacement_data = data;
        self.modified = true;
    }
}
