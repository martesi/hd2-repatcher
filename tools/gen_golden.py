#!/usr/bin/env python3
"""Generate differential golden fixtures for the Rust engine port.

Builds synthetic — but structurally valid — patch files, runs the *reference*
Python engine (`reference/update_unit_mods.py`) on them with a stubbed unit-data
source, and writes the byte-exact results into
`crates/engine/tests/fixtures/<case>/`. The Rust golden test
(`crates/engine/tests/golden.rs`) replays the same inputs and asserts it produces
identical bytes and outcome.

Run from the repo root:  python tools/gen_golden.py
"""

import json
import os
import struct
import sys
import types
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
REFERENCE = REPO / "reference"
FIXTURES = REPO / "crates" / "engine" / "tests" / "fixtures"

# The reference engine imports `from lz4 import block` at module load, but the
# update_patch_file path never actually decompresses anything, so stub it out to
# avoid needing the real lz4 dependency just to generate fixtures.
_fake_lz4 = types.ModuleType("lz4")
_fake_lz4.block = types.ModuleType("lz4.block")
sys.modules.setdefault("lz4", _fake_lz4)
sys.modules.setdefault("lz4.block", _fake_lz4.block)

sys.path.insert(0, str(REFERENCE))
import update_unit_mods as uum  # noqa: E402

UNIT_TYPE_ID = uum.UNIT_TYPE_ID = 16187218042980615487
OTHER_TYPE_ID = 0x1234_5678_9ABC_DEF0  # some non-unit type id (>= 2**32)
SMALL_TYPE_ID = 0x0000_0042  # < 2**32, triggers CORRUPTED_FILE

HEADER_SIZE = 80
TYPE_ENTRY_SIZE = 32
MAIN_HEADER_SIZE = 72


def filler(n: int, seed: int = 0) -> bytearray:
    """Deterministic non-zero-ish filler so item formats etc. vary."""
    return bytearray(((i * 7 + 13 + seed) & 0xFF) for i in range(n))


def main_header(num_types: int, num_files: int) -> bytes:
    return struct.pack("<IIII56s", 0xF0000011, num_types, num_files, 0, b"\x00" * 56)


def type_entry(type_id: int, count: int) -> bytes:
    return b"\x00" * 8 + struct.pack("<QQ", type_id, count) + b"\x00" * 8


def header(file_id: int, type_id: int, toc_data_offset: int, toc_data_size: int) -> bytes:
    return struct.pack(
        "<QQQQQQQIIIIII",
        file_id,
        type_id,
        toc_data_offset,
        0,  # stream_file_offset
        0,  # gpu_resource_offset
        0,
        0,
        toc_data_size,
        0,
        0,
        0,
        0,
        0,
    )


def unit_block(size, version_at_2c, lod_group_offset, group_size, *, legacy=False, seed=0):
    """Build a synthetic unit resource block.

    Layout the engine cares about:
      0x2C u32  version (read to decide the legacy layout branch)
      0x30 u32  lod_group_offset
      0x34 u32  joint_list_offset  (also entry 0 of the 16-entry offset table)
      0x34..0x74  16 u32 offset table (entries > lod_group_offset get shifted)
      0x5C u32  layout_list_offset  (only used on the legacy branch)
      [lod_group_offset .. lod_group_offset+group_size]  the lod group data
    """
    b = filler(size, seed)
    joint_list_offset = lod_group_offset + group_size
    struct.pack_into("<I", b, 0x2C, version_at_2c)
    struct.pack_into("<I", b, 0x30, lod_group_offset)
    # 16-entry offset table at 0x34; entry 0 is joint_list_offset.
    table = [joint_list_offset, 0, 0x40, lod_group_offset + 0x20, 0, 0x08, 0, lod_group_offset + 0x30]
    table += [0] * (16 - len(table))
    for i, val in enumerate(table):
        struct.pack_into("<I", b, 0x34 + i * 4, val)
    # mark the lod group region so shifts are visible
    for i in range(group_size):
        b[lod_group_offset + i] = 0xAA

    if legacy:
        layout_list_offset = 0xA0
        struct.pack_into("<I", b, 0x5C, layout_list_offset)
        struct.pack_into("<I", b, layout_list_offset, 1)  # num_layouts = 1
        struct.pack_into("<I", b, layout_list_offset + 4, 0x10)  # layout_offsets[0]
        # 16 items of 20 bytes starting at layout_list_offset+0x10+8; leave the
        # filler-provided item formats as-is (a mix of >16 and <=16).
    return bytes(b)


class PatchBuilder:
    def __init__(self):
        self.types = []  # (type_id, count)
        self.headers = []  # dict with file_id, type_id, block, size_override
        self.blocks = []  # (file_id, bytes)

    def add_type(self, type_id, count):
        self.types.append((type_id, count))

    def add_unit(self, file_id, block, *, offset_override=None):
        self.headers.append(
            {"file_id": file_id, "type_id": UNIT_TYPE_ID, "offset_override": offset_override}
        )
        self.blocks.append((file_id, block))

    def add_nonunit(self, file_id, type_id, block):
        self.headers.append(
            {"file_id": file_id, "type_id": type_id, "offset_override": None}
        )
        self.blocks.append((file_id, block))

    def build(self):
        num_types = len(self.types)
        num_files = len(self.headers)
        toc_start = MAIN_HEADER_SIZE + TYPE_ENTRY_SIZE * num_types
        data_start = toc_start + HEADER_SIZE * num_files

        # lay blocks out consecutively in the data region
        offsets = {}
        cursor = data_start
        block_map = dict(self.blocks)
        for fid, blk in self.blocks:
            offsets[fid] = cursor
            cursor += len(blk)
        file_size = cursor

        out = bytearray()
        out += main_header(num_types, num_files)
        for tid, count in self.types:
            out += type_entry(tid, count)
        for h in self.headers:
            fid = h["file_id"]
            off = h["offset_override"] if h["offset_override"] is not None else offsets[fid]
            out += header(fid, h["type_id"], off, len(block_map[fid]))
        for fid, blk in self.blocks:
            out += blk
        assert len(out) == file_size
        return bytes(out)


def write_case(name, patch_bytes, units, expected_outcome):
    """units: {file_id: (version_bytes, lod_group_bytes)}; runs the reference engine."""
    case_dir = FIXTURES / name
    case_dir.mkdir(parents=True, exist_ok=True)

    input_path = case_dir / "input.patch"
    input_path.write_bytes(patch_bytes)

    # point the reference engine at our synthetic units
    uum.game_resource_mapping = {fid: None for fid in units}
    uum.get_data_from_original_file = lambda fid: (
        units[fid][0],
        units[fid][1],
        len(units[fid][1]),
    )

    # run on a copy so input.patch stays pristine
    work_path = case_dir / "work.patch"
    work_path.write_bytes(patch_bytes)
    code, _ = uum.update_patch_file(str(work_path))

    code_name = {
        uum.UPDATE_SUCCESS: "updated",
        uum.NO_UNIT_FILES: "no_units",
        uum.CORRUPTED_FILE: "corrupted",
    }[code]
    assert code_name == expected_outcome, f"{name}: expected {expected_outcome}, got {code_name}"

    meta = {"outcome": code_name}
    (case_dir / "meta.json").write_text(json.dumps(meta, indent=2))

    resources = {
        str(fid): {"version": ver.hex(), "lod_group": lod.hex()}
        for fid, (ver, lod) in units.items()
    }
    (case_dir / "resources.json").write_text(json.dumps({"units": resources}, indent=2))

    if code_name == "updated":
        (case_dir / "expected.patch").write_bytes(work_path.read_bytes())
    else:
        # no output file for non-updated cases
        (case_dir / "expected.patch").unlink(missing_ok=True)
    work_path.unlink(missing_ok=True)
    print(f"  {name}: {code_name} ({len(patch_bytes)} bytes in)")


VERSION = b"\x01\x02\x03\x04"


def build_cases():
    print("Generating golden fixtures...")

    # 1. no units
    b = PatchBuilder()
    b.add_type(OTHER_TYPE_ID, 1)
    b.add_nonunit(0x1111, OTHER_TYPE_ID, unit_block(0x100, 0xFFFFFFFF, 0x80, 0x10))
    write_case("no_units", b.build(), {}, "no_units")

    # 2. unit grow
    b = PatchBuilder()
    b.add_type(UNIT_TYPE_ID, 1)
    b.add_unit(0xAAAA, unit_block(0x100, 0xFFFFFFFF, 0x80, 0x10))
    write_case("unit_grow", b.build(), {0xAAAA: (VERSION, bytes(filler(0x28, 5)))}, "updated")

    # 3. unit shrink
    b = PatchBuilder()
    b.add_type(UNIT_TYPE_ID, 1)
    b.add_unit(0xAAAA, unit_block(0x100, 0xFFFFFFFF, 0x80, 0x10))
    write_case("unit_shrink", b.build(), {0xAAAA: (VERSION, bytes(filler(0x08, 9)))}, "updated")

    # 4. unit same size
    b = PatchBuilder()
    b.add_type(UNIT_TYPE_ID, 1)
    b.add_unit(0xAAAA, unit_block(0x100, 0xFFFFFFFF, 0x80, 0x10))
    write_case("unit_same", b.build(), {0xAAAA: (VERSION, bytes(filler(0x10, 3)))}, "updated")

    # 5. header deletion (one unit id unknown to the game data)
    b = PatchBuilder()
    b.add_type(UNIT_TYPE_ID, 2)
    b.add_unit(0xBBBB, unit_block(0x100, 0xFFFFFFFF, 0x80, 0x10, seed=1))  # will be deleted
    b.add_unit(0xAAAA, unit_block(0x100, 0xFFFFFFFF, 0x80, 0x10, seed=2))  # kept + updated
    write_case("header_delete", b.build(), {0xAAAA: (VERSION, bytes(filler(0x20, 7)))}, "updated")

    # 6. legacy layout branch (version < 0xA4CD36)
    b = PatchBuilder()
    b.add_type(UNIT_TYPE_ID, 1)
    b.add_unit(0xAAAA, unit_block(0x400, 0x100, 0x200, 0x10, legacy=True))
    write_case("legacy_layout", b.build(), {0xAAAA: (VERSION, bytes(filler(0x18, 4)))}, "updated")

    # 7. two units, both grow (exercises size_offset accumulation)
    b = PatchBuilder()
    b.add_type(UNIT_TYPE_ID, 2)
    b.add_unit(0xA1, unit_block(0x100, 0xFFFFFFFF, 0x80, 0x10, seed=11))
    b.add_unit(0xA2, unit_block(0x100, 0xFFFFFFFF, 0x80, 0x10, seed=12))
    write_case(
        "two_units",
        b.build(),
        {0xA1: (VERSION, bytes(filler(0x24, 1))), 0xA2: (VERSION, bytes(filler(0x30, 2)))},
        "updated",
    )

    # 8. corrupted: a type id < 2**32
    b = PatchBuilder()
    b.add_type(SMALL_TYPE_ID, 1)
    b.add_nonunit(0x1, SMALL_TYPE_ID, unit_block(0x80, 0xFFFFFFFF, 0x40, 0x08))
    write_case("corrupted_smalltype", b.build(), {}, "corrupted")

    # 9. corrupted: header points past end of file
    b = PatchBuilder()
    b.add_type(UNIT_TYPE_ID, 1)
    b.add_unit(0xAAAA, unit_block(0x100, 0xFFFFFFFF, 0x80, 0x10), offset_override=0xFFFFFFF)
    write_case("corrupted_offset", b.build(), {0xAAAA: (VERSION, bytes(filler(0x10)))}, "corrupted")

    print("Done.")


if __name__ == "__main__":
    build_cases()
