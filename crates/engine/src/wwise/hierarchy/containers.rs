//! Port of the HIRC "container" types: `RandomSequenceContainer`,
//! `ActorMixer`, `SwitchContainer`, `LayerContainer`, `MusicSwitchContainer`.
//! None of these five participate in `WwiseHierarchy::import_hierarchy`'s
//! patch-file field-merge except `RandomSequenceContainer` (see its
//! `import_entry`) — the other four are never in either bank version's
//! `import_hierarchy` type filter, confirmed directly against
//! `wwise_hierarchy_140.py`/`_154.py`. Instead, all five participate in two
//! *other* real-upstream merge mechanisms this module ports:
//!
//! - **Cross-bank children union** (`GameArchive.load`, `core.py:818-829`):
//!   when the same `hierarchy_id` appears in more than one bank within a
//!   single archive, the later bank's children get unioned into the first
//!   bank's copy (dedup, `size += 4` per newly-added id) — see
//!   [`HircEntry::merge_children`], called from `archive.rs::load`.
//! - **Dangling-child pruning** (`WwiseHierarchy.get_data`, verified against
//!   both `wwise_hierarchy_140.py`/`_154.py`): before serializing, each of
//!   these five types' child id list is filtered down to ids actually
//!   present in *this bank's own* entry map (a child living only in another
//!   bank of the same archive gets silently dropped from output, `size`
//!   adjusted accordingly) — see [`prune_children`] and each type's
//!   `get_data_pruned`, called from `WwiseHierarchy::get_data`.
//!
//! (Python's `Mod.add_game_archive` has a third, narrower merge — only
//! `ActorMixer`, cross-*archive* rather than cross-bank, `core.py:1901-1918`
//! — that belongs with the rest of `Mod`, not yet ported.)
//!
//! `ContainerChildren`/`PlayListSetting`/`PlayListItem` are byte-identical
//! between bank versions (verified against real upstream), as is every field
//! of `ActorMixer`/`LayerContainer`/`MusicSwitchContainer`/`SwitchContainer`/
//! `SwitchGroup` — the one real divergence is `SwitchParam`: v140 splits a
//! bit-vector byte into `byBitVectorPlayBack`/`byBitVectorMode` (14 bytes
//! total), v154 merges it into one `byBitVector` byte (13 bytes total).

use indexmap::IndexMap;

use crate::memstream::MemoryStream;
use crate::wwise::hierarchy::base_param::BaseParam;
use crate::wwise::hierarchy::{BankVersion, HircEntry};

#[derive(Debug, Clone, Default)]
pub struct ContainerChildren {
    pub children: Vec<u32>,
}

impl ContainerChildren {
    pub fn read(stream: &mut MemoryStream) -> Self {
        let num_children = stream.read_u32();
        let children = (0..num_children).map(|_| stream.read_u32()).collect();
        ContainerChildren { children }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + 4 * self.children.len());
        out.extend_from_slice(&(self.children.len() as u32).to_le_bytes());
        for child in &self.children {
            out.extend_from_slice(&child.to_le_bytes());
        }
        out
    }
}

#[derive(Debug, Clone, Default)]
pub struct PlayListSetting {
    pub loop_count: u16,
    pub loop_mod_min: u16,
    pub loop_mod_max: u16,
    pub transition_time: f32,
    pub transition_time_mod_min: f32,
    pub transition_time_mod_max: f32,
    pub avoid_repeat_count: u16,
    pub transition_mode: u8,
    pub random_mode: u8,
    pub mode: u8,
    pub bit_vector_play_list: u8,
}

impl PlayListSetting {
    pub fn read(stream: &mut MemoryStream) -> Self {
        PlayListSetting {
            loop_count: stream.read_u16(),
            loop_mod_min: stream.read_u16(),
            loop_mod_max: stream.read_u16(),
            transition_time: stream.read_f32(),
            transition_time_mod_min: stream.read_f32(),
            transition_time_mod_max: stream.read_f32(),
            avoid_repeat_count: stream.read_u16(),
            transition_mode: stream.read_u8(),
            random_mode: stream.read_u8(),
            mode: stream.read_u8(),
            bit_vector_play_list: stream.read_u8(),
        }
    }

    pub fn get_data(&self) -> [u8; 24] {
        let mut out = [0u8; 24];
        out[0..2].copy_from_slice(&self.loop_count.to_le_bytes());
        out[2..4].copy_from_slice(&self.loop_mod_min.to_le_bytes());
        out[4..6].copy_from_slice(&self.loop_mod_max.to_le_bytes());
        out[6..10].copy_from_slice(&self.transition_time.to_le_bytes());
        out[10..14].copy_from_slice(&self.transition_time_mod_min.to_le_bytes());
        out[14..18].copy_from_slice(&self.transition_time_mod_max.to_le_bytes());
        out[18..20].copy_from_slice(&self.avoid_repeat_count.to_le_bytes());
        out[20] = self.transition_mode;
        out[21] = self.random_mode;
        out[22] = self.mode;
        out[23] = self.bit_vector_play_list;
        out
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PlayListItem {
    pub play_id: u32,
    pub weight: i32,
}

impl PlayListItem {
    pub fn read(stream: &mut MemoryStream) -> Self {
        PlayListItem { play_id: stream.read_u32(), weight: stream.read_i32() }
    }

    pub fn get_data(&self) -> [u8; 8] {
        let mut out = [0u8; 8];
        out[0..4].copy_from_slice(&self.play_id.to_le_bytes());
        out[4..8].copy_from_slice(&self.weight.to_le_bytes());
        out
    }
}

/// `WwiseHierarchy.get_data()`'s dangling-child pruning, shared by all five
/// container types' `get_data_pruned`. Returns `None` when every child id is
/// present in `entries` (fast path: caller should just use `get_data()`
/// unpruned — behaviorally identical output, no clone needed).
fn prune_children(
    children: &ContainerChildren,
    size: u32,
    entries: &IndexMap<u32, HircEntry>,
) -> Option<(ContainerChildren, u32)> {
    if children.children.iter().all(|c| entries.contains_key(c)) {
        return None;
    }
    let mut new_children = children.children.clone();
    let before = new_children.len();
    new_children.retain(|c| entries.contains_key(c));
    let removed = before - new_children.len();
    Some((ContainerChildren { children: new_children }, size - 4 * removed as u32))
}

/// `RandomSequenceContainer`. Byte layout is identical between bank
/// versions; only the `import_hierarchy` merge behaviour differs (see
/// [`Self::import_entry`]).
#[derive(Debug, Clone)]
pub struct RandomSequenceContainer {
    pub size: u32,
    pub hierarchy_id: u32,
    pub base_param: BaseParam,
    pub play_list_setting: PlayListSetting,
    pub children: ContainerChildren,
    pub play_list_items: Vec<PlayListItem>,
}

impl RandomSequenceContainer {
    /// `hierarchy_type` and `size` are already consumed by the dispatcher
    /// (`hierarchy::parse_entry`).
    pub fn read(stream: &mut MemoryStream, size: u32, version: BankVersion) -> Self {
        let head = stream.tell();

        let hierarchy_id = stream.read_u32();
        let base_param = BaseParam::read(stream, version);
        let play_list_setting = PlayListSetting::read(stream);
        let children = ContainerChildren::read(stream);
        let num_play_list_items = stream.read_u16();
        let play_list_items = (0..num_play_list_items).map(|_| PlayListItem::read(stream)).collect();

        let tail = stream.tell();
        assert_eq!(
            tail - head,
            size as usize,
            "RandomSequenceContainer {hierarchy_id}: header size and read data size mismatch"
        );

        RandomSequenceContainer { size, hierarchy_id, base_param, play_list_setting, children, play_list_items }
    }

    fn pack(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.hierarchy_id.to_le_bytes());
        out.extend_from_slice(&self.base_param.get_data());
        out.extend_from_slice(&self.play_list_setting.get_data());
        out.extend_from_slice(&self.children.get_data());
        out.extend_from_slice(&(self.play_list_items.len() as u16).to_le_bytes());
        for item in &self.play_list_items {
            out.extend_from_slice(&item.get_data());
        }
        out
    }

    pub fn get_data(&self) -> Vec<u8> {
        let data = self.pack();
        assert_eq!(
            data.len(),
            self.size as usize,
            "RandomSequenceContainer {}: header size and packed data size mismatch",
            self.hierarchy_id
        );

        let mut out = Vec::with_capacity(5 + data.len());
        out.push(0x05); // HircType::RandomSequenceContainer
        out.extend_from_slice(&self.size.to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    /// `RandomSequenceContainer.set_data`'s merge, as driven by
    /// `WwiseHierarchy::import_hierarchy`. v140 wholesale-replaces
    /// `base_param`/`children`/`play_list_setting`/`play_list_items`; v154
    /// only replaces `base_param.prop_bundle` (not the whole `base_param`)
    /// and `play_list_setting`, leaving `children`/`play_list_items`
    /// untouched — a real, verified-against-upstream asymmetry, not an
    /// oversight.
    pub fn import_entry(&mut self, other: &RandomSequenceContainer, version: BankVersion) {
        match version {
            BankVersion::V140 => {
                self.base_param = other.base_param.clone();
                self.children = other.children.clone();
                self.play_list_setting = other.play_list_setting.clone();
                self.play_list_items = other.play_list_items.clone();
            }
            BankVersion::V154 => {
                self.base_param.prop_bundle = other.base_param.prop_bundle.clone();
                self.play_list_setting = other.play_list_setting.clone();
            }
        }
        self.size = self.pack().len() as u32;
    }

    /// See [`prune_children`].
    pub fn get_data_pruned(&self, entries: &IndexMap<u32, HircEntry>) -> Vec<u8> {
        match prune_children(&self.children, self.size, entries) {
            None => self.get_data(),
            Some((children, size)) => {
                let mut pruned = self.clone();
                pruned.children = children;
                pruned.size = size;
                pruned.get_data()
            }
        }
    }
}

/// `ActorMixer`. Byte layout is identical between bank versions.
#[derive(Debug, Clone)]
pub struct ActorMixer {
    pub size: u32,
    pub hierarchy_id: u32,
    pub base_param: BaseParam,
    pub children: ContainerChildren,
}

impl ActorMixer {
    /// `hierarchy_type` and `size` are already consumed by the dispatcher.
    pub fn read(stream: &mut MemoryStream, size: u32, version: BankVersion) -> Self {
        let head = stream.tell();

        let hierarchy_id = stream.read_u32();
        let base_param = BaseParam::read(stream, version);
        let children = ContainerChildren::read(stream);

        let tail = stream.tell();
        assert_eq!(
            tail - head,
            size as usize,
            "ActorMixer {hierarchy_id}: header size and read data size mismatch"
        );

        ActorMixer { size, hierarchy_id, base_param, children }
    }

    fn pack(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.hierarchy_id.to_le_bytes());
        out.extend_from_slice(&self.base_param.get_data());
        out.extend_from_slice(&self.children.get_data());
        out
    }

    pub fn get_data(&self) -> Vec<u8> {
        let data = self.pack();
        assert_eq!(
            data.len(),
            self.size as usize,
            "ActorMixer {}: header size and packed data size mismatch",
            self.hierarchy_id
        );

        let mut out = Vec::with_capacity(5 + data.len());
        out.push(0x07); // HircType::ActorMixer
        out.extend_from_slice(&self.size.to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    /// See [`prune_children`].
    pub fn get_data_pruned(&self, entries: &IndexMap<u32, HircEntry>) -> Vec<u8> {
        match prune_children(&self.children, self.size, entries) {
            None => self.get_data(),
            Some((children, size)) => {
                let mut pruned = self.clone();
                pruned.children = children;
                pruned.size = size;
                pruned.get_data()
            }
        }
    }
}

/// `LayerContainer`. Byte layout is identical between bank versions.
#[derive(Debug, Clone)]
pub struct LayerContainer {
    pub size: u32,
    pub hierarchy_id: u32,
    pub base_param: BaseParam,
    pub children: ContainerChildren,
    /// Raw trailing bytes (`ulNumLayers` + per-layer data) — never inspected
    /// by `import_patch`/`write_patch`, kept as an opaque blob like the rest
    /// of this port's opaque-passthrough fields.
    pub layer_data: Vec<u8>,
}

impl LayerContainer {
    pub fn read(stream: &mut MemoryStream, size: u32, version: BankVersion) -> Self {
        let head = stream.tell();

        let hierarchy_id = stream.read_u32();
        let base_param = BaseParam::read(stream, version);
        let children = ContainerChildren::read(stream);
        let consumed = stream.tell() - head;
        let layer_data = stream.read(size as usize - consumed).to_vec();

        let tail = stream.tell();
        assert_eq!(
            tail - head,
            size as usize,
            "LayerContainer {hierarchy_id}: header size and read data size mismatch"
        );

        LayerContainer { size, hierarchy_id, base_param, children, layer_data }
    }

    fn pack(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.hierarchy_id.to_le_bytes());
        out.extend_from_slice(&self.base_param.get_data());
        out.extend_from_slice(&self.children.get_data());
        out.extend_from_slice(&self.layer_data);
        out
    }

    pub fn get_data(&self) -> Vec<u8> {
        let data = self.pack();
        assert_eq!(
            data.len(),
            self.size as usize,
            "LayerContainer {}: header size and packed data size mismatch",
            self.hierarchy_id
        );

        let mut out = Vec::with_capacity(5 + data.len());
        out.push(0x09); // HircType::LayerContainer
        out.extend_from_slice(&self.size.to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    /// See [`prune_children`].
    pub fn get_data_pruned(&self, entries: &IndexMap<u32, HircEntry>) -> Vec<u8> {
        match prune_children(&self.children, self.size, entries) {
            None => self.get_data(),
            Some((children, size)) => {
                let mut pruned = self.clone();
                pruned.children = children;
                pruned.size = size;
                pruned.get_data()
            }
        }
    }
}

/// `SwitchGroup`. Byte-identical between bank versions.
#[derive(Debug, Clone, Default)]
pub struct SwitchGroup {
    pub switch_id: u32,
    pub node_list: Vec<u32>,
}

impl SwitchGroup {
    pub fn read(stream: &mut MemoryStream) -> Self {
        let switch_id = stream.read_u32();
        let num_items = stream.read_u32();
        let node_list = (0..num_items).map(|_| stream.read_u32()).collect();
        SwitchGroup { switch_id, node_list }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + 4 * self.node_list.len());
        out.extend_from_slice(&self.switch_id.to_le_bytes());
        out.extend_from_slice(&(self.node_list.len() as u32).to_le_bytes());
        for node in &self.node_list {
            out.extend_from_slice(&node.to_le_bytes());
        }
        out
    }
}

/// `SwitchParam`. The one real layout divergence in this module: v140 reads
/// two separate bit-vector bytes (`byBitVectorPlayBack`, `byBitVectorMode`,
/// 14 bytes total); v154 reads a single merged `byBitVector` byte (13 bytes
/// total) — confirmed directly against both `wwise_hierarchy_*.py`, not
/// inferred. Modeled at the v140 shape; `bit_vector_mode` is `Some` only for
/// v140 (same `Option`-encodes-version convention as `BankSourceStruct::cache_id`).
#[derive(Debug, Clone, Copy, Default)]
pub struct SwitchParam {
    pub node_id: u32,
    pub bit_vector_playback: u8,
    pub bit_vector_mode: Option<u8>,
    pub fade_out_time: i32,
    pub fade_in_time: i32,
}

impl SwitchParam {
    pub fn read(stream: &mut MemoryStream, version: BankVersion) -> Self {
        let node_id = stream.read_u32();
        let bit_vector_playback = stream.read_u8();
        let bit_vector_mode = match version {
            BankVersion::V140 => Some(stream.read_u8()),
            BankVersion::V154 => None,
        };
        let fade_out_time = stream.read_i32();
        let fade_in_time = stream.read_i32();
        SwitchParam { node_id, bit_vector_playback, bit_vector_mode, fade_out_time, fade_in_time }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(14);
        out.extend_from_slice(&self.node_id.to_le_bytes());
        out.push(self.bit_vector_playback);
        if let Some(mode) = self.bit_vector_mode {
            out.push(mode);
        }
        out.extend_from_slice(&self.fade_out_time.to_le_bytes());
        out.extend_from_slice(&self.fade_in_time.to_le_bytes());
        out
    }
}

/// `SwitchContainer`. Every field is byte-identical between bank versions
/// except `switch_params`' element type (see [`SwitchParam`]).
#[derive(Debug, Clone)]
pub struct SwitchContainer {
    pub size: u32,
    pub hierarchy_id: u32,
    pub base_param: BaseParam,
    pub group_type: u8,
    pub group_id: u32,
    pub default_switch: u32,
    pub is_continuous_validation: u8,
    pub children: ContainerChildren,
    pub switch_groups: Vec<SwitchGroup>,
    pub switch_params: Vec<SwitchParam>,
}

impl SwitchContainer {
    pub fn read(stream: &mut MemoryStream, size: u32, version: BankVersion) -> Self {
        let head = stream.tell();

        let hierarchy_id = stream.read_u32();
        let base_param = BaseParam::read(stream, version);
        let group_type = stream.read_u8();
        let group_id = stream.read_u32();
        let default_switch = stream.read_u32();
        let is_continuous_validation = stream.read_u8();
        let children = ContainerChildren::read(stream);
        let num_switch_groups = stream.read_u32();
        let switch_groups = (0..num_switch_groups).map(|_| SwitchGroup::read(stream)).collect();
        let num_switch_params = stream.read_u32();
        let switch_params = (0..num_switch_params).map(|_| SwitchParam::read(stream, version)).collect();

        let tail = stream.tell();
        assert_eq!(
            tail - head,
            size as usize,
            "SwitchContainer {hierarchy_id}: header size and read data size mismatch"
        );

        SwitchContainer {
            size,
            hierarchy_id,
            base_param,
            group_type,
            group_id,
            default_switch,
            is_continuous_validation,
            children,
            switch_groups,
            switch_params,
        }
    }

    fn pack(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.hierarchy_id.to_le_bytes());
        out.extend_from_slice(&self.base_param.get_data());
        out.push(self.group_type);
        out.extend_from_slice(&self.group_id.to_le_bytes());
        out.extend_from_slice(&self.default_switch.to_le_bytes());
        out.push(self.is_continuous_validation);
        out.extend_from_slice(&self.children.get_data());
        out.extend_from_slice(&(self.switch_groups.len() as u32).to_le_bytes());
        for group in &self.switch_groups {
            out.extend_from_slice(&group.get_data());
        }
        out.extend_from_slice(&(self.switch_params.len() as u32).to_le_bytes());
        for param in &self.switch_params {
            out.extend_from_slice(&param.get_data());
        }
        out
    }

    pub fn get_data(&self) -> Vec<u8> {
        let data = self.pack();
        assert_eq!(
            data.len(),
            self.size as usize,
            "SwitchContainer {}: header size and packed data size mismatch",
            self.hierarchy_id
        );

        let mut out = Vec::with_capacity(5 + data.len());
        out.push(0x06); // HircType::SwitchContainer
        out.extend_from_slice(&self.size.to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    /// See [`prune_children`].
    pub fn get_data_pruned(&self, entries: &IndexMap<u32, HircEntry>) -> Vec<u8> {
        match prune_children(&self.children, self.size, entries) {
            None => self.get_data(),
            Some((children, size)) => {
                let mut pruned = self.clone();
                pruned.children = children;
                pruned.size = size;
                pruned.get_data()
            }
        }
    }
}

/// `MusicSwitchContainer`. Byte layout is identical between bank versions.
/// Unlike the other four container types, upstream's `get_data` has no
/// `size`-vs-packed-length assertion — replicated faithfully (no assert
/// here either), not "fixed", per this port's established policy of
/// preserving real upstream quirks rather than second-guessing them.
#[derive(Debug, Clone)]
pub struct MusicSwitchContainer {
    pub size: u32,
    pub hierarchy_id: u32,
    /// The single unexplained byte upstream reads right after
    /// `hierarchy_id` (`unused_sections[0]`, always exactly 1 byte).
    pub unused_byte: u8,
    pub base_param: BaseParam,
    pub children: ContainerChildren,
    /// Raw trailing bytes (`unused_sections[1]`), opaque like `layer_data`.
    pub unused_tail: Vec<u8>,
}

impl MusicSwitchContainer {
    pub fn read(stream: &mut MemoryStream, size: u32, version: BankVersion) -> Self {
        let start = stream.tell();

        let hierarchy_id = stream.read_u32();
        let unused_byte = stream.read_u8();
        let base_param = BaseParam::read(stream, version);
        let children = ContainerChildren::read(stream);
        let consumed = stream.tell() - start;
        let unused_tail = stream.read(size as usize - consumed).to_vec();

        MusicSwitchContainer { size, hierarchy_id, unused_byte, base_param, children, unused_tail }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(0x0c); // HircType::MusicSwitchContainer
        out.extend_from_slice(&self.size.to_le_bytes());
        out.extend_from_slice(&self.hierarchy_id.to_le_bytes());
        out.push(self.unused_byte);
        out.extend_from_slice(&self.base_param.get_data());
        out.extend_from_slice(&self.children.get_data());
        out.extend_from_slice(&self.unused_tail);
        out
    }

    /// See [`prune_children`]. `size` here isn't otherwise validated (see
    /// the struct doc), but pruning still adjusts it the same way the other
    /// four types do — `WwiseHierarchy.get_data()` treats all five
    /// identically, this type included.
    pub fn get_data_pruned(&self, entries: &IndexMap<u32, HircEntry>) -> Vec<u8> {
        match prune_children(&self.children, self.size, entries) {
            None => self.get_data(),
            Some((children, size)) => {
                let mut pruned = self.clone();
                pruned.children = children;
                pruned.size = size;
                pruned.get_data()
            }
        }
    }
}
