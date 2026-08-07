//! Port of `BankSourceStruct` and `Sound` (`wwise_hierarchy_140.py`/`_154.py`).

use crate::memstream::MemoryStream;
use crate::wwise::hierarchy::base_param::BaseParam;
use crate::wwise::hierarchy::BankVersion;

/// `BankSourceStruct`. v154 inserts a `cache_id: u32` between `source_id`
/// and `mem_size` that v140 doesn't have.
#[derive(Debug, Clone)]
pub struct BankSourceStruct {
    pub plugin_id: u32,
    pub stream_type: u8,
    pub source_id: u32,
    /// `Some` only for v154.
    pub cache_id: Option<u32>,
    pub mem_size: u32,
    pub bit_flags: u8,
    pub plugin_data: Vec<u8>,
}

impl BankSourceStruct {
    pub fn read(stream: &mut MemoryStream, version: BankVersion) -> Self {
        let plugin_id = stream.read_u32();
        let stream_type = stream.read_u8();
        let source_id = stream.read_u32();
        let cache_id = match version {
            BankVersion::V154 => Some(stream.read_u32()),
            BankVersion::V140 => None,
        };
        let mem_size = stream.read_u32();
        let bit_flags = stream.read_u8();

        let mut plugin_data = Vec::new();
        if plugin_id & 0x0F == 2 {
            let plugin_size = stream.read_u32();
            if plugin_size > 0 {
                plugin_data = stream.read(plugin_size as usize).to_vec();
            }
        }

        BankSourceStruct {
            plugin_id,
            stream_type,
            source_id,
            cache_id,
            mem_size,
            bit_flags,
            plugin_data,
        }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(18 + self.plugin_data.len());
        out.extend_from_slice(&self.plugin_id.to_le_bytes());
        out.push(self.stream_type);
        out.extend_from_slice(&self.source_id.to_le_bytes());
        if let Some(cache_id) = self.cache_id {
            out.extend_from_slice(&cache_id.to_le_bytes());
        }
        out.extend_from_slice(&self.mem_size.to_le_bytes());
        out.push(self.bit_flags);

        if self.plugin_id & 0x0F == 2 {
            out.extend_from_slice(&(self.plugin_data.len() as u32).to_le_bytes());
            if !self.plugin_data.is_empty() {
                out.extend_from_slice(&self.plugin_data);
            }
        }
        out
    }
}

/// Port of `Sound`. Always carries exactly one [`BankSourceStruct`] (a `Vec`
/// to mirror Python's `sources: list[BankSourceStruct]`, which the DIDX/DATA
/// regeneration loop iterates uniformly across `Sound` and `MusicTrack`).
#[derive(Debug, Clone)]
pub struct Sound {
    pub size: u32,
    pub hierarchy_id: u32,
    pub sources: Vec<BankSourceStruct>,
    pub base_param: BaseParam,
}

impl Sound {
    /// `hierarchy_type` and `size` are already consumed by the dispatcher
    /// (`hierarchy::parse_entry`), matching the convention established by
    /// `OpaqueEntry::read`.
    pub fn read(stream: &mut MemoryStream, size: u32, version: BankVersion) -> Self {
        let head = stream.tell();

        let hierarchy_id = stream.read_u32();
        let sources = vec![BankSourceStruct::read(stream, version)];
        let base_param = BaseParam::read(stream, version);

        let tail = stream.tell();
        assert_eq!(
            tail - head,
            size as usize,
            "Sound {hierarchy_id}: header size and read data size mismatch"
        );

        Sound { size, hierarchy_id, sources, base_param }
    }

    fn pack(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.hierarchy_id.to_le_bytes());
        out.extend_from_slice(&self.sources[0].get_data());
        out.extend_from_slice(&self.base_param.get_data());
        out
    }

    pub fn get_data(&self) -> Vec<u8> {
        let data = self.pack();
        assert_eq!(
            data.len(),
            self.size as usize,
            "Sound {}: header size and packed data size mismatch",
            self.hierarchy_id
        );

        let mut out = Vec::with_capacity(5 + data.len());
        out.push(0x02); // HircType::Sound
        out.extend_from_slice(&self.size.to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    /// `Sound.set_data`'s merge, as driven by `WwiseHierarchy::import_hierarchy`.
    /// v140 wholesale-replaces `sources`/`base_param`; v154 only replaces
    /// `base_param.prop_bundle` (not the whole `base_param`, and not
    /// `sources` at all — verified against real upstream, matching the same
    /// "propBundle only" pattern `RandomSequenceContainer` uses for v154).
    pub fn import_entry(&mut self, other: &Sound, version: BankVersion) {
        match version {
            BankVersion::V140 => {
                self.sources = other.sources.clone();
                self.base_param = other.base_param.clone();
            }
            BankVersion::V154 => {
                self.base_param.prop_bundle = other.base_param.prop_bundle.clone();
            }
        }
        self.size = self.pack().len() as u32;
    }
}
