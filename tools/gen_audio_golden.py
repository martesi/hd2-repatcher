#!/usr/bin/env python3
"""Generate differential golden fixtures for the Rust audio-patching port
(phase 1: TOC/archive skeleton, opaque HIRC only).

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
from wwise_hierarchy_140 import HircEntry, WwiseHierarchy_140  # noqa: E402


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

    print("Done.")


if __name__ == "__main__":
    build_cases()
