//! Port of `MusicTrack` (`wwise_hierarchy_140.py`/`_154.py`) and its
//! `TrackInfoStruct`/`ClipAutomationStruct` dependencies. The two bank
//! versions diverge more here than for any other structured HIRC type
//! (field order, `BaseParam` presence, header packing) — modeled as two
//! fully separate read/write code paths behind one enum rather than forcing
//! a shared shape, per the plan doc's own advice for this type.

use crate::memstream::MemoryStream;
use crate::wwise::hierarchy::base_param::BaseParam;
use crate::wwise::hierarchy::{BankSourceStruct, BankVersion};

/// `TrackInfoStruct`. v154 inserts a `cache_id: u32` between `source_id`
/// and `event_id` that v140 doesn't have (44 vs 48 bytes total).
#[derive(Debug, Clone)]
pub struct TrackInfoStruct {
    pub track_id: u32,
    pub source_id: u32,
    /// `Some` only for v154.
    pub cache_id: Option<u32>,
    pub event_id: u32,
    pub play_at: f64,
    pub begin_trim_offset: f64,
    pub end_trim_offset: f64,
    pub source_duration: f64,
}

impl TrackInfoStruct {
    pub fn read(stream: &mut MemoryStream, version: BankVersion) -> Self {
        let track_id = stream.read_u32();
        let source_id = stream.read_u32();
        let cache_id = match version {
            BankVersion::V154 => Some(stream.read_u32()),
            BankVersion::V140 => None,
        };
        let event_id = stream.read_u32();
        let play_at = stream.read_f64();
        let begin_trim_offset = stream.read_f64();
        let end_trim_offset = stream.read_f64();
        let source_duration = stream.read_f64();
        TrackInfoStruct {
            track_id,
            source_id,
            cache_id,
            event_id,
            play_at,
            begin_trim_offset,
            end_trim_offset,
            source_duration,
        }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(48);
        out.extend_from_slice(&self.track_id.to_le_bytes());
        out.extend_from_slice(&self.source_id.to_le_bytes());
        if let Some(cache_id) = self.cache_id {
            out.extend_from_slice(&cache_id.to_le_bytes());
        }
        out.extend_from_slice(&self.event_id.to_le_bytes());
        out.extend_from_slice(&self.play_at.to_le_bytes());
        out.extend_from_slice(&self.begin_trim_offset.to_le_bytes());
        out.extend_from_slice(&self.end_trim_offset.to_le_bytes());
        out.extend_from_slice(&self.source_duration.to_le_bytes());
        out
    }

    /// v154-only matched-by-id in-place merge (see [`super::music_track::MusicTrack::import_entry`]).
    pub fn import_entry(&mut self, other: &TrackInfoStruct) {
        self.track_id = other.track_id;
        self.source_id = other.source_id;
        self.cache_id = other.cache_id;
        self.event_id = other.event_id;
        self.play_at = other.play_at;
        self.begin_trim_offset = other.begin_trim_offset;
        self.end_trim_offset = other.end_trim_offset;
        self.source_duration = other.source_duration;
    }
}

/// `ClipAutomationStruct`. Byte-identical between versions. Python stores a
/// separate `num_graph_points` count alongside the list, but the two never
/// diverge in practice, so this recomputes the count from `graph_points.len()`
/// on write instead (same style already used for e.g. `PropBundle`).
#[derive(Debug, Clone, Default)]
pub struct ClipAutomationStruct {
    pub clip_index: u32,
    pub auto_type: u32,
    pub graph_points: Vec<(f32, f32, u32)>,
}

impl ClipAutomationStruct {
    pub fn read(stream: &mut MemoryStream) -> Self {
        let clip_index = stream.read_u32();
        let auto_type = stream.read_u32();
        let num_graph_points = stream.read_u32();
        let graph_points =
            (0..num_graph_points).map(|_| (stream.read_f32(), stream.read_f32(), stream.read_u32())).collect();
        ClipAutomationStruct { clip_index, auto_type, graph_points }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + 12 * self.graph_points.len());
        out.extend_from_slice(&self.clip_index.to_le_bytes());
        out.extend_from_slice(&self.auto_type.to_le_bytes());
        out.extend_from_slice(&(self.graph_points.len() as u32).to_le_bytes());
        for (from, to, interp) in &self.graph_points {
            out.extend_from_slice(&from.to_le_bytes());
            out.extend_from_slice(&to.to_le_bytes());
            out.extend_from_slice(&interp.to_le_bytes());
        }
        out
    }
}

/// Version-specific body. Genuinely two shapes: field order (`bit_flags`
/// placement relative to `sources`), `BaseParam` presence, and header
/// packing all differ.
#[derive(Debug, Clone)]
pub enum MusicTrackBody {
    /// v140: `bit_flags` right after `hierarchy_id` (before `sources`), no
    /// `BaseParam` — raw `override_bus_id`/`parent_id` fields instead.
    /// `override_bus_id` is read but NOT merged by `import_hierarchy`
    /// (survives unchanged) — verified against real upstream.
    V140 { bit_flags: u8, unused_0: Vec<u8>, unused_1: Vec<u8>, override_bus_id: u32, parent_id: u32 },
    /// v154: `bit_flags` after `sources`, a full `BaseParam` at the tail.
    /// `base_param` IS wholesale-replaced by `import_hierarchy` (along with
    /// `clip_automations`); `bit_flags`/`unk1`/`track_type` are not.
    V154 { bit_flags: u8, unk1: Vec<u8>, base_param: BaseParam, track_type: u8 },
}

#[derive(Debug, Clone)]
pub struct MusicTrack {
    pub size: u32,
    pub hierarchy_id: u32,
    pub sources: Vec<BankSourceStruct>,
    pub track_info: Vec<TrackInfoStruct>,
    pub clip_automations: Vec<ClipAutomationStruct>,
    pub misc: Vec<u8>,
    pub body: MusicTrackBody,
}

impl MusicTrack {
    /// `hierarchy_type` and `size` are already consumed by the dispatcher
    /// (`hierarchy::parse_entry`).
    pub fn read(stream: &mut MemoryStream, size: u32, version: BankVersion) -> Self {
        let start = stream.tell();
        let hierarchy_id = stream.read_u32();
        match version {
            BankVersion::V140 => {
                let bit_flags = stream.read_u8();
                let num_sources = stream.read_u32();
                let sources = (0..num_sources).map(|_| BankSourceStruct::read(stream, version)).collect();
                let num_track_info = stream.read_u32();
                let track_info = (0..num_track_info).map(|_| TrackInfoStruct::read(stream, version)).collect();
                let unused_0 = stream.read(4).to_vec();
                let num_clips = stream.read_u32();
                let clip_automations = (0..num_clips).map(|_| ClipAutomationStruct::read(stream)).collect();
                let unused_1 = stream.read(5).to_vec();
                let override_bus_id = stream.read_u32();
                let parent_id = stream.read_u32();
                let misc = stream.read(size as usize - (stream.tell() - start)).to_vec();
                MusicTrack {
                    size,
                    hierarchy_id,
                    sources,
                    track_info,
                    clip_automations,
                    misc,
                    body: MusicTrackBody::V140 { bit_flags, unused_0, unused_1, override_bus_id, parent_id },
                }
            }
            BankVersion::V154 => {
                let num_sources = stream.read_u32();
                let sources = (0..num_sources).map(|_| BankSourceStruct::read(stream, version)).collect();
                let bit_flags = stream.read_u8();
                let num_track_info = stream.read_u32();
                let track_info = (0..num_track_info).map(|_| TrackInfoStruct::read(stream, version)).collect();
                let unk1 = if num_track_info > 0 { stream.read(4).to_vec() } else { Vec::new() };
                let num_clips = stream.read_u32();
                let clip_automations = (0..num_clips).map(|_| ClipAutomationStruct::read(stream)).collect();
                let base_param = BaseParam::read(stream, version);
                let track_type = stream.read_u8();
                let misc = stream.read(size as usize - (stream.tell() - start)).to_vec();
                MusicTrack {
                    size,
                    hierarchy_id,
                    sources,
                    track_info,
                    clip_automations,
                    misc,
                    body: MusicTrackBody::V154 { bit_flags, unk1, base_param, track_type },
                }
            }
        }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let sources_bytes: Vec<u8> = self.sources.iter().flat_map(|s| s.get_data()).collect();
        let track_bytes: Vec<u8> = self.track_info.iter().flat_map(|t| t.get_data()).collect();
        let clip_bytes: Vec<u8> = self.clip_automations.iter().flat_map(|c| c.get_data()).collect();

        match &self.body {
            MusicTrackBody::V140 { bit_flags, unused_0, unused_1, override_bus_id, parent_id } => {
                let mut payload = Vec::new();
                payload.extend_from_slice(&sources_bytes);
                payload.extend_from_slice(&(self.track_info.len() as u32).to_le_bytes());
                payload.extend_from_slice(&track_bytes);
                payload.extend_from_slice(unused_0);
                payload.extend_from_slice(&(self.clip_automations.len() as u32).to_le_bytes());
                payload.extend_from_slice(&clip_bytes);
                payload.extend_from_slice(unused_1);
                payload.extend_from_slice(&override_bus_id.to_le_bytes());
                payload.extend_from_slice(&parent_id.to_le_bytes());
                payload.extend_from_slice(&self.misc);

                // Python: `self.size = 9 + len(payload)`, mutated fresh on
                // every get_data() call — mirrored here rather than trusting
                // a possibly-stale `self.size`.
                let size = 9 + payload.len() as u32;
                let mut out = Vec::with_capacity(14 + payload.len());
                out.push(0x0b); // HircType::MusicTrack
                out.extend_from_slice(&size.to_le_bytes());
                out.extend_from_slice(&self.hierarchy_id.to_le_bytes());
                out.push(*bit_flags);
                out.extend_from_slice(&(self.sources.len() as u32).to_le_bytes());
                out.extend_from_slice(&payload);
                out
            }
            MusicTrackBody::V154 { bit_flags, unk1, base_param, track_type } => {
                let mut payload = Vec::new();
                payload.extend_from_slice(&sources_bytes);
                payload.push(*bit_flags);
                payload.extend_from_slice(&(self.track_info.len() as u32).to_le_bytes());
                payload.extend_from_slice(&track_bytes);
                if !self.track_info.is_empty() {
                    payload.extend_from_slice(unk1);
                }
                payload.extend_from_slice(&(self.clip_automations.len() as u32).to_le_bytes());
                payload.extend_from_slice(&clip_bytes);
                payload.extend_from_slice(&base_param.get_data());
                payload.push(*track_type);
                payload.extend_from_slice(&self.misc);

                let size = 8 + payload.len() as u32;
                let mut out = Vec::with_capacity(13 + payload.len());
                out.push(0x0b); // HircType::MusicTrack
                out.extend_from_slice(&size.to_le_bytes());
                out.extend_from_slice(&self.hierarchy_id.to_le_bytes());
                out.extend_from_slice(&(self.sources.len() as u32).to_le_bytes());
                out.extend_from_slice(&payload);
                out
            }
        }
    }

    /// `MusicTrack.set_data`'s merge, as driven by
    /// `WwiseHierarchy::import_hierarchy`. Version-specific — v140 wholesale
    /// replaces most fields (except `override_bus_id`, which upstream never
    /// merges); v154 only replaces `clip_automations`/`base_param`
    /// wholesale, plus a matched-by-id in-place merge of individual
    /// `track_info` entries (not a wholesale list replace).
    pub fn import_entry(&mut self, other: &MusicTrack, version: BankVersion) {
        match version {
            BankVersion::V140 => {
                self.sources = other.sources.clone();
                self.track_info = other.track_info.clone();
                self.clip_automations = other.clip_automations.clone();
                self.misc = other.misc.clone();
                if let (
                    MusicTrackBody::V140 { bit_flags, unused_0, unused_1, parent_id, .. },
                    MusicTrackBody::V140 { bit_flags: ob, unused_0: o0, unused_1: o1, parent_id: op, .. },
                ) = (&mut self.body, &other.body)
                {
                    *bit_flags = *ob;
                    *unused_0 = o0.clone();
                    *unused_1 = o1.clone();
                    *parent_id = *op;
                }
            }
            BankVersion::V154 => {
                self.clip_automations = other.clip_automations.clone();
                if let (MusicTrackBody::V154 { base_param, .. }, MusicTrackBody::V154 { base_param: ob, .. }) =
                    (&mut self.body, &other.body)
                {
                    *base_param = ob.clone();
                }
                for track in &mut self.track_info {
                    let matched = other.track_info.iter().find(|t| {
                        (track.source_id != 0 && track.source_id == t.source_id)
                            || (track.event_id != 0 && track.event_id == t.event_id)
                    });
                    if let Some(t) = matched {
                        track.import_entry(t);
                    }
                }
            }
        }
    }
}
