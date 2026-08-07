//! Port of the HIRC hierarchy container (`WwiseHierarchy_140`/`_154` in the
//! Python source) and the `HircEntryFactory` dispatch. Currently every entry
//! parses as [`opaque::OpaqueEntry`] regardless of `hierarchy_type` — later
//! phases add structured variants (`Sound`, `MusicTrack`, the container
//! types, ...) and extend [`parse_entry`]'s dispatch to match.

mod opaque;

pub use opaque::OpaqueEntry;

use indexmap::IndexMap;

use crate::memstream::MemoryStream;

/// Which BKHD version a bank was authored against; some HIRC struct layouts
/// (ported in later phases) differ between the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankVersion {
    V140,
    V154,
}

/// A single HIRC hierarchy entry. Only [`HircEntry::Opaque`] exists so far;
/// structured variants are added as their Python counterparts are ported.
#[derive(Debug, Clone)]
pub enum HircEntry {
    Opaque(OpaqueEntry),
}

impl HircEntry {
    pub fn id(&self) -> u32 {
        match self {
            HircEntry::Opaque(e) => e.hierarchy_id,
        }
    }

    pub fn hierarchy_type(&self) -> u8 {
        match self {
            HircEntry::Opaque(e) => e.hierarchy_type,
        }
    }

    pub fn get_data(&self) -> Vec<u8> {
        match self {
            HircEntry::Opaque(e) => e.get_data(),
        }
    }
}

/// `HircEntryFactory.from_memory_stream`: reads the `(type u8, size u32)`
/// header (without consuming it — callers that dispatch on `hierarchy_type`
/// peek it the same way Python does) and produces the right entry variant.
/// Every type currently falls through to [`HircEntry::Opaque`].
fn parse_entry(stream: &mut MemoryStream, _version: BankVersion) -> HircEntry {
    let hierarchy_type = stream.read_u8();
    let size = stream.read_u32();
    HircEntry::Opaque(OpaqueEntry::read(stream, hierarchy_type, size))
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

    /// No structured `Sound` entries exist yet (opaque-only phase); real
    /// results start once phase 2 adds `HircEntry::Sound`.
    pub fn sounds(&self) -> Vec<&HircEntry> {
        Vec::new()
    }

    /// No structured `MusicTrack` entries exist yet; see [`Self::sounds`].
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
