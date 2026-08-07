//! Port of `MusicSegment` (`wwise_hierarchy_140.py`/`_154.py`). The two bank
//! versions genuinely diverge here: v140 is an ad-hoc raw-skip shape with no
//! `BaseParam` (its `parent_id` is a real, separately-serialized field);
//! v154 reads `bit_flags` + a full `BaseParam` (its `parent_id` is derived
//! from `base_param.direct_parent_id` and is never separately stored). See
//! `reference/wwise_hierarchy_{140,154}.py`'s `MusicSegment` docstrings —
//! and the upstream filename-vs-internal-docstring gotcha documented there
//! — before touching this file; the naming is easy to get backwards.

use crate::memstream::MemoryStream;
use crate::wwise::hierarchy::base_param::BaseParam;
use crate::wwise::hierarchy::BankVersion;

/// A single music segment marker (Python's `[id, position, name]`). `name`
/// is read byte-by-byte until and including a NUL terminator, and stored
/// with that terminator still attached (matching Python's accumulation
/// loop, which appends every byte it reads, including the final `\x00`).
#[derive(Debug, Clone)]
pub struct Marker {
    pub id: u32,
    pub position: f64,
    pub name: Vec<u8>,
}

impl Marker {
    pub fn read(stream: &mut MemoryStream) -> Self {
        let id = stream.read_u32();
        let position = stream.read_f64();
        let mut name = Vec::new();
        loop {
            let b = stream.read(1)[0];
            name.push(b);
            if b == 0 {
                break;
            }
        }
        Marker { id, position, name }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + self.name.len());
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.position.to_le_bytes());
        out.extend_from_slice(&self.name);
        out
    }
}

/// Version-specific header, read/written before `tracks`.
#[derive(Debug, Clone)]
pub enum MusicSegmentHead {
    /// v140: ad hoc raw-skip blobs plus a real, separately-serialized
    /// `parent_id`. All five fields are wholesale-replaced together on an
    /// `import_hierarchy` merge (see [`MusicSegment::import_entry`]).
    V140 {
        unused_0: Vec<u8>,
        parent_id: u32,
        unused_1: Vec<u8>,
        prop_skip_1: Vec<u8>,
        prop_skip_2: Vec<u8>,
    },
    /// v154: `bit_flags` + a full `BaseParam`. Neither is touched by an
    /// `import_hierarchy` merge.
    V154 { bit_flags: u8, base_param: BaseParam },
}

/// Peeks a `u8` prop count, rewinds, and re-reads `5n+1` bytes raw —
/// mirrors `BaseParam`'s `parse_positioning_params` peek-then-reread style.
fn read_prop_skip_1(stream: &mut MemoryStream) -> Vec<u8> {
    let n = stream.read_u8();
    stream.advance(-1);
    stream.read(5 * n as usize + 1).to_vec()
}

/// Same idea as [`read_prop_skip_1`] but for the second, larger skip span.
fn read_prop_skip_2(stream: &mut MemoryStream) -> Vec<u8> {
    let n = stream.read_u8();
    stream.advance(-1);
    stream.read(5 * n as usize + 1 + 12 + 4).to_vec()
}

/// Peeks a `u32` stinger count, rewinds, and re-reads `24n+4` bytes raw.
fn read_stingers(stream: &mut MemoryStream) -> Vec<u8> {
    let n = stream.read_u32();
    stream.advance(-4);
    stream.read(24 * n as usize + 4).to_vec()
}

#[derive(Debug, Clone)]
pub struct MusicSegment {
    pub size: u32,
    pub hierarchy_id: u32,
    pub head: MusicSegmentHead,
    pub tracks: Vec<u32>,
    /// 23 bytes, opaque meter info. Same on-wire position/shape in both
    /// versions; only merged (wholesale, alongside the v140 head) for v140.
    pub meter_info: Vec<u8>,
    /// `24n+4` bytes, opaque stingers. Same merge behaviour as `meter_info`.
    pub stingers: Vec<u8>,
    pub duration: f64,
    pub markers: Vec<Marker>,
}

impl MusicSegment {
    /// `hierarchy_type` and `size` are already consumed by the dispatcher
    /// (`hierarchy::parse_entry`).
    pub fn read(stream: &mut MemoryStream, size: u32, version: BankVersion) -> Self {
        let hierarchy_id = stream.read_u32();
        let head = match version {
            BankVersion::V140 => {
                let unused_0 = stream.read(10).to_vec();
                let parent_id = stream.read_u32();
                let unused_1 = stream.read(1).to_vec();
                let prop_skip_1 = read_prop_skip_1(stream);
                let prop_skip_2 = read_prop_skip_2(stream);
                MusicSegmentHead::V140 { unused_0, parent_id, unused_1, prop_skip_1, prop_skip_2 }
            }
            BankVersion::V154 => {
                let bit_flags = stream.read_u8();
                let base_param = BaseParam::read(stream, version);
                MusicSegmentHead::V154 { bit_flags, base_param }
            }
        };

        let num_tracks = stream.read_u32();
        let tracks = (0..num_tracks).map(|_| stream.read_u32()).collect();
        let meter_info = stream.read(23).to_vec();
        let stingers = read_stingers(stream);
        let duration = stream.read_f64();
        let num_markers = stream.read_u32();
        let markers = (0..num_markers).map(|_| Marker::read(stream)).collect();

        MusicSegment { size, hierarchy_id, head, tracks, meter_info, stingers, duration, markers }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&self.hierarchy_id.to_le_bytes());
        match &self.head {
            MusicSegmentHead::V140 { unused_0, parent_id, unused_1, prop_skip_1, prop_skip_2 } => {
                body.extend_from_slice(unused_0);
                body.extend_from_slice(&parent_id.to_le_bytes());
                body.extend_from_slice(unused_1);
                body.extend_from_slice(prop_skip_1);
                body.extend_from_slice(prop_skip_2);
            }
            MusicSegmentHead::V154 { bit_flags, base_param } => {
                body.push(*bit_flags);
                body.extend_from_slice(&base_param.get_data());
            }
        }
        body.extend_from_slice(&(self.tracks.len() as u32).to_le_bytes());
        for t in &self.tracks {
            body.extend_from_slice(&t.to_le_bytes());
        }
        body.extend_from_slice(&self.meter_info);
        body.extend_from_slice(&self.stingers);
        body.extend_from_slice(&self.duration.to_le_bytes());
        body.extend_from_slice(&(self.markers.len() as u32).to_le_bytes());
        for m in &self.markers {
            body.extend_from_slice(&m.get_data());
        }

        let mut out = Vec::with_capacity(5 + body.len());
        out.push(0x0a); // HircType::MusicSegment
        out.extend_from_slice(&self.size.to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// `MusicSegment.set_data`'s field-copy loop, as driven by
    /// `WwiseHierarchy::import_hierarchy`. Version-specific: v140
    /// wholesale-replaces `parent_id`/`tracks`/`duration`/`meter_info`/
    /// `stingers`/`markers` (all bundled into one Python `unused_sections`
    /// list + separate scalars there); v154 only replaces `tracks`/
    /// `duration`/`markers` — its `parent_id` is baseParam-derived and never
    /// separately merged, and `bit_flags`/`base_param` are excluded from
    /// `import_values` entirely (verified against real upstream).
    pub fn import_entry(&mut self, other: &MusicSegment, version: BankVersion) {
        self.tracks = other.tracks.clone();
        self.duration = other.duration;
        self.markers = other.markers.clone();
        if version == BankVersion::V140 {
            self.meter_info = other.meter_info.clone();
            self.stingers = other.stingers.clone();
            if let (
                MusicSegmentHead::V140 { unused_0, parent_id, unused_1, prop_skip_1, prop_skip_2 },
                MusicSegmentHead::V140 {
                    unused_0: o0,
                    parent_id: op,
                    unused_1: o1,
                    prop_skip_1: ops1,
                    prop_skip_2: ops2,
                },
            ) = (&mut self.head, &other.head)
            {
                *unused_0 = o0.clone();
                *parent_id = *op;
                *unused_1 = o1.clone();
                *prop_skip_1 = ops1.clone();
                *prop_skip_2 = ops2.clone();
            }
        }
        // Python: `self.size = len(self.get_data()) - 5`. Safe to compute
        // from the just-updated fields since the `size` field's *value*
        // never changes get_data()'s output length (always 4 bytes).
        self.size = self.get_data().len() as u32 - 5;
    }
}
