//! Port of the HIRC "container" types. Only `RandomSequenceContainer` is
//! populated so far (needed by phase 3's `import_hierarchy` field-merge
//! scope); `ActorMixer`, `SwitchContainer`, `LayerContainer`, and
//! `MusicSwitchContainer` are added in a later phase once their
//! children-merge behaviour is ported — until then they round-trip as
//! [`super::opaque::OpaqueEntry`], matching every other unhandled HIRC type.
//!
//! `ContainerChildren`/`PlayListSetting`/`PlayListItem` are byte-identical
//! between bank versions (verified against real upstream); only
//! `RandomSequenceContainer`'s `import_hierarchy` merge behaviour differs
//! per version (see [`RandomSequenceContainer::import_entry`]).

use crate::memstream::MemoryStream;
use crate::wwise::hierarchy::base_param::BaseParam;
use crate::wwise::hierarchy::BankVersion;

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
}
