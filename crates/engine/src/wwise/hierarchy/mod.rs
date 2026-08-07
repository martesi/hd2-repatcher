//! Port of the HIRC hierarchy container (`WwiseHierarchy_140`/`_154` in the
//! Python source) and the `HircEntryFactory` dispatch. `Sound` is the first
//! structured HIRC type; everything else still parses as
//! [`opaque::OpaqueEntry`] — later phases add more structured variants
//! (`MusicTrack`, the container types, ...) and extend [`parse_entry`]'s
//! dispatch to match.

mod base_param;
mod opaque;
mod sound;

pub use opaque::OpaqueEntry;
pub use sound::{BankSourceStruct, Sound};

use indexmap::IndexMap;

use crate::memstream::MemoryStream;

/// Which BKHD version a bank was authored against; some HIRC struct layouts
/// (ported in later phases) differ between the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankVersion {
    V140,
    V154,
}

/// A single HIRC hierarchy entry. Structured variants are added as their
/// Python counterparts are ported; everything else stays
/// [`HircEntry::Opaque`].
#[derive(Debug, Clone)]
pub enum HircEntry {
    Opaque(OpaqueEntry),
    Sound(Sound),
}

impl HircEntry {
    pub fn id(&self) -> u32 {
        match self {
            HircEntry::Opaque(e) => e.hierarchy_id,
            HircEntry::Sound(s) => s.hierarchy_id,
        }
    }

    pub fn hierarchy_type(&self) -> u8 {
        match self {
            HircEntry::Opaque(e) => e.hierarchy_type,
            HircEntry::Sound(_) => 0x02,
        }
    }

    pub fn get_data(&self) -> Vec<u8> {
        match self {
            HircEntry::Opaque(e) => e.get_data(),
            HircEntry::Sound(s) => s.get_data(),
        }
    }
}

/// `HircEntryFactory.from_memory_stream`: reads the `(type u8, size u32)`
/// header and dispatches on `hierarchy_type`. Every type but `Sound` (0x02)
/// currently falls through to [`HircEntry::Opaque`].
fn parse_entry(stream: &mut MemoryStream, version: BankVersion) -> HircEntry {
    let hierarchy_type = stream.read_u8();
    let size = stream.read_u32();
    match hierarchy_type {
        0x02 => HircEntry::Sound(Sound::read(stream, size, version)),
        _ => HircEntry::Opaque(OpaqueEntry::read(stream, hierarchy_type, size)),
    }
}

/// Port of `WwiseHierarchy_140`/`WwiseHierarchy_154`. Entry order is
/// significant for byte-exact `get_data()` output (Python relies on `dict`
/// insertion order), hence `IndexMap`.
#[derive(Debug, Clone)]
pub struct WwiseHierarchy {
    pub version: BankVersion,
    pub entries: IndexMap<u32, HircEntry>,
}

impl WwiseHierarchy {
    pub fn new(version: BankVersion) -> Self {
        WwiseHierarchy {
            version,
            entries: IndexMap::new(),
        }
    }

    /// `WwiseHierarchy.load`: parses the HIRC chunk body (item count prefix,
    /// then that many hierarchy entries back to back).
    pub fn load(version: BankVersion, hierarchy_data: &[u8]) -> Self {
        let mut hierarchy = WwiseHierarchy::new(version);
        let mut reader = MemoryStream::new(hierarchy_data.to_vec());
        reader.seek(0);
        let num_items = reader.read_u32();
        for _ in 0..num_items {
            let entry = parse_entry(&mut reader, version);
            hierarchy.entries.insert(entry.id(), entry);
        }
        hierarchy
    }

    pub fn has_entry(&self, id: u32) -> bool {
        self.entries.contains_key(&id)
    }

    pub fn get_entry(&self, id: u32) -> Option<&HircEntry> {
        self.entries.get(&id)
    }

    /// `WwiseHierarchy.get_sounds`: a live filter over `entries` rather than
    /// a separately maintained list — since `IndexMap` iteration order
    /// already matches Python's dict insertion order, this yields the exact
    /// same sequence Python's incrementally-built `self.sounds` list would.
    pub fn sounds(&self) -> Vec<&Sound> {
        self.entries
            .values()
            .filter_map(|e| match e {
                HircEntry::Sound(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    /// No structured `MusicTrack` entries exist yet; see [`Self::sounds`].
    /// `WwiseBank::generate`'s DIDX/DATA loop chains this onto
    /// [`Self::sounds`] (matching Python's `get_sounds() + get_music_tracks()`),
    /// so it stays a no-op contribution until phase 3 ports `MusicTrack`.
    pub fn music_tracks(&self) -> Vec<&HircEntry> {
        Vec::new()
    }

    /// `WwiseHierarchy.get_data`: item count prefix + each entry's bytes, in
    /// insertion order.
    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for entry in self.entries.values() {
            out.extend_from_slice(&entry.get_data());
        }
        out
    }
}
