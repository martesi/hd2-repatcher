"""Trimmed, phase-scoped port of hd2-audio-modder/core.py for the golden-fixture
oracle. Dropped entirely: `SoundHandler` (playback) and everything GUI/
Wwise-project related. Kept: `GameArchive` and everything it touches
(`AudioSource`, `WwiseStream`, `WwiseBank`, `WwiseDep`, `TextBank`,
`StringEntry`, `VideoSource`, `BankParser`, `MediaIndex`, `TocHeader`) — the
read/write skeleton phase 1 of the native port targets — plus `Mod`
(`import_patch`/`write_patch`/`write_separate_patches`/`add_game_archive`/
`load_archive_file`), trimmed per `.ref/native-audio-patch-plan.md`'s phase 5
scope (see `Mod`'s own docstring below for exactly what's dropped).
"""

import os
import struct
from typing import Literal

from audio_const import *
from audio_util import *
import wwise_hierarchy_140
import wwise_hierarchy_154
from wwise_hierarchy_154 import WwiseHierarchy_154
from wwise_hierarchy_140 import WwiseHierarchy_140
from slim import load_package


class VideoSource:
    def __init__(self):
        self.replacement_filepath: str = ""
        self.filepath: str = ""
        self.video_size: int = 0
        self.replacement_video_size: int = 0
        self.replacement_video_offset: int = 0
        self.stream_offset: int = 0
        self.file_id: int = 0
        self.modified: bool = False

    def revert_modifications(self):
        self.modified = False

    def get_data(self):
        if not self.modified:
            _, _, stream_data = load_package(self.filepath)
            return stream_data[self.stream_offset : self.stream_offset + self.video_size]
        else:
            with open(self.replacement_filepath, "rb") as f:
                f.seek(self.replacement_video_offset)
                return f.read(self.replacement_video_size)

    def set_data(self, replacement_filepath: str):
        self.replacement_filepath = replacement_filepath
        self.replacement_video_size = os.path.getsize(replacement_filepath)
        self.modified = True

    def get_id(self):
        return self.file_id


class AudioSource:
    def __init__(self):
        self.data: bytearray | Literal[b""] = b""
        self.size: int = 0
        self.resource_id: int = 0
        self.short_id: int = 0
        self.modified: bool = False
        self.data_old: bytearray | Literal[b""] = b""
        self.parents = set()
        self.stream_type: int = 0
        self.muted = False

    def set_data(self, data, notify_subscribers: bool = True, set_modified: bool = True):
        if not self.modified and set_modified:
            self.data_old = self.data
        self.data = data
        self.size = len(self.data)
        if set_modified:
            self.modified = True

    def get_id(self) -> int:
        if self.stream_type == BANK:
            return self.get_short_id()
        else:
            return self.get_resource_id()

    def is_modified(self) -> bool:
        return self.modified

    def get_data(self):
        if not self.muted:
            return bytearray() if self.data == b"" else self.data
        else:
            return b""

    def get_resource_id(self) -> int:
        return self.resource_id

    def get_short_id(self) -> int:
        return self.short_id

    def revert_modifications(self, notify_subscribers: bool = True):
        if self.modified:
            self.modified = False
            if self.data_old != b"":
                self.data = self.data_old
                self.data_old = b""
            self.size = len(self.data)


class TocHeader:
    def __init__(self):
        self.file_id = self.type_id = self.toc_data_offset = self.stream_file_offset = self.gpu_resource_offset = 0
        self.unknown1 = self.unknown2 = self.toc_data_size = self.stream_size = self.gpu_resource_size = 0
        self.unknown3 = 16
        self.unknown4 = 64
        self.entry_index = 0

    def from_memory_stream(self, stream: MemoryStream):
        self.file_id = stream.uint64_read()
        self.type_id = stream.uint64_read()
        self.toc_data_offset = stream.uint64_read()
        self.stream_file_offset = stream.uint64_read()
        self.gpu_resource_offset = stream.uint64_read()
        self.unknown1 = stream.uint64_read()
        self.unknown2 = stream.uint64_read()
        self.toc_data_size = stream.uint32_read()
        self.stream_size = stream.uint32_read()
        self.gpu_resource_size = stream.uint32_read()
        self.unknown3 = stream.uint32_read()
        self.unknown4 = stream.uint32_read()
        self.entry_index = stream.uint32_read()

    def get_data(self) -> bytes:
        return struct.pack(
            "<QQQQQQQIIIIII",
            self.file_id,
            self.type_id,
            self.toc_data_offset,
            self.stream_file_offset,
            self.gpu_resource_offset,
            self.unknown1,
            self.unknown2,
            self.toc_data_size,
            self.stream_size,
            self.gpu_resource_size,
            self.unknown3,
            self.unknown4,
            self.entry_index,
        )


class WwiseDep:
    def __init__(self):
        self.data: str = ""
        self.skip = True
        self.file_id = 0

    def from_memory_stream(self, stream: MemoryStream):
        self.offset = stream.tell()
        self.tag = stream.uint32_read()
        self.data_size = stream.uint32_read()
        self.skip = False
        self.data = stream.read(self.data_size).decode("utf-8")

    def get_data(self) -> bytes:
        return (
            self.tag.to_bytes(4, byteorder="little")
            + self.data_size.to_bytes(4, byteorder="little")
            + self.data.encode("utf-8")
        )


class DidxEntry:
    def __init__(self):
        self.id = self.offset = self.size = 0

    @classmethod
    def from_bytes(cls, bytes):
        e = DidxEntry()
        e.id, e.offset, e.size = struct.unpack("<III", bytes)
        return e

    def get_data(self) -> bytes:
        return struct.pack("<III", self.id, self.offset, self.size)


class MediaIndex:
    def __init__(self):
        self.entries = {}
        self.data = {}

    def load(self, didxChunk, dataChunk):
        for n in range(int(len(didxChunk) / 12)):
            entry = DidxEntry.from_bytes(didxChunk[12 * n : 12 * (n + 1)])
            self.entries[entry.id] = entry
            self.data[entry.id] = dataChunk[entry.offset : entry.offset + entry.size]

    def get_data(self) -> bytes:
        arr = [x.get_data() for x in self.entries.values()]
        data_arr = self.data.values()
        return b"".join(arr) + b"".join(data_arr)


class BankParser:
    def __init__(self):
        self.chunks = {}

    def load(self, bank_data):
        self.chunks.clear()
        reader = MemoryStream()
        reader.write(bank_data)
        reader.seek(0)
        while True:
            tag = ""
            try:
                tag = reader.read(4).decode("utf-8")
            except Exception:
                break
            size = reader.uint32_read()
            self.chunks[tag] = reader.read(size)

    def GetChunk(self, chunk_tag: str):
        try:
            return self.chunks[chunk_tag]
        except Exception:
            return bytearray()


class WwiseBank:
    def __init__(self):
        self.bank_header: bytes = b""
        self.bank_misc_data: bytes = b""
        self.modified: bool = False
        self.dep = None
        self.modified_count: int = 0
        self.hierarchy = None
        self.content = []
        self.media_index = None
        self.file_id: int = 0

    def import_hierarchy(self, new_hierarchy):
        """Trimmed: real upstream propagates `raise_modified()` per merged
        entry via a `soundbanks` back-reference this oracle's `HircEntry`
        trim drops (see wwise_hierarchy_154.py's module docstring).
        Behaviorally equivalent substitute used here (and mirrored by the
        Rust port): compare the hierarchy's serialized bytes before/after
        the merge and raise_modified() on any change — same net effect
        (this bank appears in write_patch's modified-filtered output
        whenever the merge actually changed something)."""
        before = self.hierarchy.get_data()
        self.hierarchy.import_hierarchy(new_hierarchy)
        if self.hierarchy.get_data() != before:
            self.raise_modified()

    def add_content(self, content: int):
        self.content.append(content)

    def get_content(self):
        return self.content

    def raise_modified(self):
        self.modified = True
        self.modified_count += 1

    def lower_modified(self):
        if self.modified:
            self.modified_count -= 1
            if self.modified_count == 0:
                self.modified = False

    def get_name(self) -> str:
        return self.dep.data

    def get_id(self) -> int:
        return self.file_id

    def generate(self, audio_sources) -> bytearray:
        data = bytearray()
        data += self.bank_header
        offset = 0

        didx_array = []
        data_array = []
        added_sources = set()

        entries = self.hierarchy.get_sounds() + self.hierarchy.get_music_tracks()
        for entry in entries:
            for source in entry.sources:
                if source.plugin_id == VORBIS:
                    if self.media_index is None:
                        continue
                    if self.media_index and source.source_id not in self.media_index:
                        continue
                    try:
                        audio = audio_sources[source.source_id]
                    except KeyError:
                        continue
                    if source.stream_type == PREFETCH_STREAM and source.source_id not in added_sources:
                        data_array.append(audio.get_data()[: source.mem_size])
                        didx_array.append(struct.pack("<III", source.source_id, offset, source.mem_size))
                        offset += source.mem_size
                        added_sources.add(source.source_id)
                    elif source.stream_type == BANK and source.source_id not in added_sources:
                        data_array.append(audio.get_data())
                        didx_array.append(struct.pack("<III", source.source_id, offset, audio.size))
                        offset += audio.size
                        added_sources.add(source.source_id)
                elif source.plugin_id == REV_AUDIO:
                    try:
                        custom_fx_entry = self.hierarchy.entries[source.source_id]
                        fx_data = custom_fx_entry.get_data()
                        plugin_param_size = int.from_bytes(fx_data[13:17], byteorder="little")
                        media_index_id = int.from_bytes(
                            fx_data[19 + plugin_param_size : 23 + plugin_param_size], byteorder="little"
                        )
                        audio = audio_sources[media_index_id]
                    except KeyError:
                        continue
                    if source.stream_type == BANK and source.source_id not in added_sources:
                        data_array.append(audio.get_data())
                        didx_array.append(struct.pack("<III", media_index_id, offset, audio.size))
                        offset += audio.size
                        added_sources.add(media_index_id)

        if len(didx_array) > 0:
            data += "DIDX".encode("utf-8") + (12 * len(didx_array)).to_bytes(4, byteorder="little")
            data += b"".join(didx_array)
            data += "DATA".encode("utf-8") + sum([len(x) for x in data_array]).to_bytes(4, byteorder="little")
            data += b"".join(data_array)

        hierarchy_section = self.hierarchy.get_data()
        data += "HIRC".encode("utf-8") + len(hierarchy_section).to_bytes(4, byteorder="little")
        data += hierarchy_section
        data += self.bank_misc_data
        return data


class WwiseStream:
    def __init__(self):
        self.audio_source = None
        self.modified: bool = False
        self.file_id: int = 0

    def set_source(self, audio_source):
        self.audio_source = audio_source

    def raise_modified(self):
        self.modified = True

    def lower_modified(self):
        self.modified = False

    def get_id(self) -> int:
        return self.file_id

    def get_data(self):
        return self.audio_source.get_data()


class StringEntry:
    def __init__(self):
        self.text = ""
        self.text_old = ""
        self.string_id = 0
        self.modified = False
        self.parent = None

    def get_id(self) -> int:
        return self.string_id

    def get_text(self) -> str:
        return self.text

    def set_text(self, text: str):
        if not self.modified:
            self.text_old = self.text
            if self.parent is not None:
                self.parent.raise_modified()
        self.modified = True
        self.text = text

    def revert_modifications(self):
        if self.modified:
            self.text = self.text_old
            self.modified = False
            if self.parent is not None:
                self.parent.lower_modified()


class TextBank:
    def __init__(self):
        self.file_id = 0
        self.entries = {}
        self.language = 0
        self.modified = False
        self.modified_count = 0

    def set_data(self, data: bytearray):
        self.entries.clear()
        num_entries = int.from_bytes(data[8:12], byteorder="little")
        self.language = int.from_bytes(data[12:16], byteorder="little")
        id_section_start = 16
        offset_section_start = id_section_start + 4 * num_entries
        data_section_start = offset_section_start + 4 * num_entries
        ids = data[id_section_start:offset_section_start]
        offsets = data[offset_section_start:data_section_start]
        for n in range(num_entries):
            entry = StringEntry()
            entry.parent = self
            string_id = int.from_bytes(ids[4 * n : +4 * (n + 1)], byteorder="little")
            string_offset = int.from_bytes(offsets[4 * n : 4 * (n + 1)], byteorder="little")
            entry.string_id = string_id
            stopIndex = string_offset + 1
            while data[stopIndex] != 0:
                stopIndex += 1
            entry.text = data[string_offset:stopIndex].decode("utf-8")
            self.entries[string_id] = entry

    def revert_modifications(self, entry_id: int = 0):
        if entry_id:
            self.entries[entry_id].revert_modifications()
        else:
            for entry in self.entries.values():
                entry.revert_modifications()

    def get_language(self) -> int:
        return self.language

    def is_modified(self) -> bool:
        return self.modified

    def import_text(self, text_bank: "TextBank"):
        for new_string_entry in text_bank.entries.values():
            try:
                old_string_entry = self.entries[new_string_entry.string_id]
            except Exception:
                continue
            if (
                old_string_entry.modified
                and new_string_entry.get_text() != old_string_entry.text_old
                or (not old_string_entry.modified and new_string_entry.get_text() != old_string_entry.get_text())
            ):
                old_string_entry.set_text(new_string_entry.get_text())

    def generate(self) -> bytearray:
        stream = MemoryStream()
        stream.write(b"\xae\xf3\x85\x3e\x01\x00\x00\x00")
        stream.write(len(self.entries).to_bytes(4, byteorder="little"))
        stream.write(self.language.to_bytes(4, byteorder="little"))
        offset = 16 + 8 * len(self.entries)
        for entry in self.entries.values():
            stream.write(entry.get_id().to_bytes(4, byteorder="little"))
        for entry in self.entries.values():
            stream.write(offset.to_bytes(4, byteorder="little"))
            initial_position = stream.tell()
            stream.seek(offset)
            text_bytes = entry.text.encode("utf-8") + b"\x00"
            stream.write(text_bytes)
            offset += len(text_bytes)
            stream.seek(initial_position)
        return stream.data

    def get_id(self) -> int:
        return self.file_id

    def raise_modified(self):
        self.modified_count += 1
        self.modified = True

    def lower_modified(self):
        if self.modified:
            self.modified_count -= 1
            if self.modified_count == 0:
                self.modified = False


class GameArchive:
    def __init__(self):
        self.magic: int = -1
        self.name: str = ""
        self.num_files: int = -1
        self.num_types: int = -1
        self.path: str = ""
        self.unk4Data = b""
        self.unknown: int = -1
        self.wwise_streams = {}
        self.wwise_banks = {}
        self.audio_sources = {}
        self.hierarchy_entries = {}
        self.video_sources = {}
        self.text_banks = {}

    @classmethod
    def from_file(cls, path: str) -> "GameArchive":
        archive = GameArchive()
        archive.name = os.path.basename(path)
        archive.path = path

        toc_data, _, stream_data = load_package(path)
        if not toc_data:
            return None
        toc_file = MemoryStream(toc_data)
        stream_file = MemoryStream(stream_data)
        archive.load(toc_file, stream_file)
        return archive

    def get_wwise_streams(self):
        return self.wwise_streams

    def get_wwise_banks(self):
        return self.wwise_banks

    def get_audio_sources(self):
        return self.audio_sources

    def get_video_sources(self):
        return self.video_sources

    def get_text_banks(self):
        return self.text_banks

    def get_hierarchy_entries(self):
        return self.hierarchy_entries

    def write_type_header(self, toc_file: MemoryStream, entry_type: int, num_entries: int):
        if num_entries > 0:
            toc_file.write(struct.pack("<QQQII", 0, entry_type, num_entries, 16, 64))

    def to_file(self, path: str):
        toc_file = MemoryStream()
        stream_file = MemoryStream()
        wwise_deps = [bank.dep for bank in self.wwise_banks.values() if not bank.dep.skip]
        self.num_files = (
            len(self.wwise_streams) + len(self.wwise_banks) + len(wwise_deps) + len(self.text_banks) + len(self.video_sources)
        )
        self.num_types = 0
        for item in [self.wwise_streams, self.wwise_banks, self.text_banks, self.video_sources, wwise_deps]:
            if item:
                self.num_types += 1

        toc_file.write(struct.pack("<IIII56s", self.magic, self.num_types, self.num_files, self.unknown, self.unk4Data))

        self.write_type_header(toc_file, WWISE_STREAM, len(self.wwise_streams))
        self.write_type_header(toc_file, WWISE_BANK, len(self.wwise_banks))
        self.write_type_header(toc_file, WWISE_DEP, len(wwise_deps))
        self.write_type_header(toc_file, TEXT_BANK, len(self.text_banks))
        self.write_type_header(toc_file, BINK_VIDEO, len(self.video_sources))

        toc_data_offset = toc_file.tell() + 80 * self.num_files + 8
        stream_file_offset = 0

        toc_entries = []
        toc_data = []
        stream_data = []
        entry_index = 0

        for stream in self.wwise_streams.values():
            s_data = pad_to_16_byte_align(stream.get_data())
            t_data = bytes.fromhex("D82F767800000000") + struct.pack("<Q", len(stream.get_data()))
            toc_entry = TocHeader()
            toc_entry.file_id = stream.get_id()
            toc_entry.type_id = WWISE_STREAM
            toc_entry.toc_data_offset = toc_data_offset
            toc_entry.stream_file_offset = stream_file_offset
            toc_entry.toc_data_size = 0x0C
            toc_entry.stream_size = len(stream.get_data())
            toc_entry.entry_index = entry_index
            stream_data.append(s_data)
            toc_data.append(t_data)
            toc_entries.append(toc_entry)
            entry_index += 1
            stream_file_offset += len(s_data)
            toc_data_offset += 16

        for bank in self.wwise_banks.values():
            bank_data = bank.generate(self.audio_sources)
            toc_entry = TocHeader()
            toc_entry.file_id = bank.get_id()
            toc_entry.type_id = WWISE_BANK
            toc_entry.toc_data_offset = toc_data_offset
            toc_entry.stream_file_offset = stream_file_offset
            toc_entry.toc_data_size = len(bank_data) + 16
            toc_entry.entry_index = entry_index
            toc_entries.append(toc_entry)
            bank_data = b"".join(
                [
                    bytes.fromhex("D82F7678"),
                    len(bank_data).to_bytes(4, byteorder="little"),
                    bank.get_id().to_bytes(8, byteorder="little"),
                    pad_to_16_byte_align(bank_data),
                ]
            )
            toc_data.append(bank_data)

            toc_data_offset += len(bank_data)
            entry_index += 1

        for text_bank in self.text_banks.values():
            text_data = text_bank.generate()
            toc_entry = TocHeader()
            toc_entry.file_id = text_bank.get_id()
            toc_entry.type_id = TEXT_BANK
            toc_entry.toc_data_offset = toc_data_offset
            toc_entry.stream_file_offset = stream_file_offset
            toc_entry.toc_data_size = len(text_data)
            toc_entry.entry_index = entry_index
            text_data = pad_to_16_byte_align(text_data)

            toc_entries.append(toc_entry)
            toc_data.append(text_data)

            toc_data_offset += len(text_data)
            entry_index += 1

        for dep in wwise_deps:
            dep_data = dep.get_data()
            toc_entry = TocHeader()
            toc_entry.file_id = dep.file_id
            toc_entry.type_id = WWISE_DEP
            toc_entry.toc_data_offset = toc_data_offset
            toc_entry.stream_file_offset = stream_file_offset
            toc_entry.toc_data_size = len(dep_data)
            toc_entry.entry_index = entry_index
            toc_entries.append(toc_entry)
            dep_data = pad_to_16_byte_align(dep_data)
            toc_data.append(dep_data)

            toc_data_offset += len(dep_data)
            entry_index += 1

        for video in self.video_sources.values():
            data = video.get_data()
            s_data = pad_to_16_byte_align(data)
            t_data = bytes.fromhex("E9030000000000000000000000000000")
            toc_entry = TocHeader()
            toc_entry.file_id = video.file_id
            toc_entry.type_id = BINK_VIDEO
            toc_entry.toc_data_offset = toc_data_offset
            toc_entry.stream_file_offset = stream_file_offset
            toc_entry.toc_data_size = 0x10
            toc_entry.stream_size = len(data)
            toc_entry.entry_index = entry_index
            stream_data.append(s_data)
            toc_data.append(t_data)
            toc_entries.append(toc_entry)
            entry_index += 1
            stream_file_offset += len(s_data)
            toc_data_offset += 16

        toc_file.write(b"".join([entry.get_data() for entry in toc_entries]))
        toc_file.advance(8)
        toc_file.write(b"".join(toc_data))
        stream_file.write(b"".join(stream_data))

        with open(os.path.join(path, self.name), "w+b") as f:
            min_size = len(toc_entries) * 256
            if len(toc_file.data) < min_size:
                toc_file.write(bytearray(min_size - len(toc_file.data)))
            f.write(toc_file.data)

        if len(stream_file.data) > 0:
            with open(os.path.join(path, self.name + ".stream"), "w+b") as f:
                f.write(stream_file.data)

    def load(self, toc_file: MemoryStream, stream_file: MemoryStream):
        self.wwise_streams.clear()
        self.wwise_banks.clear()
        self.audio_sources.clear()
        self.video_sources.clear()
        self.text_banks.clear()
        self.hierarchy_entries.clear()

        media_index = MediaIndex()

        self.magic = toc_file.uint32_read()
        if self.magic != 0xF0000011:
            return False

        self.num_types = toc_file.uint32_read()
        self.num_files = toc_file.uint32_read()
        self.unknown = toc_file.uint32_read()
        self.unk4Data = toc_file.read(56)
        toc_file.seek(toc_file.tell() + 32 * self.num_types)
        toc_start = toc_file.tell()
        for n in range(self.num_files):
            toc_file.seek(toc_start + n * 80)
            toc_header = TocHeader()
            toc_header.from_memory_stream(toc_file)
            if toc_header.type_id == WWISE_STREAM:
                audio = AudioSource()
                audio.stream_type = STREAM
                entry = WwiseStream()
                entry.file_id = toc_header.file_id
                toc_file.seek(toc_header.toc_data_offset)
                stream_file.seek(toc_header.stream_file_offset)
                audio.set_data(stream_file.read(toc_header.stream_size), notify_subscribers=False, set_modified=False)
                audio.resource_id = toc_header.file_id
                entry.set_source(audio)
                self.wwise_streams[entry.get_id()] = entry
            elif toc_header.type_id == WWISE_BANK:
                entry = WwiseBank()
                toc_file.seek(toc_header.toc_data_offset)
                toc_file.advance(16)
                entry.file_id = toc_header.file_id
                bank = BankParser()
                bank.load(toc_file.read(toc_header.toc_data_size - 16))
                entry.bank_header = (
                    "BKHD".encode("utf-8") + len(bank.chunks["BKHD"]).to_bytes(4, byteorder="little") + bank.chunks["BKHD"]
                )
                bank_version = int.from_bytes(bank.chunks["BKHD"][0:4], "little") ^ BANK_VERSION_KEY
                if bank_version == 154:
                    hirc = WwiseHierarchy_154(soundbank=entry)
                else:
                    hirc = WwiseHierarchy_140(soundbank=entry)
                try:
                    hirc.load(bank.chunks["HIRC"])
                except KeyError:
                    pass
                # `GameArchive.load`'s cross-bank dedup (verified against
                # real upstream `core.py:818-837`): a `hierarchy_id` shared
                # by more than one bank in this archive gets its children
                # unioned into the first-seen copy (for the five container
                # types), and every sharing bank's own `hirc.entries[id]`
                # gets reassigned to that same (shared) object so later
                # mutations/pruning stay in sync across banks.
                replacements = {}
                for hirc_id, hirc_entry in hirc.entries.items():
                    if hirc_id in self.hierarchy_entries:
                        existing_entry = self.hierarchy_entries[hirc_id]
                        if isinstance(
                            hirc_entry,
                            (
                                wwise_hierarchy_140.ActorMixer,
                                wwise_hierarchy_140.SwitchContainer,
                                wwise_hierarchy_140.RandomSequenceContainer,
                                wwise_hierarchy_140.LayerContainer,
                                wwise_hierarchy_140.MusicSwitchContainer,
                                wwise_hierarchy_154.ActorMixer,
                                wwise_hierarchy_154.SwitchContainer,
                                wwise_hierarchy_154.RandomSequenceContainer,
                                wwise_hierarchy_154.LayerContainer,
                                wwise_hierarchy_154.MusicSwitchContainer,
                            ),
                        ):
                            for child in hirc_entry.children.children:
                                if child not in existing_entry.children.children:
                                    existing_entry.children.children.append(child)
                                    existing_entry.size += 4
                        replacements[hirc_id] = existing_entry
                    else:
                        self.hierarchy_entries[hirc_id] = hirc_entry
                hirc.entries.update(replacements)
                entry.hierarchy = hirc

                if "DIDX" in bank.chunks.keys():
                    new_media_index = MediaIndex()
                    new_media_index.load(bank.chunks["DIDX"], bank.chunks["DATA"])
                    new_media_index.data = {}
                    entry.media_index = list(new_media_index.entries.keys())
                    media_index.load(bank.chunks["DIDX"], bank.chunks["DATA"])

                entry.bank_misc_data = b""
                for chunk in bank.chunks.keys():
                    if chunk not in ["BKHD", "DATA", "DIDX", "HIRC"]:
                        entry.bank_misc_data = (
                            entry.bank_misc_data + chunk.encode("utf-8") + len(bank.chunks[chunk]).to_bytes(4, byteorder="little") + bank.chunks[chunk]
                        )

                dep = WwiseDep()
                dep.skip = True
                dep.data = f"Bank {entry.get_id()}"
                dep.file_id = toc_header.file_id
                entry.dep = dep

                self.wwise_banks[entry.get_id()] = entry
            elif toc_header.type_id == WWISE_DEP:
                dep = WwiseDep()
                dep.file_id = toc_header.file_id
                toc_file.seek(toc_header.toc_data_offset)
                dep.from_memory_stream(toc_file)
                try:
                    self.wwise_banks[toc_header.file_id].dep = dep
                except KeyError:
                    pass
            elif toc_header.type_id == TEXT_BANK:
                toc_file.seek(toc_header.toc_data_offset)
                data = toc_file.read(toc_header.toc_data_size)
                text_bank = TextBank()
                text_bank.file_id = toc_header.file_id
                text_bank.set_data(data)
                self.text_banks[text_bank.get_id()] = text_bank
            elif toc_header.type_id == BINK_VIDEO:
                new_video_source = VideoSource()
                new_video_source.file_id = toc_header.file_id
                new_video_source.stream_offset = toc_header.stream_file_offset
                new_video_source.video_size = toc_header.stream_size
                new_video_source.filepath = self.path
                self.video_sources[new_video_source.file_id] = new_video_source

        self._create_all_audio_source_objects(media_index)

    def _create_all_audio_source_objects(self, media_index: MediaIndex):
        for bank in self.wwise_banks.values():
            self._create_all_audio_source_objects_from_bank(bank, media_index)

    def _create_all_audio_source_objects_from_bank(self, bank: WwiseBank, media_index: MediaIndex):
        hirc = bank.hierarchy
        dep = bank.dep

        entries_with_audio_sources = hirc.get_sounds() + hirc.get_music_tracks()
        for entry_with_audio_source in entries_with_audio_sources:
            for source_struct in entry_with_audio_source.sources:
                audio_source = self._create_audio_source(source_struct, media_index, hirc, dep)
                if audio_source is None:
                    continue
                self.audio_sources[audio_source.short_id] = audio_source

    def _create_audio_source(self, source, media_index: MediaIndex, hirc, dep):
        source_id = source.source_id
        if source_id in self.audio_sources:
            return None

        plugin_id = source.plugin_id
        if plugin_id not in [VORBIS, REV_AUDIO]:
            return None

        stream_type = source.stream_type
        if stream_type not in [BANK, STREAM, PREFETCH_STREAM]:
            return None

        if stream_type == BANK and plugin_id == REV_AUDIO:
            if not hirc.has_entry(source_id):
                return None
            return self._create_audio_source_type_rev_audio(hirc.get_entry(source_id), media_index)
        if stream_type == BANK:
            if source_id not in media_index.data:
                return None
            return self._create_audio_source_type_bank(source, media_index)
        if stream_type in [STREAM, PREFETCH_STREAM]:
            return self._create_audio_source_type_stream(source, dep)

        raise AssertionError("Invalid code path!")

    @staticmethod
    def _create_audio_source_type_bank(source, media_index: MediaIndex):
        audio = AudioSource()
        audio.stream_type = BANK
        audio.short_id = source.source_id
        audio.set_data(media_index.data[source.source_id], set_modified=False, notify_subscribers=False)
        return audio

    def _create_audio_source_type_rev_audio(self, custom_fx_entry, media_index: MediaIndex):
        data = custom_fx_entry.get_data()
        plugin_param_size = int.from_bytes(data[13:17], byteorder="little")

        plugin_data_start = 19 + plugin_param_size
        plugin_data_end = 23 + plugin_param_size

        media_index_id = int.from_bytes(data[plugin_data_start:plugin_data_end], byteorder="little")
        if media_index_id not in media_index.data:
            return None

        audio = AudioSource()
        audio.stream_type = BANK
        audio.short_id = media_index_id
        audio.set_data(media_index.data[media_index_id], set_modified=False, notify_subscribers=False)
        return audio

    def _create_audio_source_type_stream(self, source, dep):
        stream_resource_id = murmur64_hash((os.path.dirname(dep.data) + "/" + str(source.source_id)).encode("utf-8"))
        if stream_resource_id not in self.wwise_streams:
            return None

        audio = self.wwise_streams[stream_resource_id].audio_source
        if audio is None:
            return None
        audio.short_id = source.source_id
        return audio


class Mod:
    """Trimmed port of `core.py::Mod`. Dropped: `db` constructor argument
    (GUI undo/redo persistence only), per-resource `*_count` dicts (only
    needed by `remove_game_archive`, out of scope — see the plan doc),
    `import_wems`/`import_wavs`/`import_files`, `dump_*`, `create_dummy_bank`,
    hierarchy CRUD beyond `import_wwise_hierarchy`, `revert_*`. Kept: exactly
    what `import_patch`/`write_patch`/`write_separate_patches`/
    `add_game_archive`/`load_archive_file` need.

    `add_game_archive`'s `parents`-set reconciliation (real upstream
    reparents `AudioSource.parents`/`HircEntry.soundbanks` across the old vs.
    new owning bank) is dropped along with `parents`/`soundbanks` themselves
    (see `AudioSource`'s and `HircEntry`'s trims) — this GUI undo/redo-only
    bookkeeping is genuinely dead here. `parents` is NOT otherwise dead,
    though: real upstream also uses it, via `AudioSource.set_data`'s
    `notify_subscribers` path, to mark a bank modified when one of its
    Sound/MusicTrack sources gets new audio bytes — `import_patch` restores
    that specific effect below via a live scan instead (see the
    `swapped_ids` comment).
    """

    def __init__(self, name: str = ""):
        self.wwise_streams = {}
        self.wwise_banks = {}
        self.audio_sources = {}
        self.text_banks = {}
        self.video_sources = {}
        self.hierarchy_entries = {}
        self.game_archives = {}
        self.name = name

    def get_audio_source(self, audio_id: int):
        try:
            return self.audio_sources[audio_id]
        except KeyError:
            pass
        for source in self.audio_sources.values():
            if source.resource_id == audio_id:
                return source
        raise KeyError(f"Cannot find audio source with id {audio_id}")

    def get_wwise_bank(self, soundbank_id: int):
        try:
            return self.wwise_banks[soundbank_id]
        except KeyError:
            raise KeyError(f"Cannot find soundbank with id {soundbank_id}")

    def get_wwise_streams(self):
        return self.wwise_streams

    def get_wwise_banks(self):
        return self.wwise_banks

    def get_audio_sources(self):
        return self.audio_sources

    def get_text_banks(self):
        return self.text_banks

    def get_video_sources(self):
        return self.video_sources

    def get_video_source(self, file_id: int):
        try:
            return self.video_sources[file_id]
        except KeyError:
            raise KeyError(f"Cannot find video with id {file_id}")

    def get_hierarchy_entries(self):
        return self.hierarchy_entries

    def get_hierarchy_entry(self, hierarchy_id: int):
        return self.hierarchy_entries[hierarchy_id]

    def get_game_archives(self):
        return self.game_archives

    def load_archive_file(self, archive_file: str = ""):
        if os.path.splitext(archive_file)[1] in (".stream", ".gpu_resources"):
            archive_file = os.path.splitext(archive_file)[0]
        new_archive = GameArchive.from_file(archive_file)
        if not new_archive:
            return False
        key = new_archive.name
        if key in self.game_archives:
            return False
        self.add_game_archive(new_archive)
        return True

    def import_wwise_hierarchy(self, soundbank_id: int, new_hierarchy):
        self.get_wwise_bank(soundbank_id).import_hierarchy(new_hierarchy)

    def import_video(self, video_path: str, video_id: int):
        self.get_video_sources()[video_id].set_data(video_path)
        self.get_video_source(video_id).replacement_video_offset = 0

    def add_game_archive(self, game_archive: "GameArchive"):
        """`core.py::Mod.add_game_archive`, trimmed of `parents`/count-dict
        bookkeeping (see class docstring). Still faithful to real upstream
        for what matters here: video/hierarchy-entry/bank/stream/text-bank/
        audio-source de-dup against the already-pooled state, including the
        real, narrower-than-`GameArchive.load`'s-cross-bank-union
        ActorMixer-only children merge (`core.py:1907-1912`)."""
        key = game_archive.name
        if key in self.game_archives:
            return
        self.game_archives[key] = game_archive

        for vid_key, entry in game_archive.video_sources.items():
            if vid_key in self.video_sources:
                game_archive.video_sources[vid_key] = self.video_sources[vid_key]
            else:
                self.video_sources[vid_key] = entry

        replacements = {}
        for hid, entry in game_archive.get_hierarchy_entries().items():
            if hid in self.hierarchy_entries:
                existing_entry = self.hierarchy_entries[hid]
                replacements[hid] = existing_entry
                if isinstance(entry, (wwise_hierarchy_154.ActorMixer, wwise_hierarchy_140.ActorMixer)):
                    for child in entry.children.children:
                        if child not in existing_entry.children.children:
                            existing_entry.children.children.append(child)
                            existing_entry.size += 4
            else:
                self.hierarchy_entries[hid] = entry
        for bank in game_archive.wwise_banks.values():
            for hid, replacement in replacements.items():
                if hid in bank.hierarchy.entries:
                    bank.hierarchy.entries[hid] = replacement
        game_archive.get_hierarchy_entries().update(replacements)

        for bkey in list(game_archive.wwise_banks.keys()):
            if bkey in self.wwise_banks:
                game_archive.wwise_banks[bkey] = self.wwise_banks[bkey]
            else:
                self.wwise_banks[bkey] = game_archive.wwise_banks[bkey]
        for skey in list(game_archive.wwise_streams.keys()):
            if skey in self.wwise_streams:
                game_archive.wwise_streams[skey] = self.wwise_streams[skey]
            else:
                self.wwise_streams[skey] = game_archive.wwise_streams[skey]
        for tkey in list(game_archive.text_banks.keys()):
            if tkey in self.text_banks:
                game_archive.text_banks[tkey] = self.text_banks[tkey]
            else:
                self.text_banks[tkey] = game_archive.text_banks[tkey]
        for akey in list(game_archive.audio_sources.keys()):
            if akey in self.audio_sources:
                game_archive.audio_sources[akey] = self.audio_sources[akey]
            else:
                self.audio_sources[akey] = game_archive.audio_sources[akey]

    def import_patch(self, patch_file: str = "", import_hierarchy: bool = True) -> bool:
        if os.path.splitext(patch_file)[1] in (".stream", ".gpu_resources"):
            patch_file = os.path.splitext(patch_file)[0]
        if not os.path.exists(patch_file) or not os.path.isfile(patch_file):
            raise OSError("Invalid file!")

        patch_game_archive = GameArchive.from_file(patch_file)
        if patch_game_archive is None:
            return False

        swapped_ids = set()
        for new_audio in patch_game_archive.get_audio_sources().values():
            try:
                old_audio = self.get_audio_source(new_audio.get_short_id())
            except KeyError:
                continue
            if (
                not old_audio.modified
                and new_audio.get_data() != old_audio.get_data()
                or old_audio.modified
                and new_audio.get_data() != old_audio.data_old
            ):
                old_audio.set_data(new_audio.get_data())
                swapped_ids.add(new_audio.get_short_id())

        if swapped_ids:
            # Trimmed: real upstream discovers which banks to mark modified
            # via `AudioSource.parents`/`HircEntry.soundbanks` back-references
            # (`AudioSource.set_data`'s `notify_subscribers` path,
            # `core.py:76-90`) that this oracle's trim drops along with
            # `parents`/`soundbanks`. Behaviorally equivalent substitute: a
            # live scan for which banks' Sound/MusicTrack sources reference a
            # swapped id, same as the hierarchy-merge case above.
            for bank in self.get_wwise_banks().values():
                entries = bank.hierarchy.get_sounds() + bank.hierarchy.get_music_tracks()
                if any(src.source_id in swapped_ids for e in entries for src in e.sources):
                    bank.raise_modified()

            # Same gap, for streams: `old_audio.set_data` above already
            # mutated `self.wwise_streams[resource_id].audio_source` too
            # (same aliased object, per `_create_audio_source_type_stream`),
            # but real upstream's `notify_subscribers` path is what would
            # have raised the owning `WwiseStream.modified` flag, and this
            # oracle's trim drops that along with `parents`.
            for stream in self.get_wwise_streams().values():
                if stream.audio_source is not None and stream.audio_source.get_short_id() in swapped_ids:
                    stream.raise_modified()

        if import_hierarchy:
            for bank in patch_game_archive.get_wwise_banks().values():
                try:
                    self.import_wwise_hierarchy(bank.get_id(), bank.hierarchy)
                except Exception:
                    pass

        for text_bank in patch_game_archive.get_text_banks().values():
            try:
                self.get_text_banks()[text_bank.get_id()].import_text(text_bank)
            except Exception:
                pass

        add_patch = False
        for bank in list(patch_game_archive.get_wwise_banks().values()):
            if bank.get_id() in self.get_wwise_banks():
                del patch_game_archive.wwise_banks[bank.get_id()]
        if len(patch_game_archive.get_wwise_banks()) > 0:
            add_patch = True

        for video in list(patch_game_archive.get_video_sources().values()):
            has_video_source = False
            try:
                self.get_video_source(video.file_id)
                has_video_source = True
            except KeyError:
                pass

            if not has_video_source:
                video.modified = True
                video.replacement_video_offset = video.stream_offset
                video.replacement_video_size = video.video_size
                video.replacement_filepath = video.filepath + ".stream"
                add_patch = True
            else:
                try:
                    self.import_video(patch_file + ".stream", video.file_id)
                    self.get_video_source(video.file_id).replacement_video_offset = video.stream_offset
                except Exception:
                    pass
                del patch_game_archive.video_sources[video.file_id]

        if add_patch:
            patch_game_archive.text_banks.clear()
            self.add_game_archive(patch_game_archive)

        return True

    def write_patch(self, output_folder: str = "", output_filename: str = ""):
        patch_game_archive = GameArchive()
        patch_game_archive.name = "9ba626afa44a3aa3.patch_0" if output_filename == "" else output_filename
        patch_game_archive.magic = 0xF0000011
        patch_game_archive.num_types = 0
        patch_game_archive.num_files = 0
        patch_game_archive.unknown = 0
        patch_game_archive.unk4Data = bytes.fromhex(
            "CE09F5F4000000000C729F9E8872B8BD00A06B02000000000079510000000000000000000000000000000000000000000000000000000000"
        )
        patch_game_archive.audio_sources = self.audio_sources

        for key, value in self.get_wwise_streams().items():
            if value.modified:
                patch_game_archive.wwise_streams[key] = value
        for key, value in self.get_wwise_banks().items():
            if value.modified:
                patch_game_archive.wwise_banks[key] = value
        for key, value in self.get_text_banks().items():
            if value.modified:
                patch_game_archive.text_banks[key] = value
        for key, value in self.get_video_sources().items():
            if value.modified:
                patch_game_archive.video_sources[key] = value

        patch_game_archive.to_file(output_folder)

    def write_separate_patches(self, output_folder: str = ""):
        for archive in self.game_archives.values():
            patch_game_archive = GameArchive()
            patch_game_archive.name = f"{archive.name}.patch_0"
            patch_game_archive.magic = 0xF0000011
            patch_game_archive.num_types = 0
            patch_game_archive.num_files = 0
            patch_game_archive.unknown = archive.unknown
            patch_game_archive.unk4Data = archive.unk4Data
            patch_game_archive.audio_sources = archive.audio_sources

            for key, value in archive.get_wwise_streams().items():
                if value.modified:
                    patch_game_archive.wwise_streams[key] = value
            for key, value in archive.get_wwise_banks().items():
                if value.modified:
                    patch_game_archive.wwise_banks[key] = value
            for key, value in archive.get_text_banks().items():
                if value.modified:
                    patch_game_archive.text_banks[key] = value
            for key, value in archive.get_video_sources().items():
                if value.modified:
                    patch_game_archive.video_sources[key] = value

            patch_game_archive.to_file(output_folder)
