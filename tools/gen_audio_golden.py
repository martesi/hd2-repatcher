#!/usr/bin/env python3
"""Generate differential golden fixtures for the Rust audio-patching port
(phases 1-2: TOC/archive skeleton with opaque HIRC, plus structured `Sound`/
`BaseParam` and DIDX/DATA regeneration).

Builds a small synthetic `GameArchive` (via the reference oracle's own
encoder — a legitimate way to manufacture a well-formed input archive; the
actual thing under test is the read -> [mutate] -> write round trip, not the
encoder used to produce the starting fixture), then runs it through the
*reference* Python engine (`reference/audio_core.py`) to capture the expected
re-serialized bytes. The Rust golden test (`crates/engine/tests/audio_golden.rs`)
replays the same input through the Rust port and asserts byte-identical
output.

Run from the repo root:  python tools/gen_audio_golden.py
"""

import json
import sys
import tempfile
import types
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
REFERENCE = REPO / "reference"
FIXTURES = REPO / "crates" / "engine" / "tests" / "fixtures"

# `reference/slim.py` imports `from lz4 import block` at module load, but none
# of these fixtures touch compressed packages, so stub it out.
_fake_lz4 = types.ModuleType("lz4")
_fake_lz4.block = types.ModuleType("lz4.block")
sys.modules.setdefault("lz4", _fake_lz4)
sys.modules.setdefault("lz4.block", _fake_lz4.block)

sys.path.insert(0, str(REFERENCE))
import audio_core as ac  # noqa: E402
import audio_const as ac_const  # noqa: E402
import wwise_hierarchy_140 as h140  # noqa: E402
import wwise_hierarchy_154 as h154  # noqa: E402
from wwise_hierarchy_140 import HircEntry, WwiseHierarchy_140  # noqa: E402


def hirc_module(version: int):
    return h154 if version == 154 else h140


def build_bank_source(version: int, source_id: int, mem_size: int, plugin_id=None):
    """A `BankSourceStruct` for the given bank version, defaulting to a
    BANK-embedded VORBIS source (the common case DIDX/DATA regeneration
    exercises)."""
    mod = hirc_module(version)
    src = mod.BankSourceStruct()
    src.plugin_id = ac_const.VORBIS if plugin_id is None else plugin_id
    src.stream_type = ac_const.BANK
    src.source_id = source_id
    src.mem_size = mem_size
    src.bit_flags = 0
    if version == 154:
        src.cache_id = 0
    return src


def build_base_param(version: int, *, num_fx: int = 0):
    """A minimal-but-valid `BaseParam`: every list/count defaults to empty
    (matching upstream's own `__init__` defaults), except
    `positioningParamData`, which must contain at least the one
    "no positioning" flag byte `parse_positioning_params` always reads."""
    mod = hirc_module(version)
    bp = mod.BaseParam()
    bp.positioningParamData = b"\x00"
    if num_fx > 0:
        bp.uNumFx = num_fx
        if version == 154:
            bp.fxChunks = [mod.FxChunk(i, 1000 + i, 0x07) for i in range(num_fx)]
        else:
            bp.fxChunks = [mod.FxChunk(i, 1000 + i, 1, 0) for i in range(num_fx)]
            bp.bitsFxBypass = 0xAB
    return bp


def build_sound(version: int, hierarchy_id: int, source, base_param):
    mod = hirc_module(version)
    s = mod.Sound()
    s.hierarchy_type = 0x02
    s.hierarchy_id = hierarchy_id
    s.sources = [source]
    s.baseParam = base_param
    s.size = len(s._pack())
    return s


def set_prop_bundle(version: int, base_param, ids, values):
    """Overwrites `base_param.propBundle` with a fresh one built from `ids`/
    `values` (each value must be exactly 4 bytes) — used to build distinct
    before/after `propBundle`s for the v154 "propBundle-only merge" fixtures."""
    mod = hirc_module(version)
    pb = mod.PropBundle()
    pb.cProps = len(ids)
    pb.pIDs = list(ids)
    pb.pValues = [bytes(v) for v in values]
    base_param.propBundle = pb
    return base_param


def build_track_info(version: int, *, track_id, source_id=0, event_id=0, play_at=0.0, begin=0.0, end=0.0, duration=0.0, cache_id=0):
    mod = hirc_module(version)
    t = mod.TrackInfoStruct()
    t.track_id = track_id
    t.source_id = source_id
    t.event_id = event_id
    t.play_at = play_at
    t.begin_trim_offset = begin
    t.end_trim_offset = end
    t.source_duration = duration
    if version == 154:
        t.cache_id = cache_id
    return t


def build_clip_automation(version: int, clip_index: int, auto_type: int, points):
    mod = hirc_module(version)
    c = mod.ClipAutomationStruct()
    c.clip_index = clip_index
    c.auto_type = auto_type
    c.graph_points = list(points)
    return c


def build_music_track(
    version: int,
    hierarchy_id: int,
    *,
    sources,
    track_infos,
    clip_automations,
    bit_flags: int = 0,
    override_bus_id: int = 0,
    parent_id: int = 0,
    track_type: int = 0,
    num_fx: int = 0,
):
    mod = hirc_module(version)
    t = mod.MusicTrack()
    t.hierarchy_type = 0x0B
    t.hierarchy_id = hierarchy_id
    t.sources = list(sources)
    t.track_info = list(track_infos)
    t.clip_automations = list(clip_automations)
    t.bit_flags = bit_flags
    t.misc = b""
    if version == 140:
        t.unused_sections = [filler(4, seed=hierarchy_id), filler(5, seed=hierarchy_id + 1)]
        t.override_bus_id = override_bus_id
        t.parent_id = parent_id
    else:
        t.unk1 = filler(4, seed=hierarchy_id) if track_infos else b""
        t.baseParam = build_base_param(154, num_fx=num_fx)
        t.baseParam.directParentID = parent_id
        t.track_type = track_type
    return t


def build_music_segment(version: int, hierarchy_id: int, *, tracks, duration, markers, bit_flags: int = 0, parent_id: int = 0):
    """`markers`: list of `(id, position, name_bytes)`; `name_bytes` must NOT
    contain an embedded NUL (the trailing NUL terminator is added here,
    matching the read-until-NUL loop's stored representation)."""
    mod = hirc_module(version)
    seg = mod.MusicSegment()
    seg.hierarchy_type = 0x0A
    seg.hierarchy_id = hierarchy_id
    seg.tracks = list(tracks)
    seg.duration = duration
    seg.markers = [[mid, pos, name + b"\x00"] for (mid, pos, name) in markers]
    if version == 140:
        seg.parent_id = parent_id
        seg.unused_sections = [
            filler(10, seed=hierarchy_id),  # [0]
            filler(1, seed=hierarchy_id + 1),  # [1]
            b"\x00",  # [2]: peek-u8 n=0 -> 5*0+1 = 1 byte
            b"\x00" + filler(16, seed=hierarchy_id + 2),  # [3]: n=0 -> 5*0+1+12+4 = 17 bytes
            filler(23, seed=hierarchy_id + 3),  # [4]: meter info
            b"\x00\x00\x00\x00",  # [5]: peek-u32 stinger count=0 -> 24*0+4 = 4 bytes
        ]
    else:
        seg.bit_flags = bit_flags
        seg.baseParam = build_base_param(154)
        seg.baseParam.directParentID = parent_id
        seg.meter_info = filler(23, seed=hierarchy_id + 3)
        seg.stingers = b"\x00\x00\x00\x00"  # peek-u32 stinger count=0 -> 4 bytes
    # MusicSegment.get_data() doesn't assert on `size` (unlike Sound/
    # RandomSequenceContainer) — its *length* doesn't depend on the value
    # already in `size`, so this is safe as a single pass (mirrors
    # `MusicSegment.set_data`'s own `self.size = len(self.get_data()) - 5`).
    seg.size = len(seg.get_data()) - 5
    return seg


def build_random_sequence_container(version: int, hierarchy_id: int, *, children_ids, play_list_items, num_fx: int = 0, loop_count: int = 0):
    mod = hirc_module(version)
    cntr = mod.RandomSequenceContainer()
    cntr.hierarchy_type = 0x05
    cntr.hierarchy_id = hierarchy_id
    cntr.baseParam = build_base_param(version, num_fx=num_fx)
    cntr.playListSetting = mod.PlayListSetting()
    cntr.playListSetting.sLoopCount = loop_count
    cntr.playListSetting.eMode = 1
    cntr.children.children = list(children_ids)
    cntr.ulPlayListItem = len(play_list_items)
    cntr.playListItems = [mod.PlayListItem(pid, w) for (pid, w) in play_list_items]
    cntr.size = len(cntr._pack())
    return cntr


def build_actor_mixer(version: int, hierarchy_id: int, *, children_ids, num_fx: int = 0):
    mod = hirc_module(version)
    m = mod.ActorMixer()
    m.hierarchy_type = 0x07
    m.hierarchy_id = hierarchy_id
    m.baseParam = build_base_param(version, num_fx=num_fx)
    m.children.children = list(children_ids)
    m.size = len(m._pack())
    return m


def build_layer_container(version: int, hierarchy_id: int, *, children_ids, layer_data: bytes = b""):
    mod = hirc_module(version)
    l = mod.LayerContainer()
    l.hierarchy_type = 0x09
    l.hierarchy_id = hierarchy_id
    l.baseParam = build_base_param(version)
    l.children.children = list(children_ids)
    l.layerData = layer_data
    l.size = len(l._pack())
    return l


def build_switch_group(version: int, switch_id: int, node_list):
    mod = hirc_module(version)
    g = mod.SwitchGroup()
    g.ulSwitchID = switch_id
    g.nodeList = list(node_list)
    return g


def build_switch_param(version: int, node_id: int, *, fade_out: int = 0, fade_in: int = 0, bit_playback: int = 0, bit_mode: int = 0):
    """`bit_playback` doubles as v154's single merged `byBitVector` value."""
    mod = hirc_module(version)
    p = mod.SwitchParam()
    p.ulNodeID = node_id
    p.fadeOutTime = fade_out
    p.fadeInTime = fade_in
    if version == 140:
        p.byBitVectorPlayBack = bit_playback
        p.byBitVectorMode = bit_mode
    else:
        p.byBitVector = bit_playback
    return p


def build_switch_container(
    version: int,
    hierarchy_id: int,
    *,
    children_ids,
    group_type: int = 0,
    group_id: int = 0,
    default_switch: int = 0,
    is_continuous_validation: int = 0,
    switch_groups=None,
    switch_params=None,
):
    mod = hirc_module(version)
    s = mod.SwitchContainer()
    s.hierarchy_type = 0x06
    s.hierarchy_id = hierarchy_id
    s.baseParam = build_base_param(version)
    s.eGroupType = group_type
    s.ulGroupID = group_id
    s.ulDefaultSwitch = default_switch
    s.bIsContinuousValidation = is_continuous_validation
    s.children.children = list(children_ids)
    s.switchGroups = list(switch_groups or [])
    s.switchParms = list(switch_params or [])
    s.size = len(s._pack())
    return s


def build_music_switch_container(version: int, hierarchy_id: int, *, children_ids, unused_byte: int = 0, tail: bytes = b""):
    mod = hirc_module(version)
    c = mod.MusicSwitchContainer()
    c.hierarchy_type = 0x0C
    c.hierarchy_id = hierarchy_id
    c.unused_sections = [bytes([unused_byte]), tail]
    c.baseParam = build_base_param(version)
    c.children.children = list(children_ids)
    # MusicSwitchContainer.get_data() doesn't assert on `size` (unlike the
    # other four container types) — matches `set_data`'s own generic
    # `self.size = len(self.get_data()) - 5` recompute.
    c.size = len(c.get_data()) - 5
    return c


def build_bank_audio_source(short_id: int, data: bytes) -> "ac.AudioSource":
    audio = ac.AudioSource()
    audio.stream_type = ac_const.BANK
    audio.short_id = short_id
    audio.set_data(bytearray(data), set_modified=False)
    return audio


def filler(n: int, seed: int = 0) -> bytes:
    return bytes(((i * 7 + 13 + seed) & 0xFF) for i in range(n))


def opaque_entry(hierarchy_type: int, hierarchy_id: int, misc: bytes) -> HircEntry:
    e = HircEntry()
    e.hierarchy_type = hierarchy_type
    e.hierarchy_id = hierarchy_id
    e.misc = bytearray(misc)
    e.size = len(misc) + 4
    return e


def build_hierarchy(entries) -> WwiseHierarchy_140:
    h = WwiseHierarchy_140()
    for e in entries:
        h.entries[e.hierarchy_id] = e
    return h


def build_bank(file_id: int, entries, extra_chunk=None, version: int = 140) -> "ac.WwiseBank":
    bank = ac.WwiseBank()
    bank.file_id = file_id
    bkhd_body = (version ^ ac_const.BANK_VERSION_KEY).to_bytes(4, "little") + filler(24, seed=file_id)
    bank.bank_header = b"BKHD" + len(bkhd_body).to_bytes(4, "little") + bkhd_body
    bank.hierarchy = build_hierarchy(entries)
    dep = ac.WwiseDep()
    dep.skip = True
    dep.data = f"Bank {file_id}"
    dep.file_id = file_id
    bank.dep = dep
    bank.bank_misc_data = b""
    if extra_chunk:
        tag, payload = extra_chunk
        bank.bank_misc_data = tag.encode("utf-8") + len(payload).to_bytes(4, byteorder="little") + payload
    return bank


def build_stream(file_id: int, data: bytes) -> "ac.WwiseStream":
    stream = ac.WwiseStream()
    stream.file_id = file_id
    audio = ac.AudioSource()
    audio.stream_type = ac_const.STREAM
    audio.resource_id = file_id
    audio.set_data(bytearray(data), set_modified=False)
    stream.audio_source = audio
    return stream


def build_text_bank(file_id: int, language: int, strings: dict) -> "ac.TextBank":
    tb = ac.TextBank()
    tb.file_id = file_id
    tb.language = language
    for sid, text in strings.items():
        se = ac.StringEntry()
        se.string_id = sid
        se.text = text
        tb.entries[sid] = se
    return tb


def build_video(file_id: int, data: bytes) -> "ac.VideoSource":
    v = ac.VideoSource()
    v.file_id = file_id
    v.video_size = len(data)
    v.modified = True
    v.replacement_video_offset = 0
    v.replacement_video_size = len(data)
    fd, path = tempfile.mkstemp(prefix="hd2-audio-golden-video-")
    with open(fd, "wb") as f:
        f.write(data)
    v.replacement_filepath = path
    return v


def base_archive(name: str) -> "ac.GameArchive":
    a = ac.GameArchive()
    a.magic = 0xF0000011
    a.name = name
    a.unknown = 0
    a.unk4Data = filler(56, seed=1)
    return a


def write_case(name: str, archive, mutate=None):
    """archive: fully populated GameArchive, not yet serialized.

    mutate: optional (description dict, callback) applied to the *loaded*
    archive before the final `to_file`, exercising the audio-byte-swap path.
    """
    case_dir = FIXTURES / name
    case_dir.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="hd2-audio-golden-") as tmp:
        input_dir = Path(tmp) / "input"
        input_dir.mkdir()

        # Manufacture a well-formed input archive via the oracle's own encoder.
        archive.to_file(str(input_dir))
        input_path = input_dir / archive.name
        (case_dir / "input.bin").write_bytes(input_path.read_bytes())
        stream_path = Path(str(input_path) + ".stream")
        if stream_path.exists():
            (case_dir / "input.bin.stream").write_bytes(stream_path.read_bytes())
        elif (case_dir / "input.bin.stream").exists():
            (case_dir / "input.bin.stream").unlink()

        # The operation under test: load -> [mutate] -> write.
        reloaded = ac.GameArchive.from_file(str(input_path))
        mutation_meta = {}
        if mutate is not None:
            mutation_meta, callback = mutate
            callback(reloaded)

        output_dir = Path(tmp) / "output"
        output_dir.mkdir()
        reloaded.to_file(str(output_dir))
        output_path = output_dir / reloaded.name
        (case_dir / "expected.bin").write_bytes(output_path.read_bytes())
        expected_stream = Path(str(output_path) + ".stream")
        if expected_stream.exists():
            (case_dir / "expected.bin.stream").write_bytes(expected_stream.read_bytes())
        elif (case_dir / "expected.bin.stream").exists():
            (case_dir / "expected.bin.stream").unlink()

    meta = {"archive_name": archive.name, "mutation": mutation_meta}
    (case_dir / "meta.json").write_text(json.dumps(meta, indent=2))
    print(f"  {name}: ok")


def build_cases():
    print("Generating audio golden fixtures...")

    # Case 1: plain round trip — one stream, one bank (opaque HIRC entries,
    # plus an unrecognized trailing chunk to exercise bank_misc_data
    # passthrough), one text bank, one video, no mutation.
    a = base_archive("9ba626afa44a3aa3.patch_0")
    a.wwise_streams[0x1111] = build_stream(0x1111, filler(64, seed=1))
    entries = [
        opaque_entry(0x01, 0xAAAA0001, filler(12, seed=2)),  # State
        opaque_entry(0x08, 0xAAAA0002, filler(20, seed=3)),  # AudioBus
        opaque_entry(0x0E, 0xAAAA0003, filler(8, seed=4)),  # Attenuation
    ]
    a.wwise_banks[0x2222] = build_bank(0x2222, entries, extra_chunk=("STID", filler(16, seed=5)))
    a.text_banks[0x3333] = build_text_bank(0x3333, 1, {100: "hello", 200: "woréld"})
    a.video_sources[0x4444] = build_video(0x4444, filler(48, seed=6))
    write_case("audio_roundtrip_basic", a)

    # Case 2: same shape, but the stream's audio bytes are swapped after
    # loading (the "one audio-byte swap" phase-1 goal) — and the extra
    # WwiseDep on-disk record is exercised too (bank.dep.skip = False).
    b = base_archive("9ba626afa44a3aa3.patch_0")
    b.wwise_streams[0x5555] = build_stream(0x5555, filler(32, seed=10))
    entries2 = [opaque_entry(0x0E, 0xBBBB0001, filler(6, seed=11))]
    bank = build_bank(0x6666, entries2)
    bank.dep.skip = False
    bank.dep.tag = 0x18
    bank.dep.data = "audio/mods/example.bnk"
    bank.dep.data_size = len(bank.dep.data.encode("utf-8"))
    b.wwise_banks[0x6666] = bank

    new_bytes = filler(40, seed=99)

    def swap(archive):
        archive.wwise_streams[0x5555].audio_source.set_data(bytearray(new_bytes))

    write_case(
        "audio_stream_byte_swap",
        b,
        mutate=({"stream_id": 0x5555, "new_data_hex": new_bytes.hex()}, swap),
    )

    # Case 3 (phase 2): a `Sound` HIRC entry with a BANK-embedded VORBIS
    # source — exercises structured Sound/BaseParam parsing and the
    # DIDX/DATA regeneration loop in `WwiseBank.generate`. Round trip only;
    # proves regeneration reproduces the original bytes exactly.
    sound_source_id = 0x7777
    original_bytes = filler(96, seed=20)

    c = base_archive("9ba626afa44a3aa3.patch_0")
    sound = build_sound(140, 0xCCCC0001, build_bank_source(140, sound_source_id, len(original_bytes)), build_base_param(140))
    sound_bank = build_bank(0x8888, [sound], version=140)
    sound_bank.media_index = [sound_source_id]
    c.wwise_banks[0x8888] = sound_bank
    c.audio_sources[sound_source_id] = build_bank_audio_source(sound_source_id, original_bytes)
    write_case("audio_sound_bank_source", c)

    # Case 4: same shape, but the underlying audio bytes are swapped after
    # loading — through `audio_sources` (short_id-keyed), not `wwise_streams`
    # — with a deliberately *different* length, proving DIDX/DATA offsets
    # and sizes are re-derived from the new data rather than cached.
    d = base_archive("9ba626afa44a3aa3.patch_0")
    sound2 = build_sound(140, 0xDDDD0001, build_bank_source(140, sound_source_id, len(original_bytes)), build_base_param(140))
    sound_bank2 = build_bank(0x9999, [sound2], version=140)
    sound_bank2.media_index = [sound_source_id]
    d.wwise_banks[0x9999] = sound_bank2
    d.audio_sources[sound_source_id] = build_bank_audio_source(sound_source_id, original_bytes)

    new_sound_bytes = filler(40, seed=21)

    def swap_bank_audio(archive):
        archive.audio_sources[sound_source_id].set_data(bytearray(new_sound_bytes))

    write_case(
        "audio_sound_bank_source_swap",
        d,
        mutate=({"short_id": sound_source_id, "new_data_hex": new_sound_bytes.hex()}, swap_bank_audio),
    )

    # Case 5: bank version 154 — exercises the real layout divergences found
    # by diffing upstream's wwise_hierarchy_140.py/_154.py: BankSourceStruct's
    # cache_id, FxChunk's packed bitVector byte (vs v140's two separate
    # bytes) via uNumFx>0 (also exercises the bBypassAll/bPypassAll upstream
    # bug — see wwise_hierarchy_154.py's module docstring), and a
    # StateGroupState with a non-empty AkPropBundle list (v140's
    # StateGroupState has no such list at all).
    v154_source_id = 0x6001
    v154_bytes = filler(52, seed=30)

    bp = build_base_param(154, num_fx=2)
    ak_props = [h154.AkPropBundle(5, 1.5), h154.AkPropBundle(9, -2.25)]
    state = h154.StateGroupState(0x42, len(ak_props), ak_props)
    bp.stateParams.ulNumStateGroups = 1
    bp.stateParams.stateGroups = [h154.StateGroup(0x99, 1, 1, [state])]

    e = base_archive("9ba626afa44a3aa3.patch_0")
    v154_sound = build_sound(154, 0xEEEE0001, build_bank_source(154, v154_source_id, len(v154_bytes)), bp)
    v154_bank = build_bank(0x8001, [v154_sound], version=154)
    v154_bank.media_index = [v154_source_id]
    e.wwise_banks[0x8001] = v154_bank
    e.audio_sources[v154_source_id] = build_bank_audio_source(v154_source_id, v154_bytes)
    write_case("audio_sound_v154_fx_state", e)

    print("Done.")


def build_phase3_roundtrip_cases():
    """Phase 3: `MusicSegment`/`MusicTrack`/`RandomSequenceContainer` parsing
    and `get_data()` re-serialization, exercised via the same full
    `GameArchive` round trip as phase 1-2's fixtures — one archive per bank
    version, mixing a `Sound` and a `MusicTrack` in the same bank so the
    DIDX/DATA loop's `get_sounds() + get_music_tracks()` chain gets covered
    too (two distinct BANK-embedded VORBIS sources)."""
    print("Generating phase-3 round-trip fixtures...")

    for version, case_name, base_id in ((140, "audio_music_types_v140", 0xA000_0000), (154, "audio_music_types_v154", 0xB000_0000)):
        sound_source_id = base_id | 0x01
        track_source_id = base_id | 0x02
        sound_bytes = filler(72, seed=base_id + 1)
        track_bytes = filler(56, seed=base_id + 2)

        sound = build_sound(
            version,
            base_id | 0x1001,
            build_bank_source(version, sound_source_id, len(sound_bytes)),
            build_base_param(version),
        )

        track_info = build_track_info(
            version, track_id=1, source_id=0x777, play_at=0.5, begin=0.1, end=0.9, duration=12.0, cache_id=3
        )
        clip = build_clip_automation(version, 0, 1, [(0.0, 1.0, 0), (1.0, 0.5, 2)])
        track = build_music_track(
            version,
            base_id | 0x1002,
            sources=[build_bank_source(version, track_source_id, len(track_bytes))],
            track_infos=[track_info],
            clip_automations=[clip],
            bit_flags=7,
            override_bus_id=555,
            parent_id=999,
            track_type=1,
        )

        segment = build_music_segment(
            version,
            base_id | 0x1003,
            tracks=[base_id | 0x1002],
            duration=42.5,
            markers=[(1, 0.0, b"entry"), (2, 42.5, b"exit")],
            bit_flags=2,
            parent_id=0xDEAD,
        )

        rsc = build_random_sequence_container(
            version,
            base_id | 0x1004,
            children_ids=[base_id | 0x1001, base_id | 0x1002],
            play_list_items=[(base_id | 0x1001, 50), (base_id | 0x1002, 50)],
            loop_count=3,
        )

        archive = base_archive("9ba626afa44a3aa3.patch_0")
        bank = build_bank(base_id | 0x2000, [sound, track, segment, rsc], version=version)
        bank.media_index = [sound_source_id, track_source_id]
        archive.wwise_banks[base_id | 0x2000] = bank
        archive.audio_sources[sound_source_id] = build_bank_audio_source(sound_source_id, sound_bytes)
        archive.audio_sources[track_source_id] = build_bank_audio_source(track_source_id, track_bytes)
        write_case(case_name, archive)


def write_hierarchy_merge_case(name: str, version: int, base_entries, patch_entries):
    """`import_hierarchy`'s field-merge, tested directly at the
    `WwiseHierarchy` level (raw HIRC bytes in, raw HIRC bytes out) rather
    than through a full `GameArchive`/`Mod`, since `Mod::import_patch`
    orchestration is ported in a later phase. Writes `base.bin`/`patch.bin`
    (each hierarchy's raw `get_data()`) and `expected.bin` (`base`'s
    `get_data()` after `base.import_hierarchy(patch)`)."""
    mod = hirc_module(version)
    hier_cls = mod.WwiseHierarchy_154 if version == 154 else mod.WwiseHierarchy_140

    base = hier_cls()
    for e in base_entries:
        base.entries[e.hierarchy_id] = e
    patch = hier_cls()
    for e in patch_entries:
        patch.entries[e.hierarchy_id] = e

    case_dir = FIXTURES / name
    case_dir.mkdir(parents=True, exist_ok=True)
    (case_dir / "base.bin").write_bytes(bytes(base.get_data()))
    (case_dir / "patch.bin").write_bytes(bytes(patch.get_data()))

    base.import_hierarchy(patch)
    (case_dir / "expected.bin").write_bytes(bytes(base.get_data()))

    (case_dir / "meta.json").write_text(json.dumps({"version": version}, indent=2))
    print(f"  {name}: ok")


def build_phase3_merge_cases():
    """Phase 3: `import_hierarchy` field-merge for the 4 eligible types
    (`Sound`, `MusicTrack`, `MusicSegment`, `RandomSequenceContainer`),
    covering the real per-version asymmetry found by diffing upstream:
    v140 only merges `MusicSegment`/`MusicTrack` (and adds a missing entry
    wholesale); v154 merges all 4 (narrower per-type field lists, e.g.
    `propBundle`-only for `Sound`/`RandomSequenceContainer`, matched-by-id
    `track_info` merge for `MusicTrack`) but never adds a missing entry."""
    print("Generating phase-3 import_hierarchy merge fixtures...")

    # --- v140: only MusicSegment/MusicTrack merge; Sound/RandomSequenceContainer
    # must come through untouched; a patch entry with an id absent from the
    # base gets added wholesale.
    v = 140
    sound_base = build_sound(v, 0x1001, build_bank_source(v, 100, 64), build_base_param(v))
    sound_patch = build_sound(v, 0x1001, build_bank_source(v, 999, 1), build_base_param(v, num_fx=1))

    track_base = build_music_track(
        v,
        0x1002,
        sources=[build_bank_source(v, 200, 32)],
        track_infos=[build_track_info(v, track_id=1, source_id=555, duration=1.0)],
        clip_automations=[build_clip_automation(v, 0, 0, [(0.0, 0.0, 0)])],
        bit_flags=1,
        override_bus_id=555,
        parent_id=777,
    )
    track_patch = build_music_track(
        v,
        0x1002,
        sources=[build_bank_source(v, 201, 40)],
        track_infos=[build_track_info(v, track_id=2, source_id=556, duration=2.0)],
        clip_automations=[build_clip_automation(v, 1, 1, [(1.0, 1.0, 1)])],
        bit_flags=9,
        override_bus_id=666,  # must NOT apply — v140 never merges override_bus_id
        parent_id=888,
    )

    seg_base = build_music_segment(v, 0x1003, tracks=[1, 2], duration=1.5, markers=[(1, 0.0, b"a")], parent_id=42)
    seg_patch = build_music_segment(v, 0x1003, tracks=[7, 8, 9], duration=9.75, markers=[(2, 1.0, b"b")], parent_id=100)

    rsc_base = build_random_sequence_container(v, 0x1004, children_ids=[9, 10], play_list_items=[(1, 100), (2, 200)])
    rsc_patch = build_random_sequence_container(v, 0x1004, children_ids=[11], play_list_items=[(3, 300)])

    new_entry = build_music_segment(v, 0x1005, tracks=[42], duration=0.5, markers=[])

    write_hierarchy_merge_case(
        "hirc_merge_v140",
        v,
        base_entries=[sound_base, track_base, seg_base, rsc_base],
        patch_entries=[sound_patch, track_patch, seg_patch, rsc_patch, new_entry],
    )

    # --- v154: all 4 types merge, but each with a narrower field list than a
    # wholesale replace (see docstrings in `wwise_hierarchy_154.py`); no
    # add-branch, so the id-0x2005 patch entry must be silently dropped.
    v = 154
    sound_base2 = build_sound(v, 0x2001, build_bank_source(v, 300, 64), build_base_param(v))
    set_prop_bundle(v, sound_base2.baseParam, [1, 2], [b"aaaa", b"bbbb"])
    sound_base2.size = len(sound_base2._pack())
    sound_patch2 = build_sound(v, 0x2001, build_bank_source(v, 999, 1), build_base_param(v))
    set_prop_bundle(v, sound_patch2.baseParam, [9], [b"zzzz"])
    sound_patch2.size = len(sound_patch2._pack())

    ti1_base = build_track_info(v, track_id=1, source_id=555, duration=10.0, cache_id=1)
    ti2_base = build_track_info(v, track_id=2, event_id=777, play_at=1.0, duration=20.0, cache_id=2)
    ti1_patch = build_track_info(v, track_id=91, source_id=555, play_at=99.0, begin=9.0, end=9.0, duration=99.0, cache_id=9)
    ti2_patch = build_track_info(v, track_id=92, event_id=777, play_at=88.0, begin=8.0, end=8.0, duration=88.0, cache_id=8)
    track_base2 = build_music_track(
        v,
        0x2002,
        sources=[build_bank_source(v, 400, 32)],
        track_infos=[ti1_base, ti2_base],
        clip_automations=[build_clip_automation(v, 0, 0, [(0.0, 0.0, 0)])],
        bit_flags=1,
        parent_id=42,
    )
    track_patch2 = build_music_track(
        v,
        0x2002,
        sources=[build_bank_source(v, 401, 40)],  # must NOT apply — v154 never merges sources
        track_infos=[ti1_patch, ti2_patch],
        clip_automations=[build_clip_automation(v, 1, 1, [(1.0, 1.0, 1)])],
        bit_flags=9,  # must NOT apply
        parent_id=100,
    )

    seg_base2 = build_music_segment(v, 0x2003, tracks=[1, 2], duration=1.5, markers=[(1, 0.0, b"a")], bit_flags=3, parent_id=42)
    seg_patch2 = build_music_segment(
        v, 0x2003, tracks=[7, 8, 9], duration=9.75, markers=[(2, 1.0, b"b")], bit_flags=9, parent_id=100
    )  # bit_flags/base_param must NOT apply — only tracks/duration/markers do

    rsc_base2 = build_random_sequence_container(v, 0x2004, children_ids=[9, 10], play_list_items=[(1, 100), (2, 200)], loop_count=1)
    set_prop_bundle(v, rsc_base2.baseParam, [3], [b"cccc"])
    rsc_base2.size = len(rsc_base2._pack())
    rsc_patch2 = build_random_sequence_container(v, 0x2004, children_ids=[11], play_list_items=[(3, 300)], loop_count=9)
    set_prop_bundle(v, rsc_patch2.baseParam, [4], [b"dddd"])
    rsc_patch2.size = len(rsc_patch2._pack())

    new_entry2 = build_music_segment(v, 0x2005, tracks=[42], duration=0.5, markers=[])

    write_hierarchy_merge_case(
        "hirc_merge_v154",
        v,
        base_entries=[sound_base2, track_base2, seg_base2, rsc_base2],
        patch_entries=[sound_patch2, track_patch2, seg_patch2, rsc_patch2, new_entry2],
    )

    print("Done.")


def build_phase4_cases():
    """Phase 4: the four remaining container types (`ActorMixer`,
    `SwitchContainer`, `LayerContainer`, `MusicSwitchContainer`), plus the
    two merge mechanisms that apply to all five container types —
    `GameArchive.load`'s cross-bank children union and
    `WwiseHierarchy.get_data`'s dangling-child pruning — neither of which
    phase 3's `RandomSequenceContainer` fixtures exercised."""
    print("Generating phase-4 container fixtures...")

    for version, case_name, base_id in ((140, "audio_containers_v140", 0xC000_0000), (154, "audio_containers_v154", 0xD000_0000)):
        mixer = build_actor_mixer(version, base_id | 0x01, children_ids=[base_id | 0x10], num_fx=1)
        switch = build_switch_container(
            version,
            base_id | 0x02,
            children_ids=[base_id | 0x01],
            group_type=1,
            group_id=42,
            default_switch=base_id | 0x01,
            is_continuous_validation=1,
            switch_groups=[build_switch_group(version, 42, [base_id | 0x01, base_id | 0x02])],
            switch_params=[
                build_switch_param(version, base_id | 0x01, fade_out=100, fade_in=200, bit_playback=1, bit_mode=1),
                build_switch_param(version, base_id | 0x02, fade_out=0, fade_in=50, bit_playback=0, bit_mode=0),
            ],
        )
        layer = build_layer_container(version, base_id | 0x03, children_ids=[base_id | 0x02], layer_data=filler(20, seed=base_id))
        music_switch = build_music_switch_container(
            version, base_id | 0x04, children_ids=[base_id | 0x03], unused_byte=0x5, tail=filler(9, seed=base_id + 1)
        )
        rsc = build_random_sequence_container(
            version, base_id | 0x05, children_ids=[base_id | 0x04], play_list_items=[(base_id | 0x04, 100)]
        )
        leaf = opaque_entry(0x01, base_id | 0x10, filler(6, seed=base_id + 2))

        archive = base_archive("9ba626afa44a3aa3.patch_0")
        bank = build_bank(base_id | 0x2000, [leaf, mixer, switch, layer, music_switch, rsc], version=version)
        archive.wwise_banks[base_id | 0x2000] = bank
        write_case(case_name, archive)

    # --- Cross-bank children union (`GameArchive.load`, `core.py:818-829`):
    # the same ActorMixer `hierarchy_id` appears in two banks of the same
    # archive with different (overlapping) children lists; every id each
    # bank's own list references is defined locally in *both* banks (kept
    # byte-identical) so dangling-child pruning never masks the merge
    # itself — this case is purely about the union + cross-bank backfill.
    for version, case_name, base_id in (
        (140, "audio_container_children_merge_v140", 0xE000_0000),
        (154, "audio_container_children_merge_v154", 0xE100_0000),
    ):
        leaf_a = opaque_entry(0x01, base_id | 0x01, filler(4, seed=1))
        leaf_b = opaque_entry(0x01, base_id | 0x02, filler(4, seed=2))
        leaf_c = opaque_entry(0x01, base_id | 0x03, filler(4, seed=3))
        mixer_id = base_id | 0x100

        bank1 = build_bank(
            base_id | 0x2001,
            [leaf_a, leaf_b, leaf_c, build_actor_mixer(version, mixer_id, children_ids=[base_id | 0x01, base_id | 0x02])],
            version=version,
        )
        bank2 = build_bank(
            base_id | 0x2002,
            [leaf_a, leaf_b, leaf_c, build_actor_mixer(version, mixer_id, children_ids=[base_id | 0x02, base_id | 0x03])],
            version=version,
        )

        archive = base_archive("9ba626afa44a3aa3.patch_0")
        archive.wwise_banks[base_id | 0x2001] = bank1
        archive.wwise_banks[base_id | 0x2002] = bank2
        write_case(case_name, archive)

    # NOTE: dangling-child pruning (`WwiseHierarchy.get_data`) is NOT
    # exercisable as an oracle-diffed fixture through this generator's
    # write_case flow: `GameArchive.to_file` (called both to manufacture
    # `input.bin` *and* to produce `expected.bin`) always prunes on every
    # write, so a hand-built dangling child reference never survives even
    # the very first serialization — there is no "pre-prune" input state
    # this flow can express. It's covered instead by a pure-Rust unit test
    # against `WwiseHierarchy::get_data()` directly (no oracle needed for a
    # simple filter+arithmetic operation) — see
    # `crates/engine/src/wwise/hierarchy/mod.rs`'s `#[cfg(test)]` module.

    print("Done.")


def write_mod_case(name: str, bases: list, patches: list):
    """Phase 5: exercises the full `Mod` orchestration (`import_patch`,
    `write_patch`, `write_separate_patches`, `add_game_archive`) rather than
    a bare `GameArchive` round trip.

    `bases`/`patches`: fully populated `ac.GameArchive`s (not yet
    serialized, distinct `.name`s). Each is encoded to a real file on disk
    via the oracle's own encoder (same "legitimate way to manufacture a
    well-formed input" as `write_case`), then replayed through a fresh
    `ac.Mod`: every base loaded via `load_archive_file` (sorted by name),
    then every patch applied via `import_patch` (sorted by name, matching
    `run_patch_cli`'s `sorted(os.listdir(...))`). Captures both
    `write_patch` (combined) and `write_separate_patches` (per-base-archive)
    output for the Rust golden test to replay and byte-diff.
    """
    case_dir = FIXTURES / name
    case_dir.mkdir(parents=True, exist_ok=True)
    base_dir = case_dir / "base"
    patch_dir = case_dir / "patches"
    expected_combined = case_dir / "expected_combined"
    expected_separate = case_dir / "expected_separate"
    for d in (base_dir, patch_dir, expected_combined, expected_separate):
        d.mkdir(exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="hd2-mod-golden-") as tmp:
        tmp = Path(tmp)

        def encode(archive, subdir):
            d = tmp / subdir / archive.name
            d.mkdir(parents=True)
            archive.to_file(str(d))
            return d / archive.name

        base_paths = sorted((encode(a, f"base_{a.name}") for a in bases), key=lambda p: p.name)
        patch_paths = sorted((encode(a, f"patch_{a.name}") for a in patches), key=lambda p: p.name)

        for p in base_paths:
            for f in p.parent.iterdir():
                (base_dir / f.name).write_bytes(f.read_bytes())
        for p in patch_paths:
            for f in p.parent.iterdir():
                (patch_dir / f.name).write_bytes(f.read_bytes())

        mod = ac.Mod()
        for p in base_paths:
            assert mod.load_archive_file(str(p)), f"[{name}] failed to load base archive {p.name}"
        for p in patch_paths:
            assert mod.import_patch(str(p)), f"[{name}] failed to import patch {p.name}"

        combined_dir = tmp / "combined"
        combined_dir.mkdir()
        mod.write_patch(str(combined_dir))
        for f in combined_dir.iterdir():
            (expected_combined / f.name).write_bytes(f.read_bytes())

        separate_dir = tmp / "separate"
        separate_dir.mkdir()
        mod.write_separate_patches(str(separate_dir))
        for f in separate_dir.iterdir():
            (expected_separate / f.name).write_bytes(f.read_bytes())

    meta = {
        "base_names": [p.name for p in base_paths],
        "patch_names": [p.name for p in patch_paths],
    }
    (case_dir / "meta.json").write_text(json.dumps(meta, indent=2))
    print(f"  {name}: ok")


def build_phase5_cases():
    """Phase 5: full `Mod::import_patch`/`write_patch`/
    `write_separate_patches`/`add_game_archive` orchestration — wiring
    phases 1-4 into the real control flow used by a mod-folder patch run,
    including the audio-swap-must-mark-bank-modified gap and the
    hierarchy-merge-must-mark-bank-modified gap this phase's implementation
    found and fixed (see `Mod.import_hierarchy`/`Mod.import_patch`'s
    `swapped_ids` comments in `reference/audio_core.py`), plus text-bank
    import and both video branches."""
    print("Generating phase-5 Mod-orchestration fixtures...")

    # --- Case 1: single base archive + single patch, v154. Exercises audio
    # byte swap (must mark the owning bank modified even though no HIRC
    # field changed) *and* a Sound propBundle field-merge (must also mark
    # the bank modified) *and* a text-bank string change, all landing in the
    # same bank/archive so both write_patch and write_separate_patches can
    # be checked against one combined+one per-archive expected output.
    source_id = 0x9101
    original_bytes = filler(64, seed=101)
    new_bytes = filler(48, seed=102)

    base_bp = build_base_param(154)
    set_prop_bundle(154, base_bp, [1], [b"aaaa"])
    base_sound = build_sound(154, 0xF001, build_bank_source(154, source_id, len(original_bytes)), base_bp)
    base1 = base_archive("1111111111111111")
    bank1 = build_bank(0x9001, [base_sound], version=154)
    bank1.media_index = [source_id]
    base1.wwise_banks[0x9001] = bank1
    base1.audio_sources[source_id] = build_bank_audio_source(source_id, original_bytes)
    base1.text_banks[0x9002] = build_text_bank(0x9002, 1, {1: "old text"})

    patch_bp = build_base_param(154)
    set_prop_bundle(154, patch_bp, [9], [b"zzzz"])
    patch_sound = build_sound(154, 0xF001, build_bank_source(154, source_id, len(new_bytes)), patch_bp)
    patch1 = base_archive("mod_patch_basic.patch_0")
    patch_bank1 = build_bank(0x9001, [patch_sound], version=154)
    patch_bank1.media_index = [source_id]
    patch1.wwise_banks[0x9001] = patch_bank1
    patch1.audio_sources[source_id] = build_bank_audio_source(source_id, new_bytes)
    patch1.text_banks[0x9002] = build_text_bank(0x9002, 1, {1: "new text"})

    write_mod_case("mod_import_patch_basic", bases=[base1], patches=[patch1])

    # --- Case 2: video handling, both branches. Base archive has one
    # existing video; one patch touches it (merge into the existing pooled
    # VideoSource), a second patch introduces a wholly new video (added as a
    # genuinely new resource via add_game_archive). No banks at all, to
    # prove video-only patches drive `add_patch` on their own.
    #
    # Kept as *separate* single-video patch archives rather than one patch
    # with both: real upstream's `Mod.import_video` (`core.py:2187-2189`)
    # sets `replacement_video_size` from `os.path.getsize(patch_file+".stream")`
    # — the *whole* companion `.stream` file's size, not the specific video's
    # own byte range within it — then only overrides `replacement_video_offset`,
    # not the size. For a single-video patch this quirk is unobservable (the
    # oversized read gets clamped to EOF, returning exactly that video's own
    # bytes); with two videos packed into one `.stream` file it would read
    # past the first video into the second's bytes. Faithfully replicating
    # that quirk in the Rust port's eager-byte-slicing `VideoSource` would
    # mean deliberately reintroducing a real upstream bug into new code — not
    # worth it for a case this narrow, so the fixture sidesteps it instead of
    # exercising it.
    base2 = base_archive("2222222222222222")
    base2.video_sources[0xA001] = build_video(0xA001, filler(32, seed=201))

    patch2a = base_archive("mod_patch_video_known.patch_0")
    patch2a.video_sources[0xA001] = build_video(0xA001, filler(32, seed=202))  # existing -> merge branch

    patch2b = base_archive("mod_patch_video_new.patch_0")
    patch2b.video_sources[0xA002] = build_video(0xA002, filler(24, seed=203))  # new -> add branch

    write_mod_case("mod_video_both_branches", bases=[base2], patches=[patch2a, patch2b])

    # --- Case 3: `Mod.add_game_archive`'s ActorMixer-only cross-*archive*
    # children merge (`core.py:1907-1912`) — distinct from `GameArchive.load`'s
    # wider five-type cross-*bank* union already covered by phase 4's
    # `audio_container_children_merge_*` fixtures. Two base archives each
    # own a *different* bank, but both banks define the *same* ActorMixer
    # `hierarchy_id` with non-overlapping children; loading base_2 after
    # base_1 must union them into the pooled `hierarchy_entries` copy *and*
    # backfill base_2's own bank copy (not base_1's — a real, verified
    # upstream asymmetry, see `Mod::add_game_archive`'s doc comment). Only
    # base_2's bank ever gets marked modified (by the patch's Sound
    # propBundle merge below), so only its serialized bytes are checkable
    # here — but that's sufficient to prove the merged children made it into
    # what actually gets written.
    # Both leaves must be defined in *both* banks' own hierarchy (kept
    # byte-identical), even though each bank's ActorMixer only lists one as
    # a child — otherwise `WwiseHierarchy.get_data`'s dangling-child pruning
    # (a *different*, per-bank mechanism — see `containers.rs`'s module doc)
    # would silently drop whichever leaf isn't locally defined, masking the
    # cross-archive union this case exists to prove. Same pattern phase 4's
    # `audio_container_children_merge_*` fixtures use for the cross-*bank*
    # union.
    mixer_id = 0xC001
    leaf_a = opaque_entry(0x01, 0xC010, filler(4, seed=1))
    leaf_b = opaque_entry(0x01, 0xC020, filler(4, seed=2))

    base3a = base_archive("3333333333333333")
    base3a.wwise_banks[0xB001] = build_bank(
        0xB001, [leaf_a, leaf_b, build_actor_mixer(154, mixer_id, children_ids=[0xC010])], version=154
    )

    merge_source_id = 0xC040
    merge_original_bytes = filler(40, seed=204)
    merge_bp = build_base_param(154)
    set_prop_bundle(154, merge_bp, [2], [b"bbbb"])
    merge_sound = build_sound(154, 0xC030, build_bank_source(154, merge_source_id, len(merge_original_bytes)), merge_bp)

    base3b = base_archive("4444444444444444")
    bank_b = build_bank(
        0xB002, [leaf_a, leaf_b, build_actor_mixer(154, mixer_id, children_ids=[0xC020]), merge_sound], version=154
    )
    bank_b.media_index = [merge_source_id]
    base3b.wwise_banks[0xB002] = bank_b
    base3b.audio_sources[merge_source_id] = build_bank_audio_source(merge_source_id, merge_original_bytes)

    patch3_bp = build_base_param(154)
    set_prop_bundle(154, patch3_bp, [3], [b"cccc"])
    patch3_sound = build_sound(154, 0xC030, build_bank_source(154, merge_source_id, len(merge_original_bytes)), patch3_bp)
    patch3 = base_archive("mod_patch_actormixer.patch_0")
    patch_bank_b = build_bank(0xB002, [patch3_sound], version=154)
    patch_bank_b.media_index = [merge_source_id]
    patch3.wwise_banks[0xB002] = patch_bank_b

    write_mod_case("mod_add_game_archive_actormixer_merge", bases=[base3a, base3b], patches=[patch3])

    print("Done.")


def write_process_audio_patches_case(name: str, gamedata_archives: list, patches: list):
    """Phase 7: exercises `process_audio_patches`'s whole orchestration —
    resolving each patch's touched soundbanks to their containing base
    archive, loading those archives, importing every patch (sorted by
    name), and writing one combined `9ba626afa44a3aa3.patch_0` — by
    manually driving the same `ac.Mod` methods `run_patch_cli`
    (`hd2-audio-modder/audio_modder.py:3647-3737`) does, minus its
    friendlynames-db lookup: this fixture resolves each soundbank's archive
    directly from `gamedata_archives`, the same replacement `AudioIndex`
    provides on the Rust side.

    `gamedata_archives`: fully populated `ac.GameArchive`s representing base
    game archives, encoded via `to_file` straight into a `gamedata/` folder
    (the same on-disk shape `GameResources::load`/`AudioIndex` scan — no
    `base/<archive-name>/` staging subfolder like `write_mod_case` uses).
    `patches`: fully populated `ac.GameArchive`s encoded into a `patches/`
    folder as `.patch_N` input files.
    """
    case_dir = FIXTURES / name
    case_dir.mkdir(parents=True, exist_ok=True)
    gamedata_dir = case_dir / "gamedata"
    patch_dir = case_dir / "patches"
    expected_dir = case_dir / "expected"
    for d in (gamedata_dir, patch_dir, expected_dir):
        d.mkdir(exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="hd2-audio-orchestration-golden-") as tmp:
        tmp = Path(tmp)

        def encode(archive, subdir):
            d = tmp / subdir / archive.name
            d.mkdir(parents=True)
            archive.to_file(str(d))
            return d / archive.name

        gamedata_paths = sorted((encode(a, f"gamedata_{a.name}") for a in gamedata_archives), key=lambda p: p.name)
        patch_paths = sorted((encode(a, f"patch_{a.name}") for a in patches), key=lambda p: p.name)

        for p in gamedata_paths:
            for f in p.parent.iterdir():
                (gamedata_dir / f.name).write_bytes(f.read_bytes())
        for p in patch_paths:
            for f in p.parent.iterdir():
                (patch_dir / f.name).write_bytes(f.read_bytes())

        # Legacy-install marker file `GameResources::load`'s `Slim::init`
        # checks for on the Rust side; write an empty one if none of the
        # gamedata archives already used that exact name.
        if not (gamedata_dir / "9ba626afa44a3aa3").exists():
            (gamedata_dir / "9ba626afa44a3aa3").write_bytes(b"")

        # Mirror `run_patch_cli`'s archive-resolution step exactly, resolving
        # each touched soundbank's archive directly instead of through a
        # friendlynames db.
        bank_to_archive = {}
        for a in gamedata_archives:
            for bank_id in a.get_wwise_banks().keys():
                bank_to_archive[bank_id] = a.name

        archives_needed: set = set()
        for p in patch_paths:
            pc = ac.GameArchive.from_file(str(p))
            if len(pc.get_text_banks()) > 0:
                archives_needed.add("9ba626afa44a3aa3")
            for bank_id in pc.get_wwise_banks().keys():
                assert bank_id in bank_to_archive, f"[{name}] fixture bug: soundbank {bank_id} has no gamedata archive"
                archives_needed.add(bank_to_archive[bank_id])

        mod = ac.Mod()
        for a in sorted(archives_needed):
            assert mod.load_archive_file(str(gamedata_dir / a)), f"[{name}] failed to load gamedata archive {a}"
        for p in patch_paths:
            assert mod.import_patch(str(p)), f"[{name}] failed to import patch {p.name}"

        mod.write_patch(str(expected_dir), "9ba626afa44a3aa3.patch_0")

    meta = {"patch_names": [p.name for p in patch_paths]}
    (case_dir / "meta.json").write_text(json.dumps(meta, indent=2))
    print(f"  {name}: ok")


def build_phase7_cases():
    """Phase 7: `lib.rs::process_audio_patches`, the engine orchestration
    entry point wiring `AudioIndex`-based archive resolution + `Slim`-backed
    base-archive loading (phase 6) into the `Mod::import_patch`/`write_patch`
    control flow (phase 5)."""
    print("Generating phase-7 process_audio_patches fixtures...")

    # --- Case 1: single soundbank archive + the hardcoded text-bank archive,
    # both resolved from a gamedata scan (not passed in explicitly like
    # write_mod_case does) — proves both of process_audio_patches's archive-
    # resolution branches (WWISE_BANK via AudioIndex, TEXT_BANK via the
    # hardcoded 9ba626afa44a3aa3 name) in one patch.
    source_id = 0x9101
    original_bytes = filler(64, seed=301)
    new_bytes = filler(48, seed=302)

    base_bp = build_base_param(154)
    set_prop_bundle(154, base_bp, [1], [b"aaaa"])
    base_sound = build_sound(154, 0xF001, build_bank_source(154, source_id, len(original_bytes)), base_bp)
    gamedata1 = base_archive("1111111111111111")
    bank1 = build_bank(0x9001, [base_sound], version=154)
    bank1.media_index = [source_id]
    gamedata1.wwise_banks[0x9001] = bank1
    gamedata1.audio_sources[source_id] = build_bank_audio_source(source_id, original_bytes)

    text_archive = base_archive("9ba626afa44a3aa3")
    text_archive.text_banks[0x9002] = build_text_bank(0x9002, 1, {1: "old text"})

    patch_bp = build_base_param(154)
    set_prop_bundle(154, patch_bp, [9], [b"zzzz"])
    patch_sound = build_sound(154, 0xF001, build_bank_source(154, source_id, len(new_bytes)), patch_bp)
    patch1 = base_archive("mod_patch_orchestration.patch_0")
    patch_bank1 = build_bank(0x9001, [patch_sound], version=154)
    patch_bank1.media_index = [source_id]
    patch1.wwise_banks[0x9001] = patch_bank1
    patch1.audio_sources[source_id] = build_bank_audio_source(source_id, new_bytes)
    patch1.text_banks[0x9002] = build_text_bank(0x9002, 1, {1: "new text"})

    write_process_audio_patches_case(
        "process_audio_patches_single_archive_and_text_bank",
        gamedata_archives=[gamedata1, text_archive],
        patches=[patch1],
    )

    # --- Case 2: two patches in the same directory, each touching a
    # *different* base archive's soundbank — proves multi-archive
    # resolution and that `process_audio_patches`'s own sort-by-path
    # ordering (matching `sorted(os.listdir(...))`) produces the same
    # result as this fixture's own `sorted(patch_paths)` order.
    source_a = 0xA101
    source_b = 0xB101
    orig_a = filler(40, seed=401)
    orig_b = filler(56, seed=402)
    new_a = filler(40, seed=403)
    new_b = filler(56, seed=404)

    sound_a = build_sound(154, 0xA001, build_bank_source(154, source_a, len(orig_a)), build_base_param(154))
    gamedata_a = base_archive("aaaaaaaaaaaaaaaa")
    bank_a = build_bank(0xA002, [sound_a], version=154)
    bank_a.media_index = [source_a]
    gamedata_a.wwise_banks[0xA002] = bank_a
    gamedata_a.audio_sources[source_a] = build_bank_audio_source(source_a, orig_a)

    sound_b = build_sound(154, 0xB001, build_bank_source(154, source_b, len(orig_b)), build_base_param(154))
    gamedata_b = base_archive("bbbbbbbbbbbbbbbb")
    bank_b = build_bank(0xB002, [sound_b], version=154)
    bank_b.media_index = [source_b]
    gamedata_b.wwise_banks[0xB002] = bank_b
    gamedata_b.audio_sources[source_b] = build_bank_audio_source(source_b, orig_b)

    patch_sound_a = build_sound(154, 0xA001, build_bank_source(154, source_a, len(new_a)), build_base_param(154))
    patch_a = base_archive("mod_patch_multi_a.patch_0")
    patch_bank_a = build_bank(0xA002, [patch_sound_a], version=154)
    patch_bank_a.media_index = [source_a]
    patch_a.wwise_banks[0xA002] = patch_bank_a
    patch_a.audio_sources[source_a] = build_bank_audio_source(source_a, new_a)

    patch_sound_b = build_sound(154, 0xB001, build_bank_source(154, source_b, len(new_b)), build_base_param(154))
    patch_b = base_archive("mod_patch_multi_b.patch_0")
    patch_bank_b = build_bank(0xB002, [patch_sound_b], version=154)
    patch_bank_b.media_index = [source_b]
    patch_b.wwise_banks[0xB002] = patch_bank_b
    patch_b.audio_sources[source_b] = build_bank_audio_source(source_b, new_b)

    write_process_audio_patches_case(
        "process_audio_patches_multi_archive",
        gamedata_archives=[gamedata_a, gamedata_b],
        patches=[patch_a, patch_b],
    )

    print("Done.")


if __name__ == "__main__":
    build_cases()
    build_phase3_roundtrip_cases()
    build_phase3_merge_cases()
    build_phase4_cases()
    build_phase5_cases()
    build_phase7_cases()
