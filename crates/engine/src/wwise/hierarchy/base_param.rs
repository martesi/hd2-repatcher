//! Port of `BaseParam` and its sub-structures (`wwise_hierarchy_140.py`/
//! `_154.py`, `PropBundle` through `BaseParam`). Despite the plan doc's
//! initial assumption, this layer is **not** byte-identical between bank
//! versions — diffing the real upstream source turned up three real
//! divergences (see the version branches below) plus a latent upstream bug
//! that must be replicated for byte-exact oracle parity: v154's
//! `from_memory_stream` assigns the "bypass all FX" byte to a typo'd
//! `bPypassAll` attribute that `get_data()` never reads (it reads the
//! correctly-spelled `bBypassAll`, which stays 0), so v154 banks always
//! serialize that byte as 0 regardless of the parsed value. v140 does not
//! have this bug (its field is consistently named `bitsFxBypass`).

use crate::memstream::MemoryStream;
use crate::wwise::hierarchy::BankVersion;

/// `FxChunk`. Layout genuinely differs between versions: v140 keeps
/// `bIsShareSet`/`bIsRendered` as two separate bytes (7 bytes total); v154
/// packs them into a single `bitVector` byte (6 bytes total).
#[derive(Debug, Clone)]
pub struct FxChunk {
    pub fx_index: u8,
    pub fx_id: u32,
    pub flags: FxChunkFlags,
}

#[derive(Debug, Clone)]
pub enum FxChunkFlags {
    V140 { is_share_set: u8, is_rendered: u8 },
    V154 { bit_vector: u8 },
}

impl FxChunk {
    pub fn read(stream: &mut MemoryStream, version: BankVersion) -> Self {
        let fx_index = stream.read_u8();
        let fx_id = stream.read_u32();
        let flags = match version {
            BankVersion::V140 => FxChunkFlags::V140 {
                is_share_set: stream.read_u8(),
                is_rendered: stream.read_u8(),
            },
            BankVersion::V154 => FxChunkFlags::V154 { bit_vector: stream.read_u8() },
        };
        FxChunk { fx_index, fx_id, flags }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(7);
        out.push(self.fx_index);
        out.extend_from_slice(&self.fx_id.to_le_bytes());
        match &self.flags {
            FxChunkFlags::V140 { is_share_set, is_rendered } => {
                out.push(*is_share_set);
                out.push(*is_rendered);
            }
            FxChunkFlags::V154 { bit_vector } => out.push(*bit_vector),
        }
        out
    }
}

/// `FxChunkMetadata`: byte-identical between versions.
#[derive(Debug, Clone)]
pub struct FxChunkMetadata {
    pub fx_index: u8,
    pub fx_id: u32,
    pub is_share_set: u8,
}

impl FxChunkMetadata {
    pub fn read(stream: &mut MemoryStream) -> Self {
        FxChunkMetadata {
            fx_index: stream.read_u8(),
            fx_id: stream.read_u32(),
            is_share_set: stream.read_u8(),
        }
    }

    pub fn get_data(&self) -> [u8; 6] {
        let mut out = [0u8; 6];
        out[0] = self.fx_index;
        out[1..5].copy_from_slice(&self.fx_id.to_le_bytes());
        out[5] = self.is_share_set;
        out
    }
}

/// `PropBundle`: byte-identical between versions. Property values are kept
/// as raw 4-byte blobs (matching Python's `bytearray` storage) since their
/// interpretation (float/int/...) depends on the property id, which this
/// engine never needs to inspect.
#[derive(Debug, Clone, Default)]
pub struct PropBundle {
    pub ids: Vec<u8>,
    pub values: Vec<[u8; 4]>,
}

impl PropBundle {
    pub fn read(stream: &mut MemoryStream) -> Self {
        let count = stream.read_u8();
        let ids: Vec<u8> = (0..count).map(|_| stream.read_u8()).collect();
        let values: Vec<[u8; 4]> = (0..count).map(|_| stream.read(4).try_into().unwrap()).collect();
        PropBundle { ids, values }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + self.ids.len() + 4 * self.values.len());
        out.push(self.ids.len() as u8);
        out.extend_from_slice(&self.ids);
        for v in &self.values {
            out.extend_from_slice(v);
        }
        out
    }
}

/// `RangedPropBundle`: byte-identical between versions.
#[derive(Debug, Clone, Default)]
pub struct RangedPropBundle {
    pub ids: Vec<u8>,
    pub ranged_values: Vec<(f32, f32)>,
}

impl RangedPropBundle {
    pub fn read(stream: &mut MemoryStream) -> Self {
        let count = stream.read_u8();
        let ids: Vec<u8> = (0..count).map(|_| stream.read_u8()).collect();
        let ranged_values: Vec<(f32, f32)> = (0..count).map(|_| (stream.read_f32(), stream.read_f32())).collect();
        RangedPropBundle { ids, ranged_values }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + self.ids.len() + 8 * self.ranged_values.len());
        out.push(self.ids.len() as u8);
        out.extend_from_slice(&self.ids);
        for (from, to) in &self.ranged_values {
            out.extend_from_slice(&from.to_le_bytes());
            out.extend_from_slice(&to.to_le_bytes());
        }
        out
    }
}

/// `AuxParams`: byte-identical between versions. `has_aux` (bit 3 of
/// `bit_vector_aux`) is derived on demand rather than cached, so it can
/// never drift out of sync with the byte it comes from.
#[derive(Debug, Clone, Default)]
pub struct AuxParams {
    pub bit_vector_aux: u8,
    pub aux_ids: Vec<u32>,
    pub reflection_aux_bus: u32,
}

impl AuxParams {
    pub fn read(stream: &mut MemoryStream) -> Self {
        let bit_vector_aux = stream.read_u8();
        let has_aux = bit_vector_aux & 0b0000_1000 != 0;
        let aux_ids = if has_aux { (0..4).map(|_| stream.read_u32()).collect() } else { Vec::new() };
        let reflection_aux_bus = stream.read_u32();
        AuxParams { bit_vector_aux, aux_ids, reflection_aux_bus }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(5 + 4 * self.aux_ids.len());
        out.push(self.bit_vector_aux);
        for id in &self.aux_ids {
            out.extend_from_slice(&id.to_le_bytes());
        }
        out.extend_from_slice(&self.reflection_aux_bus.to_le_bytes());
        out
    }
}

/// `AdvSetting`: byte-identical between versions, fixed 6 bytes.
#[derive(Debug, Clone, Default)]
pub struct AdvSetting {
    pub bit_vector_adv: u8,
    pub virtual_queue_behavior: u8,
    pub max_num_instance: u16,
    pub below_threshold_behavior: u8,
    pub bit_vector_hdr: u8,
}

impl AdvSetting {
    pub fn read(stream: &mut MemoryStream) -> Self {
        AdvSetting {
            bit_vector_adv: stream.read_u8(),
            virtual_queue_behavior: stream.read_u8(),
            max_num_instance: stream.read_u16(),
            below_threshold_behavior: stream.read_u8(),
            bit_vector_hdr: stream.read_u8(),
        }
    }

    pub fn get_data(&self) -> [u8; 6] {
        let mut out = [0u8; 6];
        out[0] = self.bit_vector_adv;
        out[1] = self.virtual_queue_behavior;
        out[2..4].copy_from_slice(&self.max_num_instance.to_le_bytes());
        out[4] = self.below_threshold_behavior;
        out[5] = self.bit_vector_hdr;
        out
    }
}

/// `StateProp`: byte-identical between versions, fixed 3 bytes.
#[derive(Debug, Clone, Copy)]
pub struct StateProp {
    pub property_id: u8,
    pub accum_type: u8,
    pub in_db: u8,
}

impl StateProp {
    pub fn read(stream: &mut MemoryStream) -> Self {
        StateProp {
            property_id: stream.read_u8(),
            accum_type: stream.read_u8(),
            in_db: stream.read_u8(),
        }
    }

    pub fn get_data(&self) -> [u8; 3] {
        [self.property_id, self.accum_type, self.in_db]
    }
}

/// `AkPropBundle`: only exists in v154's `StateGroupState`.
#[derive(Debug, Clone, Copy)]
pub struct AkPropBundle {
    pub id: u16,
    pub value: f32,
}

impl AkPropBundle {
    pub fn read(stream: &mut MemoryStream) -> Self {
        AkPropBundle { id: stream.read_u16(), value: stream.read_f32() }
    }

    pub fn get_data(&self) -> [u8; 6] {
        let mut out = [0u8; 6];
        out[0..2].copy_from_slice(&self.id.to_le_bytes());
        out[2..6].copy_from_slice(&self.value.to_le_bytes());
        out
    }
}

/// `StateGroupState`. Genuinely different shape between versions: v140 is a
/// fixed 8-byte `(state_id, state_instance_id)` pair; v154 replaces the
/// second field with a count-prefixed list of `AkPropBundle`.
#[derive(Debug, Clone)]
pub enum StateGroupState {
    V140 { state_id: u32, state_instance_id: u32 },
    V154 { state_id: u32, props: Vec<AkPropBundle> },
}

impl StateGroupState {
    pub fn read(stream: &mut MemoryStream, version: BankVersion) -> Self {
        match version {
            BankVersion::V140 => StateGroupState::V140 {
                state_id: stream.read_u32(),
                state_instance_id: stream.read_u32(),
            },
            BankVersion::V154 => {
                let state_id = stream.read_u32();
                let count = stream.read_u16();
                let props = (0..count).map(|_| AkPropBundle::read(stream)).collect();
                StateGroupState::V154 { state_id, props }
            }
        }
    }

    pub fn get_data(&self) -> Vec<u8> {
        match self {
            StateGroupState::V140 { state_id, state_instance_id } => {
                let mut out = Vec::with_capacity(8);
                out.extend_from_slice(&state_id.to_le_bytes());
                out.extend_from_slice(&state_instance_id.to_le_bytes());
                out
            }
            StateGroupState::V154 { state_id, props } => {
                let mut out = Vec::with_capacity(6 + 6 * props.len());
                out.extend_from_slice(&state_id.to_le_bytes());
                out.extend_from_slice(&(props.len() as u16).to_le_bytes());
                for p in props {
                    out.extend_from_slice(&p.get_data());
                }
                out
            }
        }
    }
}

/// `StateGroup`: container shape is identical between versions; only its
/// `StateGroupState` elements differ (see above).
#[derive(Debug, Clone)]
pub struct StateGroup {
    pub state_group_id: u32,
    pub state_sync_type: u8,
    pub states: Vec<StateGroupState>,
}

impl StateGroup {
    pub fn read(stream: &mut MemoryStream, version: BankVersion) -> Self {
        let state_group_id = stream.read_u32();
        let state_sync_type = stream.read_u8();
        let num_states = stream.read_u8();
        let states = (0..num_states).map(|_| StateGroupState::read(stream, version)).collect();
        StateGroup { state_group_id, state_sync_type, states }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.state_group_id.to_le_bytes());
        out.push(self.state_sync_type);
        out.push(self.states.len() as u8);
        for s in &self.states {
            out.extend_from_slice(&s.get_data());
        }
        out
    }
}

/// `StateParams`: identical shape between versions (only `StateGroup`'s
/// nested `StateGroupState` differs, see above).
#[derive(Debug, Clone, Default)]
pub struct StateParams {
    pub state_props: Vec<StateProp>,
    pub state_groups: Vec<StateGroup>,
}

impl StateParams {
    pub fn read(stream: &mut MemoryStream, version: BankVersion) -> Self {
        let num_state_props = stream.read_u8();
        let state_props = (0..num_state_props).map(|_| StateProp::read(stream)).collect();
        let num_state_groups = stream.read_u8();
        let state_groups = (0..num_state_groups).map(|_| StateGroup::read(stream, version)).collect();
        StateParams { state_props, state_groups }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(self.state_props.len() as u8);
        for p in &self.state_props {
            out.extend_from_slice(&p.get_data());
        }
        out.push(self.state_groups.len() as u8);
        for g in &self.state_groups {
            out.extend_from_slice(&g.get_data());
        }
        out
    }
}

/// `RTPCGraphPoint`: byte-identical between versions, fixed 12 bytes.
#[derive(Debug, Clone, Copy)]
pub struct RTPCGraphPoint {
    pub from: f32,
    pub to: f32,
    pub interp: u32,
}

impl RTPCGraphPoint {
    pub fn read(stream: &mut MemoryStream) -> Self {
        RTPCGraphPoint {
            from: stream.read_f32(),
            to: stream.read_f32(),
            interp: stream.read_u32(),
        }
    }

    pub fn get_data(&self) -> [u8; 12] {
        let mut out = [0u8; 12];
        out[0..4].copy_from_slice(&self.from.to_le_bytes());
        out[4..8].copy_from_slice(&self.to.to_le_bytes());
        out[8..12].copy_from_slice(&self.interp.to_le_bytes());
        out
    }
}

/// `RTPC`: byte-identical between versions.
#[derive(Debug, Clone)]
pub struct RTPC {
    pub rtpc_id: u32,
    pub rtpc_type: u8,
    pub rtpc_accum: u8,
    pub param_id: u8,
    pub rtpc_curve_id: u32,
    pub scaling: u8,
    pub graph_points: Vec<RTPCGraphPoint>,
}

impl RTPC {
    pub fn read(stream: &mut MemoryStream) -> Self {
        let rtpc_id = stream.read_u32();
        let rtpc_type = stream.read_u8();
        let rtpc_accum = stream.read_u8();
        let param_id = stream.read_u8();
        let rtpc_curve_id = stream.read_u32();
        let scaling = stream.read_u8();
        let size = stream.read_u16();
        let graph_points = (0..size).map(|_| RTPCGraphPoint::read(stream)).collect();
        RTPC {
            rtpc_id,
            rtpc_type,
            rtpc_accum,
            param_id,
            rtpc_curve_id,
            scaling,
            graph_points,
        }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(14 + 12 * self.graph_points.len());
        out.extend_from_slice(&self.rtpc_id.to_le_bytes());
        out.push(self.rtpc_type);
        out.push(self.rtpc_accum);
        out.push(self.param_id);
        out.extend_from_slice(&self.rtpc_curve_id.to_le_bytes());
        out.push(self.scaling);
        out.extend_from_slice(&(self.graph_points.len() as u16).to_le_bytes());
        for p in &self.graph_points {
            out.extend_from_slice(&p.get_data());
        }
        out
    }
}

/// `parse_positioning_params`: walks the variable-length positioning
/// structure purely to find its length, then returns the raw byte span
/// unparsed (matching Python, which discards every value it just read and
/// re-reads the span as opaque bytes). Byte-identical between versions.
///
/// Note `e_3d_position_type` is derived from `bits_positioning` (the first
/// byte read), not `bits_3d` (the second) — easy to mis-transcribe given
/// their proximity, but that's what the upstream source does.
fn parse_positioning_params(stream: &mut MemoryStream) -> Vec<u8> {
    let head = stream.tell();

    let bits_positioning = stream.read_u8();
    let has_positioning = bits_positioning & 1 != 0;
    let mut has_3d = false;
    if has_positioning {
        has_3d = (bits_positioning >> 1) & 1 != 0;
    }

    if has_positioning && has_3d {
        let _bits_3d = stream.read_u8();
        let e_3d_position_type = (bits_positioning >> 5) & 3;
        let has_automation = e_3d_position_type != 0;

        if has_automation {
            let _path_mode = stream.read_u8();
            let _transition_time = stream.read_i32();

            let num_vertices = stream.read_u32();
            for _ in 0..num_vertices {
                stream.read_f32();
                stream.read_f32();
                stream.read_f32();
                stream.read_i32();
            }

            let num_playlist_items = stream.read_u32();
            for _ in 0..num_playlist_items {
                stream.read_u32();
                stream.read_u32();
            }
            for _ in 0..num_playlist_items {
                stream.read_f32();
                stream.read_f32();
                stream.read_f32();
            }
        }
    }

    let tail = stream.tell();
    stream.seek(head);
    stream.read(tail - head).to_vec()
}

/// `BaseParam`. Stores its own [`BankVersion`] since [`Self::get_data`]
/// needs to branch on it (both for the `override_attachment_params`
/// presence and the v154 bypass-byte bug — see the module docs).
#[derive(Debug, Clone)]
pub struct BaseParam {
    pub version: BankVersion,
    pub is_override_parent_fx: u8,
    /// The "bypass all FX" byte. Only meaningful (and only present on the
    /// wire) when `fx_chunks` is non-empty; for v154 this is always written
    /// back as 0 on `get_data()` regardless of the parsed value — see the
    /// module docs for why.
    pub bypass_all: u8,
    pub fx_chunks: Vec<FxChunk>,
    pub is_override_parent_metadata: u8,
    pub fx_chunks_metadata: Vec<FxChunkMetadata>,
    /// Present only for v140; v154 dropped this field entirely.
    pub override_attachment_params: Option<u8>,
    pub override_bus_id: u32,
    pub direct_parent_id: u32,
    pub by_bit_vector_a: u8,
    pub prop_bundle: PropBundle,
    pub ranged_prop_bundle: RangedPropBundle,
    pub positioning_param_data: Vec<u8>,
    pub aux_params: AuxParams,
    pub adv_setting: AdvSetting,
    pub state_params: StateParams,
    pub rtpcs: Vec<RTPC>,
}

impl BaseParam {
    pub fn read(stream: &mut MemoryStream, version: BankVersion) -> Self {
        let is_override_parent_fx = stream.read_u8();
        let num_fx = stream.read_u8();
        let mut bypass_all = 0u8;
        let mut fx_chunks = Vec::new();
        if num_fx > 0 {
            let raw_bypass_byte = stream.read_u8();
            // v154's from_memory_stream assigns this to a typo'd attribute
            // that get_data() never reads back (see module docs) — discard
            // it for v154 so our write path naturally reproduces that bug.
            if version == BankVersion::V140 {
                bypass_all = raw_bypass_byte;
            }
            fx_chunks = (0..num_fx).map(|_| FxChunk::read(stream, version)).collect();
        }

        let is_override_parent_metadata = stream.read_u8();
        let num_fx_metadata = stream.read_u8();
        let fx_chunks_metadata = (0..num_fx_metadata).map(|_| FxChunkMetadata::read(stream)).collect();

        let override_attachment_params = match version {
            BankVersion::V140 => Some(stream.read_u8()),
            BankVersion::V154 => None,
        };

        let override_bus_id = stream.read_u32();
        let direct_parent_id = stream.read_u32();
        let by_bit_vector_a = stream.read_u8();

        let prop_bundle = PropBundle::read(stream);
        let ranged_prop_bundle = RangedPropBundle::read(stream);
        let positioning_param_data = parse_positioning_params(stream);
        let aux_params = AuxParams::read(stream);
        let adv_setting = AdvSetting::read(stream);
        let state_params = StateParams::read(stream, version);

        let num_curves = stream.read_u16();
        let rtpcs = (0..num_curves).map(|_| RTPC::read(stream)).collect();

        BaseParam {
            version,
            is_override_parent_fx,
            bypass_all,
            fx_chunks,
            is_override_parent_metadata,
            fx_chunks_metadata,
            override_attachment_params,
            override_bus_id,
            direct_parent_id,
            by_bit_vector_a,
            prop_bundle,
            ranged_prop_bundle,
            positioning_param_data,
            aux_params,
            adv_setting,
            state_params,
            rtpcs,
        }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(self.is_override_parent_fx);
        out.push(self.fx_chunks.len() as u8);
        if !self.fx_chunks.is_empty() {
            match self.version {
                BankVersion::V140 => out.push(self.bypass_all),
                BankVersion::V154 => out.push(0),
            }
            for fx in &self.fx_chunks {
                out.extend_from_slice(&fx.get_data());
            }
        }

        out.push(self.is_override_parent_metadata);
        out.push(self.fx_chunks_metadata.len() as u8);
        for meta in &self.fx_chunks_metadata {
            out.extend_from_slice(&meta.get_data());
        }

        if let Some(b) = self.override_attachment_params {
            out.push(b);
        }

        out.extend_from_slice(&self.override_bus_id.to_le_bytes());
        out.extend_from_slice(&self.direct_parent_id.to_le_bytes());
        out.push(self.by_bit_vector_a);

        out.extend_from_slice(&self.prop_bundle.get_data());
        out.extend_from_slice(&self.ranged_prop_bundle.get_data());
        out.extend_from_slice(&self.positioning_param_data);
        out.extend_from_slice(&self.aux_params.get_data());
        out.extend_from_slice(&self.adv_setting.get_data());
        out.extend_from_slice(&self.state_params.get_data());

        out.extend_from_slice(&(self.rtpcs.len() as u16).to_le_bytes());
        for rtpc in &self.rtpcs {
            out.extend_from_slice(&rtpc.get_data());
        }

        out
    }
}
