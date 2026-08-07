"""
Bank Version 140

Trimmed, phase-scoped port of hd2-audio-modder/wwise_hierarchy_140.py. Ported
so far: the opaque passthrough path (`HircEntry`), `Sound`/`BankSourceStruct`/
`BaseParam` (and `BaseParam`'s sub-structures), `MusicTrack`, `MusicSegment`,
all five container types (`RandomSequenceContainer`, `ActorMixer`,
`SwitchContainer`, `LayerContainer`, `MusicSwitchContainer`, plus their
shared `ContainerChildren`/`PlayListSetting`/`PlayListItem`/`SwitchGroup`/
`SwitchParam`), and the `WwiseHierarchy_140` container including
`import_hierarchy` and `get_data`'s dangling-child pruning. Trimmed of
GUI/undo-redo bookkeeping not reachable from `import_patch`/`write_patch`
(`revert_modifications`, `add_entry`/`remove_entry`, `modified_children`,
parent back-references, the `PropBundle`/`RangedPropBundle` mutation
helpers, ...) — `set_data` IS kept on the types that need it (trimmed to
just the field-copy loop + size recompute, dropping the
`soundbanks`/`raise_modified` side effects), since `import_hierarchy` calls
it and that path IS reachable from `import_patch` (a prior module docstring
here claimed otherwise; that was wrong). `ActorMixer`/`SwitchContainer`/
`LayerContainer`/`MusicSwitchContainer` don't get `set_data` at all — real
upstream's `import_hierarchy` type filter excludes all four in both bank
versions, confirmed directly against `core.py`, so only
`from_memory_stream`/`get_data` (plus the raw `.children`/`.size` attributes
`GameArchive.load`'s cross-bank merge mutates directly) are needed. Every
other hierarchy entry still round-trips as `(type, size, id, misc-bytes)`,
matching how the real tool already treats every *unhandled* HIRC type.

IMPORTANT gotcha (verified directly against the real upstream source, not
just its internal comments): the upstream file named `wwise_hierarchy_140.py`
carries an internal module docstring claiming "Bank Version 154" and
`wwise_hierarchy_154.py` claims "Bank Version 140" — those in-file docstrings
are simply wrong/swapped in the upstream source. The reliable signal is
`core.py`'s `bank_version = ... ; if bank_version == 154: WwiseHierarchy_154
else: WwiseHierarchy_140`, i.e. filename/class-name, not the docstring text.
This file (and the Rust `BankVersion::V140`) matches upstream's
`wwise_hierarchy_140.py` by that filename-based signal: no `cache_id` on
`BankSourceStruct`/`TrackInfoStruct`, `MusicTrack` has no `BaseParam` (uses
raw `override_bus_id`/`parent_id` fields instead), `MusicSegment` is the
ad-hoc raw-skip shape (no `BaseParam`, a real serialized `parent_id`).
"""

import copy
import struct

from audio_util import MemoryStream, assert_equal


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
    bIsShareSet - U8x
    bIsRendered - U8x
    """

    def __init__(self, uFxIndex: int, fxId: int, bIsShareSet: int, bIsRendered: int):
        self.uFxIndex: int = uFxIndex
        self.fxId: int = fxId
        self.bIsShareSet: int = bIsShareSet
        self.bIsRendered: int = bIsRendered

    def get_data(self):
        return struct.pack("<BIBB", self.uFxIndex, self.fxId, self.bIsShareSet, self.bIsRendered)


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


class StateGroupState:
    """
    ulStateID tid
    ulStateInstanceID tid
    """

    def __init__(self, ulStateID: int = 0, ulStateInstanceID: int = 0):
        self.ulStateID = ulStateID
        self.ulStateInstanceID = ulStateInstanceID

    def get_data(self):
        return struct.pack("<II", self.ulStateID, self.ulStateInstanceID)


class StateGroup:
    """
    ulStateGroupID tid
    eStateSyncType U8x
    ulNumStates var (assume 8 bits, can be more)
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
        self.bitsFxBypass = 0
        self.fxChunks: list[FxChunk] = []

        self.bIsOverrideParentMetadata: int = 0
        self.uNumFxMetadata: int = 0
        self.fxChunksMetadata: list[FxChunkMetadata] = []

        self.bOverrideAttachmentParams: int = 0
        self.overrideBusId: int = 0
        self.directParentID: int = 0
        self.byBitVectorA: int = 0

        self.propBundle = PropBundle()

        self.positioningParamData: bytearray = bytearray()

        self.rangePropBundle = RangedPropBundle()

        self.auxParams = AuxParams()

        self.advSetting = AdvSetting()

        self.stateParams = StateParams()

        self.uNumRTPC: int = 0
        self.rtpcs: list[RTPC] = []

    @staticmethod
    def from_memory_stream(stream: MemoryStream):
        baseParam = BaseParam()

        baseParam.bIsOverrideParentFx = stream.uint8_read()
        baseParam.uNumFx = stream.uint8_read()
        if baseParam.uNumFx > 0:
            baseParam.bitsFxBypass = stream.uint8_read()
            baseParam.fxChunks = [
                FxChunk(stream.uint8_read(), stream.uint32_read(), stream.uint8_read(), stream.uint8_read())
                for _ in range(baseParam.uNumFx)
            ]

        baseParam.bIsOverrideParentMetadata = stream.uint8_read()
        baseParam.uNumFxMetadata = stream.uint8_read()
        if baseParam.uNumFxMetadata > 0:
            baseParam.fxChunksMetadata = [
                FxChunkMetadata(stream.uint8_read(), stream.uint32_read(), stream.uint8_read())
                for _ in range(baseParam.uNumFxMetadata)
            ]

        baseParam.bOverrideAttachmentParams = stream.uint8_read()

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
            states = [StateGroupState(stream.uint32_read(), stream.uint32_read()) for _ in range(ulNumStates)]
            stateGroups.append(StateGroup(ulStateGroupID, eStateSyncType, ulNumStates, states))
        baseParam.stateParams.stateGroups = stateGroups

        baseParam.uNumRTPC = stream.uint16_read()
        rtpcs: list[RTPC] = []
        for _ in range(baseParam.uNumRTPC):
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
            b += struct.pack("<B", self.bitsFxBypass)
            for fxChunk in self.fxChunks:
                b += fxChunk.get_data()

        assert_equal("# of metadata FX != # of metadata FX in the array", self.uNumFxMetadata, len(self.fxChunksMetadata))
        b += struct.pack("<BB", self.bIsOverrideParentMetadata, self.uNumFxMetadata)
        if self.uNumFxMetadata > 0:
            for fxChunkMetadata in self.fxChunksMetadata:
                b += fxChunkMetadata.to_bytes()

        b += struct.pack("<BIIB", self.bOverrideAttachmentParams, self.overrideBusId, self.directParentID, self.byBitVectorA)

        b += self.propBundle.get_data()

        b += self.rangePropBundle.get_data()

        b += struct.pack(f"<{len(self.positioningParamData)}s", self.positioningParamData)

        b += self.auxParams.get_data()

        b += self.advSetting.get_data()

        b += self.stateParams.get_data()

        b += struct.pack("<H", self.uNumRTPC)
        assert_equal("# of RTPC != # of RTPC in the array", self.uNumRTPC, len(self.rtpcs))
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
        self.mem_size: int = 0
        self.bit_flags: int = 0
        self.plugin_size: int = 0
        self.plugin_data: bytearray = bytearray()

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        b = BankSourceStruct()
        b.plugin_id, b.stream_type, b.source_id, b.mem_size, b.bit_flags = struct.unpack("<IBIIB", stream.read(14))
        if (b.plugin_id & 0x0F) == 2:
            if b.plugin_id:
                b.plugin_size = stream.uint32_read()
                if b.plugin_size > 0:
                    b.plugin_data = stream.read(b.plugin_size)
        return b

    def get_data(self):
        b = struct.pack("<IBIIB", self.plugin_id, self.stream_type, self.source_id, self.mem_size, self.bit_flags)
        if (self.plugin_id & 0x0F) == 2:
            if self.plugin_id:
                b += struct.pack("<I", self.plugin_size)
                if self.plugin_size > 0:
                    assert_equal("Plugin size mismatch size of plugin data", self.plugin_size, len(self.plugin_data))
                    b += struct.pack(f"<{len(self.plugin_data)}s", self.plugin_data)
        return b


class Sound(HircEntry):
    import_values = ["sources", "baseParam"]

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
        """Trimmed `Sound.set_data`: field-copy + size recompute only, no
        soundbanks/modified/parent bookkeeping (none of it affects output
        bytes)."""
        if entry:
            for value in self.import_values:
                try:
                    setattr(self, value, getattr(entry, value))
                except AttributeError:
                    pass
        self.size = len(self._pack())


class TrackInfoStruct:
    """v140: no `cache_id` (44 bytes total, `<IIIdddd`)."""

    def __init__(self):
        self.track_id = self.source_id = self.event_id = 0
        self.play_at = self.begin_trim_offset = self.end_trim_offset = self.source_duration = 0.0

    @classmethod
    def from_bytes(cls, data):
        t = TrackInfoStruct()
        (
            t.track_id,
            t.source_id,
            t.event_id,
            t.play_at,
            t.begin_trim_offset,
            t.end_trim_offset,
            t.source_duration,
        ) = struct.unpack("<IIIdddd", data)
        return t

    def get_data(self):
        return struct.pack(
            "<IIIdddd",
            self.track_id,
            self.source_id,
            self.event_id,
            self.play_at,
            self.begin_trim_offset,
            self.end_trim_offset,
            self.source_duration,
        )


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
    """v140: `bit_flags` right after `hierarchy_id`, no `BaseParam` — instead
    raw `override_bus_id`/`parent_id` fields at the tail. `override_bus_id`
    is read but NOT in `import_values` (matches upstream: it survives a
    merge unchanged)."""

    import_values = ["bit_flags", "unused_sections", "parent_id", "sources", "track_info", "clip_automations", "misc"]

    def __init__(self):
        super().__init__()
        self.bit_flags = 0
        self.unused_sections: list[bytes] = []
        self.sources: list[BankSourceStruct] = []
        self.track_info: list[TrackInfoStruct] = []
        self.clip_automations: list[ClipAutomationStruct] = []
        self.override_bus_id = 0
        self.parent_id = 0

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        entry = MusicTrack()
        entry.hierarchy_type = stream.uint8_read()
        entry.size = stream.uint32_read()
        start_position = stream.tell()
        entry.hierarchy_id = stream.uint32_read()
        entry.bit_flags = stream.uint8_read()
        num_sources = stream.uint32_read()
        for _ in range(num_sources):
            entry.sources.append(BankSourceStruct.from_memory_stream(stream))
        num_track_info = stream.uint32_read()
        for _ in range(num_track_info):
            entry.track_info.append(TrackInfoStruct.from_bytes(stream.read(44)))
        entry.unused_sections.append(stream.read(4))
        num_clip_automations = stream.uint32_read()
        for _ in range(num_clip_automations):
            entry.clip_automations.append(ClipAutomationStruct.from_memory_stream(stream))
        entry.unused_sections.append(stream.read(5))
        entry.override_bus_id = stream.uint32_read()
        entry.parent_id = stream.uint32_read()
        entry.misc = stream.read(entry.size - (stream.tell() - start_position))
        return entry

    def get_data(self):
        sources_bytes = b"".join([s.get_data() for s in self.sources])
        track_bytes = b"".join([t.get_data() for t in self.track_info])
        clip_bytes = b"".join([c.get_data() for c in self.clip_automations])
        payload = (
            sources_bytes
            + len(self.track_info).to_bytes(4, byteorder="little")
            + track_bytes
            + self.unused_sections[0]
            + len(self.clip_automations).to_bytes(4, byteorder="little")
            + clip_bytes
            + self.unused_sections[1]
            + self.override_bus_id.to_bytes(4, byteorder="little")
            + self.parent_id.to_bytes(4, byteorder="little")
            + self.misc
        )
        self.size = 9 + len(payload)
        return (
            struct.pack("<BIIBI", self.hierarchy_type, self.size, self.hierarchy_id, self.bit_flags, len(self.sources))
            + payload
        )

    def set_data(self, entry=None):
        if entry:
            for value in self.import_values:
                try:
                    setattr(self, value, getattr(entry, value))
                except AttributeError:
                    pass


class MusicSegment(HircEntry):
    """v140: ad-hoc raw-skip shape, no `BaseParam`. `parent_id` is a real,
    directly-serialized field here (unlike v154 where it's derived from
    `baseParam.directParentID` and not separately stored)."""

    import_values = ["parent_id", "tracks", "duration", "unused_sections", "markers"]

    def __init__(self):
        super().__init__()
        self.parent_id = 0
        self.unused_sections: list[bytes] = []
        self.tracks: list[int] = []
        self.duration = 0.0
        self.markers: list[list] = []  # [id, position, name-with-trailing-NUL]

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        entry = MusicSegment()
        entry.hierarchy_type = stream.uint8_read()
        entry.size = stream.uint32_read()
        entry.hierarchy_id = stream.uint32_read()
        entry.unused_sections.append(stream.read(10))
        entry.parent_id = stream.uint32_read()
        entry.unused_sections.append(stream.read(1))
        n = stream.uint8_read()  # number of props
        stream.seek(stream.tell() - 1)
        entry.unused_sections.append(stream.read(5 * n + 1))
        n = stream.uint8_read()  # number of props (again)
        stream.seek(stream.tell() - 1)
        entry.unused_sections.append(stream.read(5 * n + 1 + 12 + 4))
        n = stream.uint32_read()  # number of children (tracks)
        for _ in range(n):
            entry.tracks.append(stream.uint32_read())
        entry.unused_sections.append(stream.read(23))  # meter info
        n = stream.uint32_read()  # number of stingers
        stream.seek(stream.tell() - 4)
        entry.unused_sections.append(stream.read(24 * n + 4))
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
                struct.pack("<BII", self.hierarchy_type, self.size, self.hierarchy_id),
                self.unused_sections[0],
                self.parent_id.to_bytes(4, byteorder="little"),
                self.unused_sections[1],
                self.unused_sections[2],
                self.unused_sections[3],
                len(self.tracks).to_bytes(4, byteorder="little"),
                b"".join([x.to_bytes(4, byteorder="little") for x in self.tracks]),
                self.unused_sections[4],
                self.unused_sections[5],
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
    """Byte layout identical between bank versions; only the `import_hierarchy`
    merge behaviour differs (v140 wholesale-replaces all 5 fields below; v154
    only touches `baseParam.propBundle` + `playListSetting` — see
    `wwise_hierarchy_154.py`)."""

    import_values = ["baseParam", "children", "playListSetting", "ulPlayListItem", "playListItems"]

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
                    setattr(self, value, getattr(entry, value))
                except AttributeError:
                    pass
        self.size = len(self._pack())


class LayerContainer(HircEntry):
    """Byte layout identical between bank versions. Never merged via
    `import_hierarchy`/`set_data` (real upstream's type filter excludes it in
    both versions) — only `from_memory_stream`/`get_data` are needed, plus
    the raw `.children` attribute `GameArchive.load`'s cross-bank merge reads
    directly."""

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
    """v140 shape: two separate bit-vector bytes (`byBitVectorPlayBack`,
    `byBitVectorMode`) — v154 merges these into a single byte, see
    `wwise_hierarchy_154.py`."""

    def __init__(self):
        self.ulNodeID = 0
        self.byBitVectorPlayBack = 0
        self.byBitVectorMode = 0
        self.fadeOutTime = 0
        self.fadeInTime = 0

    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        p = SwitchParam()
        p.ulNodeID = stream.uint32_read()
        p.byBitVectorPlayBack = stream.uint8_read()
        p.byBitVectorMode = stream.uint8_read()
        p.fadeOutTime = stream.int32_read()
        p.fadeInTime = stream.int32_read()
        return p

    def get_data(self):
        return struct.pack(
            "<IBBii", self.ulNodeID, self.byBitVectorPlayBack, self.byBitVectorMode, self.fadeOutTime, self.fadeInTime
        )


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


class WwiseHierarchy_140:
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
        """Real upstream prunes each of the five container types' child id
        list down to ids present in `self.entries` before serializing (a
        child living only in a different bank of the same archive gets
        silently dropped from *this* bank's output), then restores the
        original list — verified directly against both
        `wwise_hierarchy_140.py`/`_154.py`."""
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
        """Trimmed `WwiseHierarchy_140.import_hierarchy`: v140 only merges
        `MusicSegment`/`MusicTrack` entries (NOT `Sound`/
        `RandomSequenceContainer` — verified against real upstream, a real
        asymmetry vs. v154, not an oversight), and adds the incoming entry
        wholesale if its id isn't already present (v154 does not have this
        add-branch — see `wwise_hierarchy_154.py`)."""
        for entry in new_hierarchy.get_entries():
            if isinstance(entry, (MusicSegment, MusicTrack)):
                if entry.hierarchy_id in self.entries:
                    self.entries[entry.hierarchy_id].set_data(entry)
                else:
                    self.entries[entry.hierarchy_id] = entry
