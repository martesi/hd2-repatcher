"""
Bank Version 154

Trimmed, phase-scoped port of hd2-audio-modder/wwise_hierarchy_154.py. See
`wwise_hierarchy_140.py`'s module docstring for the general trimming
philosophy (GUI/undo-redo/editing bookkeeping dropped, `set_data` kept in
trimmed form since `import_hierarchy` reaches it) and for the important
upstream filename/docstring-swap gotcha. This file matches upstream's
`wwise_hierarchy_154.py` by the filename-based signal: `BankSourceStruct`/
`TrackInfoStruct` carry `cache_id`, `MusicTrack` and `MusicSegment` both use
a full `BaseParam` (`MusicSegment`'s `parent_id` is derived from
`baseParam.directParentID` and NOT separately serialized here — contrast
v140 where it's a real raw field).

Diffing the real upstream `wwise_hierarchy_140.py`/`_154.py` turned up real
layout differences beyond `BankSourceStruct`'s `cache_id` field: `FxChunk` is
6 bytes here (a single packed `bitVector` byte) vs 7 in v140 (two separate
bytes); `BaseParam` drops v140's `bOverrideAttachmentParams` field entirely;
and `StateGroupState` carries a variable-length `AkPropBundle` list here
instead of v140's fixed 8-byte pair. `BaseParam.from_memory_stream` also has
a latent upstream bug worth preserving faithfully: it assigns the "bypass
all FX" byte to a typo'd `bPypassAll` attribute that `get_data` never reads
back (it reads the correctly-spelled `bBypassAll`, which stays 0) — so this
oracle (like the real tool) always serializes that byte as 0 for v154,
regardless of what was parsed. The Rust port replicates this same bug for
byte-exact parity; see `crates/engine/src/wwise/hierarchy/base_param.rs`.

Also ported: all five container types (`RandomSequenceContainer`,
`ActorMixer`, `SwitchContainer`, `LayerContainer`, `MusicSwitchContainer`),
`WwiseHierarchy_154.get_data`'s dangling-child pruning, and
`GameArchive.load`'s cross-bank children-merge (see `audio_core.py`). One
more real layout divergence found here: `SwitchParam` is 13 bytes (a single
merged `byBitVector` byte) vs v140's 14 bytes (two separate bytes) — see the
`SwitchParam` class below.
"""

import copy
import struct

from audio_util import MemoryStream, assert_equal
import wwise_hierarchy_140


class HircEntry:
    """
    Must Have:
    hierarchy_type - U8
    size - U32
    hierarchy_id - tid
    """

    import_values = ["misc"]

    def __init__(self):
        self.size: int = 0
        self.hierarchy_type: int = 0
        self.hierarchy_id: int = 0
        self.misc: bytearray = bytearray()

        self.soundbanks = []
        self.modified: bool = False

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        entry = HircEntry()
        entry.hierarchy_type = stream.uint8_read()
        entry.size = stream.uint32_read()
        entry.hierarchy_id = stream.uint32_read()
        entry.misc = stream.read(entry.size - 4)
        return entry

    def get_data(self):
        return (
            self.hierarchy_type.to_bytes(1, byteorder="little")
            + self.size.to_bytes(4, byteorder="little")
            + self.hierarchy_id.to_bytes(4, byteorder="little")
            + self.misc
        )

    def get_id(self):
        return self.hierarchy_id


class FxChunk:
    """
    uFxIndex - U8i
    fxId - tid
    bitVector - U8x (packs what v140 keeps as two separate bytes)
    """

    def __init__(self, uFxIndex: int, fxId: int, bitVector: int):
        self.uFxIndex: int = uFxIndex
        self.fxId: int = fxId
        self.bitVector: int = bitVector

    def get_data(self):
        return struct.pack("<BIB", self.uFxIndex, self.fxId, self.bitVector)


class FxChunkMetadata:
    """
    uFxIndex - u8i
    fxId - tid
    bIsShareSet - U8x
    """

    def __init__(self, uFxIndex: int, fxId: int, bIsShareSet: int):
        self.uFxIndex = uFxIndex
        self.fxId = fxId
        self.bIsShareSet = bIsShareSet

    def to_bytes(self):
        return struct.pack("<BIB", self.uFxIndex, self.fxId, self.bIsShareSet)


class PropBundle:
    """
    cProps - u8i
    pIDs[cProps] - cProps * u8i
    pValues[cProps] - cProps * tid / uni
    """

    def __init__(self, cProps: int = 0, pIDs: list[int] = [], pValues: list[bytearray] = []):
        self.cProps = cProps
        self.pIDs = pIDs
        self.pValues = pValues

    @staticmethod
    def from_memory_stream(s: MemoryStream):
        p = PropBundle()
        p.cProps = s.uint8_read()
        p.pIDs = [s.uint8_read() for _ in range(p.cProps)]
        p.pValues = [s.read(4) for _ in range(p.cProps)]
        return p

    def get_data(self):
        assert_equal("# of props != # of prop. IDs", self.cProps, len(self.pIDs))
        assert_equal("# of props != # of prop. values", self.cProps, len(self.pValues))
        b = struct.pack("<B", self.cProps)
        for pID in self.pIDs:
            b += struct.pack("<B", pID)
        for pValue in self.pValues:
            b += struct.pack("<4s", pValue)
        return b


class RangedPropBundle:
    """
    cProps - u8i
    pIDs[cProps] - cProps * u8i
    rangedValues[cProps] - cProps * (uni + uni)
    """

    def __init__(self, cProps: int = 0, pIDs: list[int] = [], rangedValues: list[tuple[float, float]] = []):
        self.cProps = cProps
        self.pIDs = pIDs
        self.rangedValues = rangedValues

    @staticmethod
    def from_memory_stream(s: MemoryStream):
        r = RangedPropBundle()
        r.cProps = s.uint8_read()
        r.pIDs = [s.uint8_read() for _ in range(r.cProps)]
        r.rangedValues = [(s.float_read(), s.float_read()) for _ in range(r.cProps)]
        return r

    def get_data(self):
        assert_equal("# of props != # of prop. IDs", self.cProps, len(self.pIDs))
        assert_equal("# of props != # of prop. values", self.cProps, len(self.rangedValues))
        b = struct.pack("<B", self.cProps)
        for pID in self.pIDs:
            b += struct.pack("<B", pID)
        for rangeValue in self.rangedValues:
            b += struct.pack("<ff", rangeValue[0], rangeValue[1])
        return b


class AuxParams:
    """
    byBitVectorAux - U8x
    auxIDs - 4 * tid if byBitVectorAux & 0b0000_1000
    reflectionAuxBus - tid
    """

    def __init__(self, byBitVectorAux: int = 0, auxIDs: list[int] = [], reflectionAuxBus: int = 0):
        self.byBitVectorAux = byBitVectorAux
        self.has_aux = self.byBitVectorAux & 0b0000_1000
        self.auxIDs = auxIDs
        self.reflectionAuxBus = reflectionAuxBus

    def get_data(self):
        b = struct.pack("<B", self.byBitVectorAux)
        if self.has_aux:
            for auxID in self.auxIDs:
                b += struct.pack("<I", auxID)
        b += struct.pack("<I", self.reflectionAuxBus)
        return b


class AdvSetting:
    """
    byBitVectorAdv U8x
    eVirtualQueueBehavior U8x
    u16MaxNumInstance u16
    eBelowThresholdBehavior U8x
    byBitVectorHDR U8x
    """

    def __init__(
        self,
        byBitVectorAdv: int = 0,
        eVirtualQueueBehavior: int = 0,
        u16MaxNumInstance: int = 0,
        eBelowThresholdBehavior: int = 0,
        byBitVectorHDR: int = 0,
    ):
        self.byBitVectorAdv = byBitVectorAdv
        self.eVirtualQueueBehavior = eVirtualQueueBehavior
        self.u16MaxNumInstance = u16MaxNumInstance
        self.eBelowThresholdBehavior = eBelowThresholdBehavior
        self.byBitVectorHDR = byBitVectorHDR

    def get_data(self):
        return struct.pack(
            "<BBHBB",
            self.byBitVectorAdv,
            self.eVirtualQueueBehavior,
            self.u16MaxNumInstance,
            self.eBelowThresholdBehavior,
            self.byBitVectorHDR,
        )


class StateProp:
    """
    propertyId var (assume 8 bits, can be more)
    accumType U8x
    inDb bool U8x
    """

    def __init__(self, propertyId: int = 0, accumType: int = 0, inDb: int = 0):
        self.propertyId = propertyId
        self.accumType = accumType
        self.inDb = inDb

    def to_bytes(self):
        return struct.pack("<3B", self.propertyId, self.accumType, self.inDb)


class AkPropBundle:
    def __init__(self, pID: int, pValue: float):
        self.pID = pID
        self.pValue = pValue

    def get_data(self):
        return struct.pack("<Hf", self.pID, self.pValue)


class StateGroupState:
    """
    ulStateID tid
    cProps u16
    pProps AkPropBundle[]
    """

    def __init__(self, ulStateID: int = 0, cProps: int = 0, pProps: list[AkPropBundle] = []):
        self.ulStateID = ulStateID
        self.cProps = cProps
        self.pProps = pProps

    def get_data(self):
        b = struct.pack("<IH", self.ulStateID, self.cProps)
        for pProp in self.pProps:
            b += pProp.get_data()
        return b


class StateGroup:
    """
    ulStateGroupID tid
    eStateSyncType U8x
    states StateGroupState[]
    """

    def __init__(
        self,
        ulStateGroupID: int = 0,
        eStateSyncType: int = 0,
        ulNumStates: int = 0,
        states: list[StateGroupState] = [],
    ):
        self.ulStateGroupID = ulStateGroupID
        self.eStateSyncType = eStateSyncType
        self.ulNumStates = ulNumStates
        self.states = states

    def get_data(self):
        assert_equal("# of states != # of states in the array", self.ulNumStates, len(self.states))
        b = struct.pack("<IBB", self.ulStateGroupID, self.eStateSyncType, self.ulNumStates)
        for state in self.states:
            b += state.get_data()
        return b


class StateParams:
    """
    ulNumStatesProps var (assume 8 bits, can be more)
    stateProps ulNumStateProps * sizeof(StateProp)
    ulNumStateGroups var (assume 8 bits, can be more)
    stateGroups
    """

    def __init__(
        self,
        ulNumStateProps: int = 0,
        stateProps: list[StateProp] = [],
        ulNumStateGroups: int = 0,
        stateGroups: list[StateGroup] = [],
    ):
        self.ulNumStateProps = ulNumStateProps
        self.stateProps = stateProps
        self.ulNumStateGroups = ulNumStateGroups
        self.stateGroups = stateGroups

    def get_data(self):
        assert_equal("# of state props != # of state props in the array", self.ulNumStateProps, len(self.stateProps))
        assert_equal(
            "# of state groups != # of state groups in the array", self.ulNumStateGroups, len(self.stateGroups)
        )
        b = struct.pack("<B", self.ulNumStateProps)
        for stateProp in self.stateProps:
            b += stateProp.to_bytes()
        b += struct.pack("<B", self.ulNumStateGroups)
        for stateGroup in self.stateGroups:
            b += stateGroup.get_data()
        return b


class RTPCGraphPoint:
    """
    from f32
    to f32
    interp U32
    """

    def __init__(self, _from: float = 0.0, to: float = 0.0, interp: int = 0):
        self._from = _from
        self.to = to
        self.interp = interp

    def get_data(self):
        return struct.pack("<ffI", self._from, self.to, self.interp)


class RTPC:
    """
    rtpcID tid
    rtpcType U8x
    rtpcAccum U8x
    paramID var (assume 8 bits, can be more)
    rtpcCurveID sid
    eScaling  U8x
    ulSize u16
    rtpcGraphPoints ulSize * sizeof(RTPCGraphPoint)
    """

    def __init__(
        self,
        rtpcID: int = 0,
        rtpcType: int = 0,
        rtpcAccum: int = 0,
        paramID: int = 0,
        rtpcCurveID: int = 0,
        eScaling: int = 0,
        ulSize: int = 0,
        rtpcGraphPoints: list[RTPCGraphPoint] = [],
    ):
        self.rtpcID = rtpcID
        self.rtpcType = rtpcType
        self.rtpcAccum = rtpcAccum
        self.paramID = paramID
        self.rtpcCurveID = rtpcCurveID
        self.eScaling = eScaling
        self.ulSize = ulSize
        self.rtpcGraphPoints = rtpcGraphPoints

    def get_data(self):
        assert_equal("# RTPC graph pts != # of RTPC graph ptr in the aray", self.ulSize, len(self.rtpcGraphPoints))
        b = struct.pack(
            "<IBBBIBH", self.rtpcID, self.rtpcType, self.rtpcAccum, self.paramID, self.rtpcCurveID, self.eScaling, self.ulSize
        )
        for p in self.rtpcGraphPoints:
            b += p.get_data()
        return b


def parse_positioning_params(stream: MemoryStream):
    """
    Keep this algorithm here but the data is not used currently
    """
    head = stream.tell()

    uBitsPositioning = stream.uint8_read()  # U8x
    has_positioning = (uBitsPositioning >> 0) & 1

    has_3d = False
    if has_positioning:
        has_3d = (uBitsPositioning >> 1) & 1

    if has_positioning and has_3d:
        uBits3d = stream.uint8_read()  # U8x type: ignore
        e3DPositionType = (uBitsPositioning >> 5) & 3
        has_automation = e3DPositionType != 0

        if has_automation:
            ePathMode = stream.uint8_read()  # U8x
            TransitionTime = stream.int32_read()  # s32

            ulNumVertices = stream.uint32_read()  # u32
            vertices = [
                (stream.float_read(), stream.float_read(), stream.float_read(), stream.int32_read())
                for _ in range(ulNumVertices)
            ]

            ulNumPlayListItem = stream.uint32_read()  # u32
            playListItems = [(stream.uint32_read(), stream.uint32_read()) for _ in range(ulNumPlayListItem)]

            _3DAutomationParams = [
                (stream.float_read(), stream.float_read(), stream.float_read()) for _ in range(ulNumPlayListItem)
            ]

    tail = stream.tell()

    stream.seek(head)

    return stream.read(tail - head)


class BaseParam:
    def __init__(self):
        self.bIsOverrideParentFx: int = 0
        self.uNumFx: int = 0
        self.bBypassAll = 0
        self.fxChunks: list[FxChunk] = []

        self.bIsOverrideParentMetadata: int = 0
        self.uNumFxMetadata: int = 0
        self.fxChunksMetadata: list[FxChunkMetadata] = []

        self.overrideBusId: int = 0
        self.directParentID: int = 0
        self.byBitVectorA: int = 0

        self.propBundle = PropBundle()

        self.positioningParamData: bytearray = bytearray()

        self.rangePropBundle = RangedPropBundle()

        self.auxParams = AuxParams()

        self.advSetting = AdvSetting()

        self.stateParams = StateParams()

        self.uNumCurves: int = 0
        self.rtpcs: list[RTPC] = []

    @staticmethod
    def from_memory_stream(stream: MemoryStream):
        # [Fx]
        baseParam = BaseParam()

        baseParam.bIsOverrideParentFx = stream.uint8_read()
        baseParam.uNumFx = stream.uint8_read()
        if baseParam.uNumFx > 0:
            # NOTE (kept intentionally, matches upstream): this assigns to
            # `bPypassAll`, not `bBypassAll` — `get_data` below reads the
            # correctly-spelled attribute, which never gets updated here, so
            # it always serializes as 0. See the module docstring.
            baseParam.bPypassAll = stream.uint8_read()
            baseParam.fxChunks = [
                FxChunk(stream.uint8_read(), stream.uint32_read(), stream.uint8_read())
                for _ in range(baseParam.uNumFx)
            ]

        baseParam.bIsOverrideParentMetadata = stream.uint8_read()
        baseParam.uNumFxMetadata = stream.uint8_read()
        if baseParam.uNumFxMetadata > 0:
            baseParam.fxChunksMetadata = [
                FxChunkMetadata(stream.uint8_read(), stream.uint32_read(), stream.uint8_read())
                for _ in range(baseParam.uNumFxMetadata)
            ]

        baseParam.overrideBusId = stream.uint32_read()

        baseParam.directParentID = stream.uint32_read()

        baseParam.byBitVectorA = stream.uint8_read()

        baseParam.propBundle = PropBundle.from_memory_stream(stream)

        baseParam.rangePropBundle = RangedPropBundle.from_memory_stream(stream)

        baseParam.positioningParamData = parse_positioning_params(stream)

        baseParam.auxParams.byBitVectorAux = stream.uint8_read()
        baseParam.auxParams.has_aux = baseParam.auxParams.byBitVectorAux & 0b0000_1000
        if baseParam.auxParams.has_aux:
            baseParam.auxParams.auxIDs = [stream.uint32_read() for _ in range(4)]
        baseParam.auxParams.reflectionAuxBus = stream.uint32_read()

        baseParam.advSetting.byBitVectorAdv = stream.uint8_read()
        baseParam.advSetting.eVirtualQueueBehavior = stream.uint8_read()
        baseParam.advSetting.u16MaxNumInstance = stream.uint16_read()
        baseParam.advSetting.eBelowThresholdBehavior = stream.uint8_read()
        baseParam.advSetting.byBitVectorHDR = stream.uint8_read()

        baseParam.stateParams.ulNumStateProps = stream.uint8_read()
        baseParam.stateParams.stateProps = [
            StateProp(stream.uint8_read(), stream.uint8_read(), stream.uint8_read())
            for _ in range(baseParam.stateParams.ulNumStateProps)
        ]
        baseParam.stateParams.ulNumStateGroups = stream.uint8_read()
        stateGroups: list[StateGroup] = []
        for _ in range(baseParam.stateParams.ulNumStateGroups):
            ulStateGroupID = stream.uint32_read()
            eStateSyncType = stream.uint8_read()
            ulNumStates = stream.uint8_read()
            states: list[StateGroupState] = []
            for _ in range(ulNumStates):
                ulStateID = stream.uint32_read()
                cProps = stream.uint16_read()
                pProps = [AkPropBundle(stream.uint16_read(), stream.float_read()) for _ in range(cProps)]
                states.append(StateGroupState(ulStateID, cProps, pProps))
            stateGroups.append(StateGroup(ulStateGroupID, eStateSyncType, ulNumStates, states))
        baseParam.stateParams.stateGroups = stateGroups

        baseParam.uNumCurves = stream.uint16_read()
        rtpcs: list[RTPC] = []
        for _ in range(baseParam.uNumCurves):
            RTPCID = stream.uint32_read()
            rtpcType = stream.uint8_read()
            rtpcAccum = stream.uint8_read()
            ParamID = stream.uint8_read()
            rtpcCurveID = stream.uint32_read()
            eScaling = stream.uint8_read()
            ulSize = stream.uint16_read()
            RTPCGraphPoints = [
                RTPCGraphPoint(stream.float_read(), stream.float_read(), stream.uint32_read()) for _ in range(ulSize)
            ]
            rtpcs.append(RTPC(RTPCID, rtpcType, rtpcAccum, ParamID, rtpcCurveID, eScaling, ulSize, RTPCGraphPoints))
        baseParam.rtpcs = rtpcs

        return baseParam

    def get_data(self):
        b = struct.pack("<BB", self.bIsOverrideParentFx, self.uNumFx)

        assert_equal("# of FX != # of FX in the array", self.uNumFx, len(self.fxChunks))
        if self.uNumFx > 0:
            b += struct.pack("<B", self.bBypassAll)
            for fxChunk in self.fxChunks:
                b += fxChunk.get_data()

        assert_equal("# of metadata FX != # of metadata FX in the array", self.uNumFxMetadata, len(self.fxChunksMetadata))
        b += struct.pack("<BB", self.bIsOverrideParentMetadata, self.uNumFxMetadata)
        if self.uNumFxMetadata > 0:
            for fxChunkMetadata in self.fxChunksMetadata:
                b += fxChunkMetadata.to_bytes()

        b += struct.pack("<IIB", self.overrideBusId, self.directParentID, self.byBitVectorA)

        b += self.propBundle.get_data()

        b += self.rangePropBundle.get_data()

        b += struct.pack(f"<{len(self.positioningParamData)}s", self.positioningParamData)

        b += self.auxParams.get_data()

        b += self.advSetting.get_data()

        b += self.stateParams.get_data()

        b += struct.pack("<H", self.uNumCurves)
        assert_equal("# of RTPC != # of RTPC in the array", self.uNumCurves, len(self.rtpcs))
        for rtpc in self.rtpcs:
            b += rtpc.get_data()

        return b


class BankSourceStruct:
    """
    plugin_id U32
    stream_type U8x
    source_id tid
    mem_size U32
    bit_flags U8x
    plugin_size U32
    plugin_contents plugin_size
    """

    def __init__(self):
        self.plugin_id: int = 0
        self.stream_type: int = 0
        self.source_id: int = 0
        self.cache_id: int = 0
        self.mem_size: int = 0
        self.bit_flags: int = 0
        self.plugin_size: int = 0
        self.plugin_data: bytearray = bytearray()

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        b = BankSourceStruct()
        b.plugin_id, b.stream_type, b.source_id, b.cache_id, b.mem_size, b.bit_flags = struct.unpack(
            "<IBIIIB", stream.read(18)
        )
        if (b.plugin_id & 0x0F) == 2:
            if b.plugin_id:
                b.plugin_size = stream.uint32_read()
                if b.plugin_size > 0:
                    b.plugin_data = stream.read(b.plugin_size)
        return b

    def get_data(self):
        b = struct.pack(
            "<IBIIIB", self.plugin_id, self.stream_type, self.source_id, self.cache_id, self.mem_size, self.bit_flags
        )
        if (self.plugin_id & 0x0F) == 2:
            if self.plugin_id:
                b += struct.pack("<I", self.plugin_size)
                if self.plugin_size > 0:
                    assert_equal("Plugin size mismatch size of plugin data", self.plugin_size, len(self.plugin_data))
                    b += struct.pack(f"<{len(self.plugin_data)}s", self.plugin_data)
        return b


class Sound(HircEntry):
    # `sources` intentionally excluded (matches upstream: v154 never merges
    # audio-source swaps through this generic path).
    import_values = ["baseParam"]

    def __init__(self):
        super().__init__()
        self.sources: list[BankSourceStruct] = []
        self.baseParam: BaseParam | None = None

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        sound = Sound()

        sound.hierarchy_type = stream.uint8_read()
        sound.size = stream.uint32_read()

        head = stream.tell()

        sound.hierarchy_id = stream.uint32_read()
        sound.sources.append(BankSourceStruct.from_memory_stream(stream))
        sound.baseParam = BaseParam.from_memory_stream(stream)

        tail = stream.tell()

        assert_equal(f"Header size and read data size mismatch for Sound {sound.hierarchy_id}", sound.size, tail - head)

        return sound

    def _pack(self):
        data = struct.pack("<I", self.hierarchy_id)
        data += self.sources[0].get_data()
        data += self.baseParam.get_data()
        return data

    def get_data(self):
        data = self._pack()
        assert_equal(f"Header size and packed data size mismatch for Sound {self.hierarchy_id}", self.size, len(data))
        header = struct.pack("<BI", self.hierarchy_type, self.size)
        return header + data

    def set_data(self, entry=None):
        """Trimmed `Sound.set_data`: v154 only merges `baseParam.propBundle`,
        not the whole `baseParam` object (matches upstream's special-case)."""
        if entry:
            self.baseParam.propBundle = entry.baseParam.propBundle
        self.size = len(self._pack())


class TrackInfoStruct:
    """v154: has `cache_id` (48 bytes total, `<IIIIdddd`)."""

    def __init__(self):
        self.track_id = self.source_id = self.cache_id = self.event_id = 0
        self.play_at = self.begin_trim_offset = self.end_trim_offset = self.source_duration = 0.0

    @classmethod
    def from_bytes(cls, data):
        t = TrackInfoStruct()
        (
            t.track_id,
            t.source_id,
            t.cache_id,
            t.event_id,
            t.play_at,
            t.begin_trim_offset,
            t.end_trim_offset,
            t.source_duration,
        ) = struct.unpack("<IIIIdddd", data)
        return t

    def get_data(self):
        return struct.pack(
            "<IIIIdddd",
            self.track_id,
            self.source_id,
            self.cache_id,
            self.event_id,
            self.play_at,
            self.begin_trim_offset,
            self.end_trim_offset,
            self.source_duration,
        )

    def import_entry(self, other):
        """v154-only: matched-by-id in-place field merge (see `MusicTrack.set_data`)."""
        self.track_id = other.track_id
        self.source_id = other.source_id
        self.cache_id = other.cache_id
        self.event_id = other.event_id
        self.play_at = other.play_at
        self.begin_trim_offset = other.begin_trim_offset
        self.end_trim_offset = other.end_trim_offset
        self.source_duration = other.source_duration


class ClipAutomationStruct:
    """Byte-identical between bank versions."""

    def __init__(self):
        self.clip_index = self.auto_type = 0
        self.graph_points: list[tuple[float, float, int]] = []

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        s = ClipAutomationStruct()
        s.clip_index, s.auto_type, num_graph_points = struct.unpack("<III", stream.read(12))
        s.graph_points = [struct.unpack("<ffI", stream.read(12)) for _ in range(num_graph_points)]
        return s

    def get_data(self):
        return struct.pack("<III", self.clip_index, self.auto_type, len(self.graph_points)) + b"".join(
            [struct.pack("<ffI", p[0], p[1], p[2]) for p in self.graph_points]
        )


class MusicTrack(HircEntry):
    """v154: `num_sources` right after `hierarchy_id` (before `bit_flags`,
    unlike v140), a full `BaseParam` at the tail instead of raw
    `override_bus_id`/`parent_id` fields. `import_values` is much narrower
    than v140's — only `clip_automations` + `baseParam` wholesale-replace;
    `sources`/`bit_flags`/`track_type`/`misc` all survive a merge unchanged.
    `track_info` is NOT wholesale-replaced either: existing entries are
    matched to incoming ones by `source_id`/`event_id` and updated in place
    (see `set_data`)."""

    import_values = ["clip_automations", "baseParam"]

    def __init__(self):
        super().__init__()
        self.bit_flags = 0
        self.sources: list[BankSourceStruct] = []
        self.track_info: list[TrackInfoStruct] = []
        self.clip_automations: list[ClipAutomationStruct] = []
        self.unk1 = b""
        self.baseParam: BaseParam | None = None
        self.track_type = 0

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        entry = MusicTrack()
        entry.hierarchy_type = stream.uint8_read()
        entry.size = stream.uint32_read()
        start_position = stream.tell()
        entry.hierarchy_id = stream.uint32_read()
        num_sources = stream.uint32_read()
        for _ in range(num_sources):
            entry.sources.append(BankSourceStruct.from_memory_stream(stream))
        entry.bit_flags = stream.uint8_read()
        num_track_info = stream.uint32_read()
        for _ in range(num_track_info):
            entry.track_info.append(TrackInfoStruct.from_bytes(stream.read(48)))
        if num_track_info > 0:
            entry.unk1 = stream.read(4)
        num_clip_automations = stream.uint32_read()
        for _ in range(num_clip_automations):
            entry.clip_automations.append(ClipAutomationStruct.from_memory_stream(stream))
        entry.baseParam = BaseParam.from_memory_stream(stream)
        entry.track_type = stream.uint8_read()
        entry.misc = stream.read(entry.size - (stream.tell() - start_position))
        return entry

    def get_data(self):
        sources_bytes = b"".join([s.get_data() for s in self.sources])
        track_bytes = b"".join([t.get_data() for t in self.track_info])
        clip_bytes = b"".join([c.get_data() for c in self.clip_automations])
        payload = (
            sources_bytes
            + self.bit_flags.to_bytes(1, "little")
            + len(self.track_info).to_bytes(4, byteorder="little")
            + track_bytes
            + (self.unk1 if len(self.track_info) > 0 else b"")
            + len(self.clip_automations).to_bytes(4, byteorder="little")
            + clip_bytes
            + self.baseParam.get_data()
            + self.track_type.to_bytes(1, "little")
            + self.misc
        )
        self.size = 8 + len(payload)
        return struct.pack("<BIII", self.hierarchy_type, self.size, self.hierarchy_id, len(self.sources)) + payload

    def set_data(self, entry=None):
        if entry:
            for value in self.import_values:
                try:
                    setattr(self, value, getattr(entry, value))
                except AttributeError:
                    pass
            for track in self.track_info:
                for t in entry.track_info:
                    if track.source_id != 0 and track.source_id == t.source_id:
                        track.import_entry(t)
                        break
                    if track.event_id != 0 and track.event_id == t.event_id:
                        track.import_entry(t)
                        break


class MusicSegment(HircEntry):
    """v154: `bit_flags` + a full `BaseParam` (unlike v140's ad-hoc
    raw-skip shape). `parent_id` is derived from `baseParam.directParentID`
    and is NOT separately stored/serialized, so it's excluded from
    `import_values` here (contrast v140, where it's a real merged field)."""

    import_values = ["tracks", "duration", "markers"]

    def __init__(self):
        super().__init__()
        self.bit_flags = 0
        self.baseParam: BaseParam | None = None
        self.tracks: list[int] = []
        self.meter_info = b""  # 23 bytes, opaque
        self.stingers = b""  # 24n+4 bytes, opaque
        self.duration = 0.0
        self.markers: list[list] = []  # [id, position, name-with-trailing-NUL]

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        entry = MusicSegment()
        entry.hierarchy_type = stream.uint8_read()
        entry.size = stream.uint32_read()
        entry.hierarchy_id = stream.uint32_read()
        entry.bit_flags = stream.uint8_read()
        entry.baseParam = BaseParam.from_memory_stream(stream)
        n = stream.uint32_read()  # number of children (tracks)
        for _ in range(n):
            entry.tracks.append(stream.uint32_read())
        entry.meter_info = stream.read(23)
        n = stream.uint32_read()  # number of stingers
        stream.seek(stream.tell() - 4)
        entry.stingers = stream.read(24 * n + 4)
        entry.duration = struct.unpack("<d", stream.read(8))[0]
        n = stream.uint32_read()  # number of markers
        for _ in range(n):
            marker_id = stream.uint32_read()
            position = struct.unpack("<d", stream.read(8))[0]
            name = []
            temp = b"1"
            while temp != b"\x00":
                temp = stream.read(1)
                name.append(temp)
            entry.markers.append([marker_id, position, b"".join(name)])
        return entry

    def get_data(self):
        return b"".join(
            [
                struct.pack("<BIIB", self.hierarchy_type, self.size, self.hierarchy_id, self.bit_flags),
                self.baseParam.get_data(),
                len(self.tracks).to_bytes(4, byteorder="little"),
                b"".join([x.to_bytes(4, byteorder="little") for x in self.tracks]),
                self.meter_info,
                self.stingers,
                struct.pack("<d", self.duration),
                len(self.markers).to_bytes(4, byteorder="little"),
                b"".join(
                    [b"".join([m[0].to_bytes(4, byteorder="little"), struct.pack("<d", m[1]), m[2]]) for m in self.markers]
                ),
            ]
        )

    def set_data(self, entry=None):
        if entry:
            for value in self.import_values:
                try:
                    setattr(self, value, getattr(entry, value))
                except AttributeError:
                    pass
        self.size = len(self.get_data()) - 5


class ContainerChildren:
    def __init__(self):
        self.children: list[int] = []

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        c = ContainerChildren()
        num_children = stream.uint32_read()
        c.children = [stream.uint32_read() for _ in range(num_children)]
        return c

    def get_data(self):
        b = struct.pack("<I", len(self.children))
        for child in self.children:
            b += struct.pack("<I", child)
        return b


class PlayListSetting:
    def __init__(self):
        self.sLoopCount = self.sLoopModMin = self.sLoopModMax = 0
        self.fTransitionTime = self.fTransitionTimeModMin = self.fTransitionTimeModMax = 0.0
        self.wAvoidReaptCount = 0
        self.eTransitionMode = self.eRandomMode = self.eMode = self.byBitVectorPlayList = 0

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        s = PlayListSetting()
        s.sLoopCount = stream.uint16_read()
        s.sLoopModMin = stream.uint16_read()
        s.sLoopModMax = stream.uint16_read()
        s.fTransitionTime = stream.float_read()
        s.fTransitionTimeModMin = stream.float_read()
        s.fTransitionTimeModMax = stream.float_read()
        s.wAvoidReaptCount = stream.uint16_read()
        s.eTransitionMode = stream.uint8_read()
        s.eRandomMode = stream.uint8_read()
        s.eMode = stream.uint8_read()
        s.byBitVectorPlayList = stream.uint8_read()
        return s

    def get_data(self):
        return struct.pack(
            "<HHHfffHBBBB",
            self.sLoopCount,
            self.sLoopModMin,
            self.sLoopModMax,
            self.fTransitionTime,
            self.fTransitionTimeModMin,
            self.fTransitionTimeModMax,
            self.wAvoidReaptCount,
            self.eTransitionMode,
            self.eRandomMode,
            self.eMode,
            self.byBitVectorPlayList,
        )


class PlayListItem:
    def __init__(self, ulPlayID: int, weight: int):
        self.ulPlayID = ulPlayID
        self.weight = weight

    def get_data(self):
        return struct.pack("<Ii", self.ulPlayID, self.weight)


class RandomSequenceContainer(HircEntry):
    """Byte layout identical to v140's (see `wwise_hierarchy_140.py`); only
    the merge behaviour differs: v154 only touches `baseParam.propBundle` +
    `playListSetting`, leaving `children`/`ulPlayListItem`/`playListItems`
    untouched by a merge (v140 wholesale-replaces all 5)."""

    import_values = ["baseParam", "playListSetting"]

    def __init__(self):
        super().__init__()
        self.baseParam: BaseParam | None = None
        self.children = ContainerChildren()
        self.playListSetting = PlayListSetting()
        self.ulPlayListItem = 0
        self.playListItems: list[PlayListItem] = []

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        cntr = RandomSequenceContainer()
        cntr.hierarchy_type = stream.uint8_read()
        cntr.size = stream.uint32_read()
        head = stream.tell()
        cntr.hierarchy_id = stream.uint32_read()
        cntr.baseParam = BaseParam.from_memory_stream(stream)
        cntr.playListSetting = PlayListSetting.from_memory_stream(stream)
        cntr.children = ContainerChildren.from_memory_stream(stream)
        cntr.ulPlayListItem = stream.uint16_read()
        cntr.playListItems = [
            PlayListItem(stream.uint32_read(), stream.int32_read()) for _ in range(cntr.ulPlayListItem)
        ]
        tail = stream.tell()
        assert_equal(
            f"Header size and read data size mismatch for RandomSequenceContainer {cntr.hierarchy_id}",
            cntr.size,
            tail - head,
        )
        return cntr

    def _pack(self):
        data = struct.pack("<I", self.hierarchy_id)
        data += self.baseParam.get_data()
        data += self.playListSetting.get_data()
        data += self.children.get_data()
        assert_equal(
            "# of playlist item mismatch # of item in the playlist item array",
            self.ulPlayListItem,
            len(self.playListItems),
        )
        data += struct.pack("<H", self.ulPlayListItem)
        for item in self.playListItems:
            data += item.get_data()
        return data

    def get_data(self):
        data = self._pack()
        assert_equal(
            f"Header size and packed data size mismatch for RandomSequenceContainer {self.hierarchy_id}",
            self.size,
            len(data),
        )
        return struct.pack("<BI", self.hierarchy_type, self.size) + data

    def set_data(self, entry=None):
        if entry:
            for value in self.import_values:
                try:
                    if value == "baseParam":
                        self.baseParam.propBundle = entry.baseParam.propBundle
                    else:
                        setattr(self, value, getattr(entry, value))
                except AttributeError:
                    pass
        self.size = len(self._pack())


class LayerContainer(HircEntry):
    """Byte layout identical between bank versions. Never merged via
    `import_hierarchy`/`set_data` — only `from_memory_stream`/`get_data` are
    needed, plus the raw `.children` attribute `GameArchive.load`'s
    cross-bank merge reads directly."""

    def __init__(self):
        super().__init__()
        self.baseParam: BaseParam | None = None
        self.children = ContainerChildren()
        self.layerData = b""

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        c = LayerContainer()
        c.hierarchy_type = stream.uint8_read()
        c.size = stream.uint32_read()
        head = stream.tell()
        c.hierarchy_id = stream.uint32_read()
        c.baseParam = BaseParam.from_memory_stream(stream)
        c.children = ContainerChildren.from_memory_stream(stream)
        c.layerData = stream.read(c.size - (stream.tell() - head))
        tail = stream.tell()
        assert_equal(
            f"Header size and read data size mismatch for LayerContainer {c.hierarchy_id}", c.size, tail - head
        )
        return c

    def _pack(self):
        data = struct.pack("<I", self.hierarchy_id)
        data += self.baseParam.get_data()
        data += self.children.get_data()
        data += self.layerData
        return data

    def get_data(self):
        data = self._pack()
        assert_equal(
            f"Header size and packed data size mismatch for LayerContainer {self.hierarchy_id}", self.size, len(data)
        )
        return struct.pack("<BI", self.hierarchy_type, self.size) + data


class ActorMixer(HircEntry):
    """Byte layout identical between bank versions. Same trimming rationale
    as `LayerContainer`."""

    def __init__(self):
        super().__init__()
        self.baseParam: BaseParam | None = None
        self.children = ContainerChildren()

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        m = ActorMixer()
        m.hierarchy_type = stream.uint8_read()
        m.size = stream.uint32_read()
        head = stream.tell()
        m.hierarchy_id = stream.uint32_read()
        m.baseParam = BaseParam.from_memory_stream(stream)
        m.children = ContainerChildren.from_memory_stream(stream)
        tail = stream.tell()
        assert_equal(f"Header size and read data size mismatch for ActorMixer {m.hierarchy_id}", m.size, tail - head)
        return m

    def _pack(self):
        data = struct.pack("<I", self.hierarchy_id)
        data += self.baseParam.get_data()
        data += self.children.get_data()
        return data

    def get_data(self):
        data = self._pack()
        assert_equal(
            f"Header size and packed data size mismatch for ActorMixer {self.hierarchy_id}", self.size, len(data)
        )
        return struct.pack("<BI", self.hierarchy_type, self.size) + data


class SwitchGroup:
    def __init__(self):
        self.ulSwitchID = 0
        self.nodeList: list[int] = []

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        g = SwitchGroup()
        g.ulSwitchID = stream.uint32_read()
        num_items = stream.uint32_read()
        g.nodeList = [stream.uint32_read() for _ in range(num_items)]
        return g

    def get_data(self):
        b = struct.pack("<II", self.ulSwitchID, len(self.nodeList))
        for node_id in self.nodeList:
            b += struct.pack("<I", node_id)
        return b


class SwitchParam:
    """v154 shape: a single merged `byBitVector` byte (13 bytes total) —
    v140 keeps two separate bytes (`byBitVectorPlayBack`/`byBitVectorMode`,
    14 bytes total), see `wwise_hierarchy_140.py`. A real, confirmed layout
    divergence, not inferred."""

    def __init__(self):
        self.ulNodeID = 0
        self.byBitVector = 0
        self.fadeOutTime = 0
        self.fadeInTime = 0

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        p = SwitchParam()
        p.ulNodeID = stream.uint32_read()
        p.byBitVector = stream.uint8_read()
        p.fadeOutTime = stream.int32_read()
        p.fadeInTime = stream.int32_read()
        return p

    def get_data(self):
        return struct.pack("<IBii", self.ulNodeID, self.byBitVector, self.fadeOutTime, self.fadeInTime)


class MusicSwitchContainer(HircEntry):
    """Byte layout identical between bank versions. Real upstream's
    `get_data` has no size-vs-packed-length assertion — replicated as-is."""

    def __init__(self):
        super().__init__()
        self.baseParam: BaseParam | None = None
        self.children = ContainerChildren()
        self.unused_sections: list[bytes] = [b"", b""]

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        c = MusicSwitchContainer()
        c.hierarchy_type = stream.uint8_read()
        c.size = stream.uint32_read()
        start = stream.tell()
        c.hierarchy_id = stream.uint32_read()
        c.unused_sections[0] = stream.read(1)
        c.baseParam = BaseParam.from_memory_stream(stream)
        c.children = ContainerChildren.from_memory_stream(stream)
        c.unused_sections[1] = stream.read(c.size - (stream.tell() - start))
        return c

    def get_data(self):
        return b"".join(
            [
                struct.pack("<BII", self.hierarchy_type, self.size, self.hierarchy_id),
                self.unused_sections[0],
                self.baseParam.get_data(),
                self.children.get_data(),
                self.unused_sections[1],
            ]
        )


class SwitchContainer(HircEntry):
    """Byte layout identical between bank versions except `switchParms`'
    element type (`SwitchParam`, see above)."""

    def __init__(self):
        super().__init__()
        self.baseParam: BaseParam | None = None
        self.eGroupType = 0
        self.ulGroupID = 0
        self.ulDefaultSwitch = 0
        self.bIsContinuousValidation = 0
        self.children = ContainerChildren()
        self.switchGroups: list[SwitchGroup] = []
        self.switchParms: list[SwitchParam] = []

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        s = SwitchContainer()
        s.hierarchy_type = stream.uint8_read()
        s.size = stream.uint32_read()
        head = stream.tell()
        s.hierarchy_id = stream.uint32_read()
        s.baseParam = BaseParam.from_memory_stream(stream)
        s.eGroupType = stream.uint8_read()
        s.ulGroupID = stream.uint32_read()
        s.ulDefaultSwitch = stream.uint32_read()
        s.bIsContinuousValidation = stream.uint8_read()
        s.children = ContainerChildren.from_memory_stream(stream)
        num_switch_groups = stream.uint32_read()
        s.switchGroups = [SwitchGroup.from_memory_stream(stream) for _ in range(num_switch_groups)]
        num_switch_params = stream.uint32_read()
        s.switchParms = [SwitchParam.from_memory_stream(stream) for _ in range(num_switch_params)]
        tail = stream.tell()
        assert_equal(
            f"Header size and read data size mismatch for SwitchContainer {s.hierarchy_id}", s.size, tail - head
        )
        return s

    def _pack(self):
        data = struct.pack("<I", self.hierarchy_id)
        data += self.baseParam.get_data()
        data += struct.pack(
            "<BIIB", self.eGroupType, self.ulGroupID, self.ulDefaultSwitch, self.bIsContinuousValidation
        )
        data += self.children.get_data()
        data += struct.pack("<I", len(self.switchGroups))
        for group in self.switchGroups:
            data += group.get_data()
        data += struct.pack("<I", len(self.switchParms))
        for param in self.switchParms:
            data += param.get_data()
        return data

    def get_data(self):
        data = self._pack()
        assert_equal(
            f"Header size and packed data size mismatch for SwitchContainer {self.hierarchy_id}", self.size, len(data)
        )
        return struct.pack("<BI", self.hierarchy_type, self.size) + data


class HircEntryFactory:
    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        hierarchy_type = stream.uint8_read()
        stream.seek(stream.tell() - 1)
        if hierarchy_type == 0x02:
            return Sound.from_memory_stream(stream)
        if hierarchy_type == 0x05:
            return RandomSequenceContainer.from_memory_stream(stream)
        if hierarchy_type == 0x06:
            return SwitchContainer.from_memory_stream(stream)
        if hierarchy_type == 0x07:
            return ActorMixer.from_memory_stream(stream)
        if hierarchy_type == 0x09:
            return LayerContainer.from_memory_stream(stream)
        if hierarchy_type == 0x0A:
            return MusicSegment.from_memory_stream(stream)
        if hierarchy_type == 0x0B:
            return MusicTrack.from_memory_stream(stream)
        if hierarchy_type == 0x0C:
            return MusicSwitchContainer.from_memory_stream(stream)
        return HircEntry.from_memory_stream(stream)


class WwiseHierarchy_154:
    def __init__(self, soundbank=None):
        self.entries = {}
        self.soundbank = soundbank

    def load(self, hierarchy_data):
        self.entries.clear()
        reader = MemoryStream()
        reader.write(hierarchy_data)
        reader.seek(0)
        num_items = reader.uint32_read()
        for _ in range(num_items):
            entry = HircEntryFactory.from_memory_stream(reader)
            entry.soundbanks.append(self.soundbank)
            self.entries[entry.get_id()] = entry

    def get_entries(self):
        return self.entries.values()

    def get_sounds(self):
        return [entry for entry in self.entries.values() if isinstance(entry, Sound)]

    def get_music_tracks(self):
        return [entry for entry in self.entries.values() if isinstance(entry, MusicTrack)]

    def has_entry(self, entry_id):
        return entry_id in self.entries

    def get_entry(self, entry_id):
        return self.entries[entry_id]

    def get_data(self):
        """See `wwise_hierarchy_140.py`'s `get_data` for the dangling-child
        pruning rationale — byte-identical algorithm, verified against real
        upstream separately for each version."""
        containers = [e for e in self.entries.values() if isinstance(e, (ActorMixer, SwitchContainer, RandomSequenceContainer, LayerContainer, MusicSwitchContainer))]
        saved = [(c, c.children, c.size) for c in containers]
        for c, children, size in saved:
            pruned = ContainerChildren()
            pruned.children = [child for child in children.children if child in self.entries]
            c.children = pruned
            c.size = size - 4 * (len(children.children) - len(pruned.children))

        arr = [entry.get_data() for entry in self.entries.values()]

        for c, children, size in saved:
            c.children = children
            c.size = size

        return len(arr).to_bytes(4, byteorder="little") + b"".join(arr)

    def import_hierarchy(self, new_hierarchy):
        """Trimmed `WwiseHierarchy_154.import_hierarchy`: v154 merges `Sound`,
        `RandomSequenceContainer`, `MusicTrack`, and `MusicSegment` (a
        strictly wider type filter than v140's), but has NO add-branch —
        an incoming entry whose id isn't already present is silently
        dropped (verified against real upstream; a real asymmetry, not an
        oversight — see `wwise_hierarchy_140.py`)."""
        for entry in new_hierarchy.get_entries():
            if isinstance(entry, (Sound, RandomSequenceContainer, MusicTrack, MusicSegment)):
                if entry.hierarchy_id in self.entries:
                    self.entries[entry.hierarchy_id].set_data(entry)
