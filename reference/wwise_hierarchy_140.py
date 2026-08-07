"""
Bank Version 140

Trimmed, phase-scoped port of hd2-audio-modder/wwise_hierarchy_140.py. Only the
opaque passthrough path (`HircEntry`) and the `WwiseHierarchy_140` container are
ported so far — structured HIRC types (Sound, MusicTrack, MusicSegment, the
container types) are added in later phases as the Rust port grows to cover
them. Until then, every hierarchy entry round-trips as `(type, size, id,
misc-bytes)`, matching how the real tool already treats every *unhandled*
HIRC type.
"""

import copy

from audio_util import MemoryStream


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


class HircEntryFactory:
    @classmethod
    def from_memory_stream(cls, stream: MemoryStream):
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
        return []

    def get_music_tracks(self):
        return []

    def has_entry(self, entry_id):
        return entry_id in self.entries

    def get_entry(self, entry_id):
        return self.entries[entry_id]

    def get_data(self):
        arr = [entry.get_data() for entry in self.entries.values()]
        return len(arr).to_bytes(4, byteorder="little") + b"".join(arr)
