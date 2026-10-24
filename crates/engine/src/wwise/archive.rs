//! Port of `GameArchive`, `TocHeader`, and `WwiseBank` — the TOC-level
//! read/write skeleton. A `GameArchive` is either a base game archive or a
//! mod `.patch_N` file; both share this exact on-disk shape.

use std::collections::HashSet;
use std::path::Path;

use indexmap::IndexMap;

use super::bank::rev_audio_media_index_id;
pub use super::bank::WwiseBank;
use crate::memstream::MemoryStream;
use crate::wwise::audio_source::{
    AudioSource, WwiseDep, WwiseStream, STREAM_TYPE_BANK, STREAM_TYPE_PREFETCH, STREAM_TYPE_STREAM,
};
use crate::wwise::bank_parser::{BankParser, MediaIndex};
use crate::wwise::hierarchy::{BankVersion, HircEntry, WwiseHierarchy};
use crate::wwise::text_bank::TextBank;
use crate::wwise::video::VideoSource;
use crate::wwise::{murmur64_hash, BANK_VERSION_KEY, REV_AUDIO, VORBIS};
use crate::{BINK_VIDEO, TEXT_BANK, WWISE_BANK, WWISE_DEP, WWISE_STREAM};

/// `os.path.dirname` for the POSIX-style `/`-separated virtual paths used by
/// bank dependency strings (never real filesystem paths, so `std::path::Path`
/// — which is host-OS-semantics-dependent — would be the wrong tool here).
/// Matches CPython's `posixpath.dirname` exactly, including its trailing-slash
/// collapsing, since a wrong dirname would silently mis-hash a stream lookup.
fn posix_dirname(path: &str) -> &str {
    match path.rfind('/') {
        None => "",
        Some(i) => {
            let head = &path[..=i];
            if head.bytes().all(|b| b == b'/') {
                head
            } else {
                head.trim_end_matches('/')
            }
        }
    }
}

const ARCHIVE_MAGIC: u32 = 0xF000_0011;

/// Port of `TocHeader`: the 80-byte per-resource TOC entry.
#[derive(Debug, Clone)]
pub struct TocHeader {
    pub file_id: u64,
    pub type_id: u64,
    pub toc_data_offset: u64,
    pub stream_file_offset: u64,
    pub gpu_resource_offset: u64,
    pub unknown1: u64,
    pub unknown2: u64,
    pub toc_data_size: u32,
    pub stream_size: u32,
    pub gpu_resource_size: u32,
    pub unknown3: u32,
    pub unknown4: u32,
    pub entry_index: u32,
}

impl Default for TocHeader {
    fn default() -> Self {
        TocHeader {
            file_id: 0,
            type_id: 0,
            toc_data_offset: 0,
            stream_file_offset: 0,
            gpu_resource_offset: 0,
            unknown1: 0,
            unknown2: 0,
            toc_data_size: 0,
            stream_size: 0,
            gpu_resource_size: 0,
            unknown3: 16,
            unknown4: 64,
            entry_index: 0,
        }
    }
}

impl TocHeader {
    pub fn read(stream: &mut MemoryStream) -> Self {
        TocHeader {
            file_id: stream.read_u64(),
            type_id: stream.read_u64(),
            toc_data_offset: stream.read_u64(),
            stream_file_offset: stream.read_u64(),
            gpu_resource_offset: stream.read_u64(),
            unknown1: stream.read_u64(),
            unknown2: stream.read_u64(),
            toc_data_size: stream.read_u32(),
            stream_size: stream.read_u32(),
            gpu_resource_size: stream.read_u32(),
            unknown3: stream.read_u32(),
            unknown4: stream.read_u32(),
            entry_index: stream.read_u32(),
        }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(80);
        out.extend_from_slice(&self.file_id.to_le_bytes());
        out.extend_from_slice(&self.type_id.to_le_bytes());
        out.extend_from_slice(&self.toc_data_offset.to_le_bytes());
        out.extend_from_slice(&self.stream_file_offset.to_le_bytes());
        out.extend_from_slice(&self.gpu_resource_offset.to_le_bytes());
        out.extend_from_slice(&self.unknown1.to_le_bytes());
        out.extend_from_slice(&self.unknown2.to_le_bytes());
        out.extend_from_slice(&self.toc_data_size.to_le_bytes());
        out.extend_from_slice(&self.stream_size.to_le_bytes());
        out.extend_from_slice(&self.gpu_resource_size.to_le_bytes());
        out.extend_from_slice(&self.unknown3.to_le_bytes());
        out.extend_from_slice(&self.unknown4.to_le_bytes());
        out.extend_from_slice(&self.entry_index.to_le_bytes());
        out
    }
}

/// Port of `GameArchive`.
#[derive(Debug, Default, Clone)]
pub struct GameArchive {
    pub magic: u32,
    pub name: String,
    pub unknown: u32,
    pub unk4_data: Vec<u8>,
    pub wwise_streams: IndexMap<u64, WwiseStream>,
    pub wwise_banks: IndexMap<u64, WwiseBank>,
    pub audio_sources: IndexMap<u64, AudioSource>,
    pub hierarchy_entries: IndexMap<u32, HircEntry>,
    pub video_sources: IndexMap<u64, VideoSource>,
    pub text_banks: IndexMap<u64, TextBank>,
}

impl GameArchive {
    /// `GameArchive.from_file`, generalized over how the toc/stream bytes
    /// were obtained (a raw file read for mod `.patch_N` files, or a `Slim`
    /// package fetch for a base game archive — see `crate::slim`). Returns
    /// `None` only when `toc_data` is empty ("package/file has no
    /// content"), matching the Python; a non-empty `toc_data` with the wrong
    /// magic still produces an (empty) archive, matching `load`'s early
    /// return being silently ignored by `from_file`.
    pub fn from_toc_and_stream(name: String, toc_data: &[u8], stream_data: &[u8]) -> Option<GameArchive> {
        if toc_data.is_empty() {
            return None;
        }
        let mut archive = GameArchive { name, ..Default::default() };
        archive.load(toc_data, stream_data);
        Some(archive)
    }

    /// Loads a mod `.patch_N` file (and its `.stream` companion, if any)
    /// directly from disk. Patch files are always plain/uncompressed, so —
    /// unlike a base game archive — no `Slim` decompression is involved.
    pub fn from_patch_file(path: &Path) -> Option<GameArchive> {
        let toc_data = std::fs::read(path).ok()?;
        let mut stream_path = path.as_os_str().to_owned();
        stream_path.push(".stream");
        let stream_data = std::fs::read(&stream_path).unwrap_or_default();
        let name = path.file_name()?.to_string_lossy().into_owned();
        GameArchive::from_toc_and_stream(name, &toc_data, &stream_data)
    }

    fn load(&mut self, toc_data: &[u8], stream_data: &[u8]) {
        self.wwise_streams.clear();
        self.wwise_banks.clear();
        self.audio_sources.clear();
        self.video_sources.clear();
        self.text_banks.clear();
        self.hierarchy_entries.clear();

        let mut toc = MemoryStream::new(toc_data.to_vec());
        toc.seek(0);
        self.magic = toc.read_u32();
        if self.magic != ARCHIVE_MAGIC {
            return;
        }
        let num_types = toc.read_u32();
        let num_files = toc.read_u32();
        self.unknown = toc.read_u32();
        self.unk4_data = toc.read(56).to_vec();
        toc.seek(toc.tell() + 32 * num_types as usize);
        let toc_start = toc.tell();

        // Accumulates every bank's DIDX/DATA across the whole archive (a
        // Sound in one bank can reference media that physically lives in
        // another bank's DIDX/DATA); used only after the loop, to resolve
        // `self.audio_sources`.
        let mut archive_media_index = MediaIndex::default();

        for n in 0..num_files as usize {
            toc.seek(toc_start + n * 80);
            let header = TocHeader::read(&mut toc);

            if header.type_id == WWISE_STREAM {
                toc.seek(header.toc_data_offset as usize);
                let mut resource_header = [0; 8];
                resource_header.copy_from_slice(toc.read(8));
                let start = header.stream_file_offset as usize;
                let end = start + header.stream_size as usize;
                let mut audio = AudioSource { stream_type: STREAM_TYPE_STREAM, ..Default::default() };
                audio.set_data(stream_data[start..end].to_vec(), false);
                audio.resource_id = header.file_id;
                let wstream =
                    WwiseStream { file_id: header.file_id, resource_header, audio_source: audio, modified: false };
                self.wwise_streams.insert(wstream.id(), wstream);
            } else if header.type_id == WWISE_BANK {
                toc.seek(header.toc_data_offset as usize);
                let resource_tag = toc.read_u32();
                toc.advance(12);
                let bank_bytes = toc.read((header.toc_data_size - 16) as usize).to_vec();
                let parser = BankParser::load(&bank_bytes);

                let bkhd = parser.get_chunk("BKHD").to_vec();
                let mut bank_header = Vec::with_capacity(8 + bkhd.len());
                bank_header.extend_from_slice(b"BKHD");
                bank_header.extend_from_slice(&(bkhd.len() as u32).to_le_bytes());
                bank_header.extend_from_slice(&bkhd);

                let bank_version_raw = u32::from_le_bytes(bkhd[0..4].try_into().unwrap());
                let version =
                    if (bank_version_raw ^ BANK_VERSION_KEY) == 154 { BankVersion::V154 } else { BankVersion::V140 };

                let hierarchy = if parser.chunks.contains_key("HIRC") {
                    WwiseHierarchy::load(version, parser.get_chunk("HIRC"))
                } else {
                    WwiseHierarchy::new(version)
                };
                // `GameArchive.load`'s cross-bank dedup (`core.py:818-829`):
                // a `hierarchy_id` shared by more than one bank in this
                // archive gets its children unioned into the first-seen
                // copy rather than overwritten. Python then reassigns every
                // sharing bank's own `hierarchy.entries[id]` to point at
                // that same (mutated) object; this port instead backfills
                // the final merged state into every bank's own entry map
                // once the whole TOC has been scanned (below), since
                // `HircEntry` is cloned rather than aliased here.
                for (id, entry) in &hierarchy.entries {
                    if let Some(existing) = self.hierarchy_entries.get_mut(id) {
                        existing.merge_children(entry);
                    } else {
                        self.hierarchy_entries.insert(*id, entry.clone());
                    }
                }

                let media_index = if parser.chunks.contains_key("DIDX") {
                    let didx = parser.get_chunk("DIDX");
                    let data = parser.get_chunk("DATA");
                    archive_media_index.load(didx, data);
                    let mut bank_media_index = MediaIndex::default();
                    bank_media_index.load(didx, data);
                    Some(bank_media_index.entries.keys().copied().collect::<Vec<u32>>())
                } else {
                    None
                };

                let mut bank_misc_data = Vec::new();
                for (tag, payload) in &parser.chunks {
                    if !matches!(tag.as_str(), "BKHD" | "DATA" | "DIDX" | "HIRC") {
                        bank_misc_data.extend_from_slice(tag.as_bytes());
                        bank_misc_data.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                        bank_misc_data.extend_from_slice(payload);
                    }
                }

                let dep = WwiseDep::synthesized(header.file_id, header.file_id);
                let bank = WwiseBank {
                    file_id: header.file_id,
                    resource_tag,
                    bank_header,
                    bank_misc_data,
                    modified: false,
                    modified_count: 0,
                    dep,
                    hierarchy,
                    media_index,
                };
                self.wwise_banks.insert(bank.id(), bank);
            } else if header.type_id == WWISE_DEP {
                toc.seek(header.toc_data_offset as usize);
                let dep = WwiseDep::read(&mut toc, header.file_id);
                if let Some(bank) = self.wwise_banks.get_mut(&header.file_id) {
                    bank.dep = dep;
                }
            } else if header.type_id == TEXT_BANK {
                toc.seek(header.toc_data_offset as usize);
                let data = toc.read(header.toc_data_size as usize).to_vec();
                let mut tb = TextBank::new(header.file_id);
                tb.set_data(&data);
                self.text_banks.insert(tb.id(), tb);
            } else if header.type_id == BINK_VIDEO {
                let start = header.stream_file_offset as usize;
                let end = start + header.stream_size as usize;
                let video =
                    VideoSource::from_original(header.file_id, header.stream_size, stream_data[start..end].to_vec());
                self.video_sources.insert(video.id(), video);
            }
        }

        // Backfill each bank's own hierarchy with the final cross-bank-merged
        // entries (see the comment above), so that a bank whose copy of a
        // shared `hierarchy_id` was the first-seen one still regenerates
        // with the full unioned children set on `to_file`/`generate`.
        for bank in self.wwise_banks.values_mut() {
            for (id, entry) in bank.hierarchy.entries.iter_mut() {
                if let Some(canonical) = self.hierarchy_entries.get(id) {
                    *entry = canonical.clone();
                }
            }
        }

        self.resolve_audio_sources(&archive_media_index);
    }

    /// `_create_all_audio_source_objects` + `_create_audio_source*`:
    /// resolves every `Sound`'s and `MusicTrack`'s source across every bank
    /// to its actual audio bytes and populates `self.audio_sources`, keyed
    /// by `short_id` — not `AudioSource::id()`;
    /// Python always keys this particular map by `short_id`, even for
    /// STREAM-origin sources where `id()` would use `resource_id` instead.
    ///
    /// Resolution is collected into a local `Vec` and inserted in one pass
    /// at the end rather than interleaved (as Python does, inserting into
    /// `self.audio_sources` as each source resolves) — behaviorally
    /// equivalent here since `seen` tracks the same dedup key
    /// (`source.source_id`) in the same iteration order, and nothing during
    /// resolution reads `self.audio_sources` back.
    ///
    /// Note: for a STREAM/PREFETCH_STREAM-origin source, Python's
    /// `self.audio_sources[short_id]` and `wwise_streams[id].audio_source`
    /// end up being the *same* object (aliased), so mutating one through
    /// `audio_sources` is visible via the other. This port clones instead,
    /// so that aliasing doesn't carry over — a real gap for whichever phase
    /// ports `import_patch`'s audio-byte mutation if it turns out to rely on
    /// mutating through `audio_sources` for STREAM-backed sources; harmless
    /// for now since nothing yet mutates through this map, and BANK-type
    /// sources (what `generate`'s DIDX/DATA loop actually reads) have no
    /// second copy to desync from in the first place.
    fn resolve_audio_sources(&mut self, media_index: &MediaIndex) {
        let mut resolved: Vec<AudioSource> = Vec::new();
        let mut seen: HashSet<u32> = HashSet::new();

        for bank in self.wwise_banks.values() {
            // Matches Python's `get_sounds() + get_music_tracks()`.
            let sound_sources = bank.hierarchy.sounds().into_iter().map(|s| &s.sources);
            let track_sources = bank.hierarchy.music_tracks().into_iter().map(|m| &m.sources);
            for sources in sound_sources.chain(track_sources) {
                for source in sources {
                    if seen.contains(&source.source_id) {
                        continue;
                    }
                    if source.plugin_id != VORBIS && source.plugin_id != REV_AUDIO {
                        continue;
                    }
                    let is_bank = source.stream_type as u32 == STREAM_TYPE_BANK;
                    let is_stream = source.stream_type as u32 == STREAM_TYPE_STREAM
                        || source.stream_type as u32 == STREAM_TYPE_PREFETCH;
                    if !is_bank && !is_stream {
                        continue;
                    }

                    let audio = if is_bank && source.plugin_id == REV_AUDIO {
                        let Some(entry) = bank.hierarchy.get_entry(source.source_id) else {
                            continue;
                        };
                        let Some(media_index_id) = rev_audio_media_index_id(&entry.get_data()) else {
                            continue;
                        };
                        let Some(data) = media_index.data.get(&media_index_id) else {
                            continue;
                        };
                        AudioSource {
                            stream_type: STREAM_TYPE_BANK,
                            short_id: media_index_id,
                            data: data.clone(),
                            ..Default::default()
                        }
                    } else if is_bank {
                        let Some(data) = media_index.data.get(&source.source_id) else {
                            continue;
                        };
                        AudioSource {
                            stream_type: STREAM_TYPE_BANK,
                            short_id: source.source_id,
                            data: data.clone(),
                            ..Default::default()
                        }
                    } else {
                        // is_stream, regardless of plugin_id (VORBIS or REV_AUDIO) — matching Python.
                        let dir = posix_dirname(&bank.dep.data);
                        let stream_resource_id = murmur64_hash(format!("{dir}/{}", source.source_id).as_bytes());
                        let Some(stream) = self.wwise_streams.get(&stream_resource_id) else {
                            continue;
                        };
                        let mut audio = stream.audio_source.clone();
                        audio.short_id = source.source_id;
                        audio
                    };

                    seen.insert(source.source_id);
                    resolved.push(audio);
                }
            }
        }

        for audio in resolved {
            self.audio_sources.insert(audio.short_id as u64, audio);
        }
    }
}
