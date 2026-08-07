"""
Bank Version 154

Trimmed, phase-scoped port of hd2-audio-modder/wwise_hierarchy_154.py. See
`wwise_hierarchy_140.py`'s module docstring for the general trimming
philosophy (GUI/undo-redo/editing bookkeeping dropped).

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


class HircEntryFactory:
    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
        hierarchy_type = stream.uint8_read()
        stream.seek(stream.tell() - 1)
        if hierarchy_type == 0x02:
            return Sound.from_memory_stream(stream)
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
        return []

    def has_entry(self, entry_id):
        return entry_id in self.entries

    def get_entry(self, entry_id):
        return self.entries[entry_id]

    def get_data(self):
        arr = [entry.get_data() for entry in self.entries.values()]
        return len(arr).to_bytes(4, byteorder="little") + b"".join(arr)
