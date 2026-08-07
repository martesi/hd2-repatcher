//! Port of the HIRC hierarchy container (`WwiseHierarchy_140`/`_154` in the
//! Python source) and the `HircEntryFactory` dispatch, plus
//! `WwiseHierarchy::import_hierarchy` (the `import_patch` field-merge for
//! the 4 eligible types: `Sound`, `MusicTrack`, `MusicSegment`,
//! `RandomSequenceContainer`). Everything else still parses as
//! [`opaque::OpaqueEntry`] — a later phase adds the remaining 4 container
//! types (`ActorMixer`, `SwitchContainer`, `LayerContainer`,
//! `MusicSwitchContainer`) and extends [`parse_entry`]'s dispatch to match.

mod base_param;
mod containers;
mod music_segment;
mod music_track;
mod opaque;
mod sound;

pub use containers::{ContainerChildren, PlayListItem, PlayListSetting, RandomSequenceContainer};
pub use music_segment::{Marker, MusicSegment, MusicSegmentHead};
pub use music_track::{ClipAutomationStruct, MusicTrack, MusicTrackBody, TrackInfoStruct};
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
    RandomSequenceContainer(RandomSequenceContainer),
    MusicSegment(MusicSegment),
    MusicTrack(MusicTrack),
}

impl HircEntry {
    pub fn id(&self) -> u32 {
        match self {
            HircEntry::Opaque(e) => e.hierarchy_id,
            HircEntry::Sound(s) => s.hierarchy_id,
            HircEntry::RandomSequenceContainer(c) => c.hierarchy_id,
            HircEntry::MusicSegment(m) => m.hierarchy_id,
            HircEntry::MusicTrack(m) => m.hierarchy_id,
        }
    }

    pub fn hierarchy_type(&self) -> u8 {
        match self {
            HircEntry::Opaque(e) => e.hierarchy_type,
            HircEntry::Sound(_) => 0x02,
            HircEntry::RandomSequenceContainer(_) => 0x05,
            HircEntry::MusicSegment(_) => 0x0a,
            HircEntry::MusicTrack(_) => 0x0b,
        }
    }

    pub fn get_data(&self) -> Vec<u8> {
        match self {
            HircEntry::Opaque(e) => e.get_data(),
            HircEntry::Sound(s) => s.get_data(),
            HircEntry::RandomSequenceContainer(c) => c.get_data(),
            HircEntry::MusicSegment(m) => m.get_data(),
            HircEntry::MusicTrack(m) => m.get_data(),
        }
    }

    /// `HircEntry.import_entry` dispatch (the `set_data` half of it — our
    /// port drops the `modified`/`data_old` change-tracking guard Python
    /// uses around it, since unconditionally re-applying the same field
    /// copy is behaviorally identical). Mismatched variant pairs (which
    /// [`WwiseHierarchy::import_hierarchy`]'s type filter should never
    /// produce) are a no-op.
    pub fn import_entry(&mut self, other: &HircEntry, version: BankVersion) {
        match (self, other) {
            (HircEntry::Sound(s), HircEntry::Sound(o)) => s.import_entry(o, version),
            (HircEntry::RandomSequenceContainer(s), HircEntry::RandomSequenceContainer(o)) => {
                s.import_entry(o, version)
            }
            (HircEntry::MusicSegment(s), HircEntry::MusicSegment(o)) => s.import_entry(o, version),
            (HircEntry::MusicTrack(s), HircEntry::MusicTrack(o)) => s.import_entry(o, version),
            _ => {}
        }
    }
}

/// `HircEntryFactory.from_memory_stream`: reads the `(type u8, size u32)`
/// header and dispatches on `hierarchy_type`. Every type not listed here
/// falls through to [`HircEntry::Opaque`].
fn parse_entry(stream: &mut MemoryStream, version: BankVersion) -> HircEntry {
    let hierarchy_type = stream.read_u8();
    let size = stream.read_u32();
    match hierarchy_type {
        0x02 => HircEntry::Sound(Sound::read(stream, size, version)),
        0x05 => HircEntry::RandomSequenceContainer(RandomSequenceContainer::read(stream, size, version)),
        0x0a => HircEntry::MusicSegment(MusicSegment::read(stream, size, version)),
        0x0b => HircEntry::MusicTrack(MusicTrack::read(stream, size, version)),
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

    /// `WwiseHierarchy.get_music_tracks`: a live filter over `entries`, same
    /// approach as [`Self::sounds`]. `WwiseBank::generate`'s DIDX/DATA loop
    /// chains this onto [`Self::sounds`] (matching Python's
    /// `get_sounds() + get_music_tracks()`).
    pub fn music_tracks(&self) -> Vec<&MusicTrack> {
        self.entries
            .values()
            .filter_map(|e| match e {
                HircEntry::MusicTrack(m) => Some(m),
                _ => None,
            })
            .collect()
    }

    /// `WwiseHierarchy_140.import_hierarchy` / `WwiseHierarchy_154.import_hierarchy`:
    /// `import_patch`'s field-level HIRC merge. A real, verified-against-
    /// upstream asymmetry between bank versions (not an oversight):
    /// - v140 only merges `MusicSegment`/`MusicTrack` entries (NOT `Sound`/
    ///   `RandomSequenceContainer`), and adds the incoming entry wholesale if
    ///   its id isn't already present in `self`.
    /// - v154 merges `Sound`/`RandomSequenceContainer`/`MusicTrack`/
    ///   `MusicSegment` (a strictly wider type filter), but has no
    ///   add-branch: an incoming entry whose id isn't already present in
    ///   `self` is silently dropped.
    ///
    /// Cross-version entries (Python's `isinstance(entry, (..., wwise_hierarchy_140.MusicTrack, ...))`
    /// checks) aren't modeled — `self` and `new_hierarchy` always share a
    /// version in practice (they're the same soundbank's base and patch
    /// hierarchies), and our unified `HircEntry` enum doesn't carry a
    /// separate "which Python module" tag the way upstream's two mirrored
    /// class hierarchies do.
    pub fn import_hierarchy(&mut self, new_hierarchy: &WwiseHierarchy) {
        let version = self.version;
        for entry in new_hierarchy.entries.values() {
            let mergeable = match version {
                BankVersion::V140 => matches!(entry, HircEntry::MusicSegment(_) | HircEntry::MusicTrack(_)),
                BankVersion::V154 => matches!(
                    entry,
                    HircEntry::Sound(_)
                        | HircEntry::RandomSequenceContainer(_)
                        | HircEntry::MusicTrack(_)
                        | HircEntry::MusicSegment(_)
                ),
            };
            if !mergeable {
                continue;
            }
            let id = entry.id();
            if let Some(existing) = self.entries.get_mut(&id) {
                existing.import_entry(entry, version);
            } else if version == BankVersion::V140 {
                self.entries.insert(id, entry.clone());
            }
        }
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
