//! Faithful port of the `MemoryStream` helper from the Python engine
//! (`reference/update_unit_mods.py`). Semantics are matched exactly, including
//! the quirk that `seek`/`advance` past the end zero-extend the buffer.
//!
//! Original MemoryStream: modified from
//! <https://github.com/kboykboy2/io_scene_helldivers2> with permission from kboykboy.

/// A growable in-memory cursor over a byte buffer with the same behaviour as
/// the Python `MemoryStream`. A few accessors are kept for API completeness even
/// though the engine does not currently call them.
pub struct MemoryStream {
    data: Vec<u8>,
    location: usize,
}

#[allow(dead_code)]
impl MemoryStream {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data, location: 0 }
    }

    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn tell(&self) -> usize {
        self.location
    }

    fn extend_to(&mut self, location: usize) {
        if location > self.data.len() {
            self.data.resize(location, 0);
        }
    }

    /// Matches Python `seek`: moving past the end zero-extends the buffer.
    pub fn seek(&mut self, location: usize) {
        self.location = location;
        self.extend_to(self.location);
    }

    /// Matches Python `advance`: clamps to 0 on the low end, zero-extends on the
    /// high end.
    pub fn advance(&mut self, offset: i64) {
        let new = self.location as i64 + offset;
        self.location = if new < 0 { 0 } else { new as usize };
        self.extend_to(self.location);
    }

    /// Reads `length` bytes and advances. Panics past the end, matching the
    /// Python `read` which raises.
    pub fn read(&mut self, length: usize) -> &[u8] {
        let end = self.location + length;
        assert!(end <= self.data.len(), "reading past end of stream");
        let slice = &self.data[self.location..end];
        self.location = end;
        slice
    }

    pub fn read_u8(&mut self) -> u8 {
        self.read(1)[0]
    }

    pub fn read_i8(&mut self) -> i8 {
        self.read(1)[0] as i8
    }

    pub fn read_u16(&mut self) -> u16 {
        let b = self.read(2);
        u16::from_le_bytes([b[0], b[1]])
    }

    pub fn read_i16(&mut self) -> i16 {
        let b = self.read(2);
        i16::from_le_bytes([b[0], b[1]])
    }

    pub fn read_u32(&mut self) -> u32 {
        let b = self.read(4);
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    pub fn read_i32(&mut self) -> i32 {
        let b = self.read(4);
        i32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    pub fn read_u64(&mut self) -> u64 {
        let b = self.read(8);
        u64::from_le_bytes(b.try_into().unwrap())
    }

    pub fn read_i64(&mut self) -> i64 {
        let b = self.read(8);
        i64::from_le_bytes(b.try_into().unwrap())
    }

    pub fn read_f32(&mut self) -> f32 {
        let b = self.read(4);
        f32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    pub fn read_f64(&mut self) -> f64 {
        let b = self.read(8);
        f64::from_le_bytes(b.try_into().unwrap())
    }

    /// Overwrites at the current location, extending as needed, and advances.
    pub fn write(&mut self, bytes: &[u8]) {
        let end = self.location + bytes.len();
        self.extend_to(end);
        self.data[self.location..end].copy_from_slice(bytes);
        self.location = end;
    }

    pub fn write_u32(&mut self, value: u32) {
        self.write(&value.to_le_bytes());
    }

    pub fn write_u64(&mut self, value: u64) {
        self.write(&value.to_le_bytes());
    }

    /// Inserts `length` zero bytes at the current location without advancing.
    pub fn insert(&mut self, length: usize) {
        let zeros = vec![0u8; length];
        self.data.splice(self.location..self.location, zeros);
    }

    /// Removes `length` bytes at the current location.
    pub fn delete(&mut self, length: usize) {
        let end = (self.location + length).min(self.data.len());
        self.data.drain(self.location..end);
    }
}
