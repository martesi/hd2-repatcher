//! Port of the HIRC hierarchy container (`WwiseHierarchy_140`/`_154` in the
//! Python source) and the `HircEntryFactory` dispatch, plus
//! `WwiseHierarchy::import_hierarchy` (the `import_patch` field-merge for
//! the 4 eligible types: `Sound`, `MusicTrack`, `MusicSegment`,
//! `RandomSequenceContainer`) and the five container types' cross-bank
//! children-merge/dangling-child pruning (see `containers.rs`'s module
//! doc). Everything else still parses as [`opaque::OpaqueEntry`].

mod base_param;
mod containers;
mod music_segment;
mod music_track;
mod opaque;
mod sound;

pub use containers::{
    ActorMixer, ContainerChildren, LayerContainer, MusicSwitchContainer, PlayListItem, PlayListSetting,
    RandomSequenceContainer, SwitchContainer, SwitchGroup, SwitchParam,
};
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
    ActorMixer(ActorMixer),
    SwitchContainer(SwitchContainer),
    LayerContainer(LayerContainer),
    MusicSwitchContainer(MusicSwitchContainer),
}

impl HircEntry {
    pub fn id(&self) -> u32 {
        match self {
            HircEntry::Opaque(e) => e.hierarchy_id,
            HircEntry::Sound(s) => s.hierarchy_id,
            HircEntry::RandomSequenceContainer(c) => c.hierarchy_id,
            HircEntry::MusicSegment(m) => m.hierarchy_id,
            HircEntry::MusicTrack(m) => m.hierarchy_id,
            HircEntry::ActorMixer(a) => a.hierarchy_id,
            HircEntry::SwitchContainer(s) => s.hierarchy_id,
            HircEntry::LayerContainer(l) => l.hierarchy_id,
            HircEntry::MusicSwitchContainer(m) => m.hierarchy_id,
        }
    }

    pub fn hierarchy_type(&self) -> u8 {
        match self {
            HircEntry::Opaque(e) => e.hierarchy_type,
            HircEntry::Sound(_) => 0x02,
            HircEntry::RandomSequenceContainer(_) => 0x05,
            HircEntry::SwitchContainer(_) => 0x06,
            HircEntry::ActorMixer(_) => 0x07,
            HircEntry::LayerContainer(_) => 0x09,
            HircEntry::MusicSegment(_) => 0x0a,
            HircEntry::MusicTrack(_) => 0x0b,
            HircEntry::MusicSwitchContainer(_) => 0x0c,
        }
    }

    pub fn get_data(&self) -> Vec<u8> {
        match self {
            HircEntry::Opaque(e) => e.get_data(),
            HircEntry::Sound(s) => s.get_data(),
            HircEntry::RandomSequenceContainer(c) => c.get_data(),
            HircEntry::MusicSegment(m) => m.get_data(),
            HircEntry::MusicTrack(m) => m.get_data(),
            HircEntry::ActorMixer(a) => a.get_data(),
            HircEntry::SwitchContainer(s) => s.get_data(),
            HircEntry::LayerContainer(l) => l.get_data(),
            HircEntry::MusicSwitchContainer(m) => m.get_data(),
        }
    }

    /// `WwiseHierarchy.get_data()`'s per-entry dangling-child pruning
    /// (`entries` is the owning hierarchy's own entry map — see
    /// `containers.rs::prune_children`). A no-op passthrough to
    /// [`Self::get_data`] for the non-container variants.
    pub fn get_data_pruned(&self, entries: &IndexMap<u32, HircEntry>) -> Vec<u8> {
        match self {
            HircEntry::RandomSequenceContainer(c) => c.get_data_pruned(entries),
            HircEntry::ActorMixer(a) => a.get_data_pruned(entries),
            HircEntry::SwitchContainer(s) => s.get_data_pruned(entries),
            HircEntry::LayerContainer(l) => l.get_data_pruned(entries),
            HircEntry::MusicSwitchContainer(m) => m.get_data_pruned(entries),
            _ => self.get_data(),
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

    /// `GameArchive.load`'s cross-bank children union (`core.py:822-829`):
    /// when the same `hierarchy_id` shows up in more than one bank within a
    /// single archive, ids present in `other`'s children but absent from
    /// `self`'s get appended (dedup by value, `size += 4` per appended id).
    /// A no-op for non-container variants or mismatched variant pairs —
    /// real upstream would `AttributeError` on the latter (an
    /// `isinstance`-on-`other`-only check), which can't happen in practice
    /// since a given `hierarchy_id` is always the same concrete type across
    /// every bank that defines it.
    pub fn merge_children(&mut self, other: &HircEntry) {
        let incoming = match other {
            HircEntry::RandomSequenceContainer(c) => &c.children,
            HircEntry::ActorMixer(a) => &a.children,
            HircEntry::SwitchContainer(s) => &s.children,
            HircEntry::LayerContainer(l) => &l.children,
            HircEntry::MusicSwitchContainer(m) => &m.children,
            _ => return,
        };
        let target = match self {
            HircEntry::RandomSequenceContainer(c) => Some((&mut c.children, &mut c.size)),
            HircEntry::ActorMixer(a) => Some((&mut a.children, &mut a.size)),
            HircEntry::SwitchContainer(s) => Some((&mut s.children, &mut s.size)),
            HircEntry::LayerContainer(l) => Some((&mut l.children, &mut l.size)),
            HircEntry::MusicSwitchContainer(m) => Some((&mut m.children, &mut m.size)),
            _ => None,
        };
        let Some((children, size)) = target else { return };
        for &child in &incoming.children {
            if !children.children.contains(&child) {
                children.children.push(child);
                *size += 4;
            }
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
        0x06 => HircEntry::SwitchContainer(SwitchContainer::read(stream, size, version)),
        0x07 => HircEntry::ActorMixer(ActorMixer::read(stream, size, version)),
        0x09 => HircEntry::LayerContainer(LayerContainer::read(stream, size, version)),
        0x0a => HircEntry::MusicSegment(MusicSegment::read(stream, size, version)),
        0x0b => HircEntry::MusicTrack(MusicTrack::read(stream, size, version)),
        0x0c => HircEntry::MusicSwitchContainer(MusicSwitchContainer::read(stream, size, version)),
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
    /// insertion order. Each entry is serialized via
    /// [`HircEntry::get_data_pruned`] (dangling-child pruning against this
    /// hierarchy's own `entries` map — see `containers.rs`'s module doc);
    /// pruning never mutates `self`, matching Python's save-mutate-restore
    /// dance by construction rather than by explicit restore.
    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for entry in self.entries.values() {
            out.extend_from_slice(&entry.get_data_pruned(&self.entries));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    //! Dangling-child pruning (`WwiseHierarchy::get_data`) isn't exercisable
    //! as an oracle-diffed golden fixture — `GameArchive.to_file` (the
    //! Python oracle's only serialization entry point) prunes on *every*
    //! write, including the one used to manufacture a fixture's `input.bin`,
    //! so a hand-built dangling child never survives to be tested against.
    //! Covered here instead as a plain unit test — a filter+arithmetic
    //! operation simple enough not to need oracle verification.

    use super::*;
    use crate::wwise::hierarchy::base_param::{AdvSetting, AuxParams, PropBundle, RangedPropBundle, StateParams};

    fn minimal_base_param(version: BankVersion) -> base_param::BaseParam {
        base_param::BaseParam {
            version,
            is_override_parent_fx: 0,
            bypass_all: 0,
            fx_chunks: Vec::new(),
            is_override_parent_metadata: 0,
            fx_chunks_metadata: Vec::new(),
            override_attachment_params: match version {
                BankVersion::V140 => Some(0),
                BankVersion::V154 => None,
            },
            override_bus_id: 0,
            direct_parent_id: 0,
            by_bit_vector_a: 0,
            prop_bundle: PropBundle::default(),
            ranged_prop_bundle: RangedPropBundle::default(),
            positioning_param_data: vec![0],
            aux_params: AuxParams::default(),
            adv_setting: AdvSetting::default(),
            state_params: StateParams::default(),
            rtpcs: Vec::new(),
        }
    }

    fn actor_mixer(id: u32, children: Vec<u32>) -> ActorMixer {
        let base_param = minimal_base_param(BankVersion::V140);
        // hierarchy_id (4) + base_param + children.get_data() (4 + 4*n).
        let size = 4 + base_param.get_data().len() + 4 + 4 * children.len();
        ActorMixer {
            size: size as u32,
            hierarchy_id: id,
            base_param,
            children: ContainerChildren { children },
        }
    }

    #[test]
    fn get_data_prunes_dangling_children_and_leaves_self_untouched() {
        let mut hierarchy = WwiseHierarchy::new(BankVersion::V140);
        // hierarchy_id 2 is intentionally never defined — dangling.
        let mixer = actor_mixer(1, vec![2, 3]);
        let leaf = HircEntry::Opaque(OpaqueEntry {
            hierarchy_type: 0x01,
            size: 4,
            hierarchy_id: 3,
            misc: vec![],
        });
        hierarchy.entries.insert(1, HircEntry::ActorMixer(mixer.clone()));
        hierarchy.entries.insert(3, leaf);

        let full_size = mixer.size;
        let pruned = hierarchy.get_data();

        // Rebuild the expected bytes: same mixer but with child 2 dropped
        // and size adjusted by exactly one child slot (4 bytes).
        let mut expected_mixer = mixer.clone();
        expected_mixer.children = ContainerChildren { children: vec![3] };
        expected_mixer.size = full_size - 4;
        let mut expected = Vec::new();
        expected.extend_from_slice(&2u32.to_le_bytes()); // entry count
        expected.extend_from_slice(&expected_mixer.get_data());
        expected.extend_from_slice(
            &HircEntry::Opaque(OpaqueEntry { hierarchy_type: 0x01, size: 4, hierarchy_id: 3, misc: vec![] }).get_data(),
        );
        assert_eq!(pruned, expected);

        // `self` must be unaffected by pruning — a second call reproduces
        // the exact same (pruned) bytes, and the in-memory struct still
        // holds the original, un-pruned children list.
        assert_eq!(hierarchy.get_data(), pruned);
        match hierarchy.entries.get(&1).unwrap() {
            HircEntry::ActorMixer(m) => {
                assert_eq!(m.children.children, vec![2, 3]);
                assert_eq!(m.size, full_size);
            }
            _ => panic!("expected ActorMixer"),
        }
    }

    #[test]
    fn merge_children_dedups_and_grows_size() {
        let mut base = HircEntry::ActorMixer(actor_mixer(1, vec![10, 20]));
        let incoming = HircEntry::ActorMixer(actor_mixer(1, vec![20, 30]));
        let before_size = match &base {
            HircEntry::ActorMixer(m) => m.size,
            _ => unreachable!(),
        };

        base.merge_children(&incoming);

        match &base {
            HircEntry::ActorMixer(m) => {
                assert_eq!(m.children.children, vec![10, 20, 30]);
                assert_eq!(m.size, before_size + 4);
            }
            _ => panic!("expected ActorMixer"),
        }
    }
}
