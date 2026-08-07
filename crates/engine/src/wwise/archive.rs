//! Port of `GameArchive`, `TocHeader`, and `WwiseBank` — the TOC-level
//! read/write skeleton. A `GameArchive` is either a base game archive or a
//! mod `.patch_N` file; both share this exact on-disk shape.

use std::path::Path;

use indexmap::IndexMap;

use crate::memstream::MemoryStream;
use crate::wwise::audio_source::{AudioSource, WwiseDep, WwiseStream, STREAM_TYPE_STREAM};
use crate::wwise::bank_parser::{BankParser, MediaIndex};
use crate::wwise::hierarchy::{BankVersion, HircEntry, WwiseHierarchy};
use crate::wwise::text_bank::TextBank;
use crate::wwise::video::VideoSource;
use crate::wwise::BANK_VERSION_KEY;
use crate::{BINK_VIDEO, TEXT_BANK, WWISE_BANK, WWISE_DEP, WWISE_STREAM};

const ARCHIVE_MAGIC: u32 = 0xF000_0011;

fn pad_to_16_byte_align(data: &[u8]) -> Vec<u8> {
    let new_len = data.len().div_ceil(16) * 16;
    let mut out = data.to_vec();
    out.resize(new_len, 0);
    out
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

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

/// Port of `WwiseBank`.
#[derive(Debug, Clone)]
pub struct WwiseBank {
    pub file_id: u64,
    pub bank_header: Vec<u8>,
    pub bank_misc_data: Vec<u8>,
    pub modified: bool,
    modified_count: u32,
    pub dep: WwiseDep,
    pub hierarchy: WwiseHierarchy,
}

impl WwiseBank {
    pub fn id(&self) -> u64 {
        self.file_id
    }

    pub fn raise_modified(&mut self) {
        self.modified = true;
        self.modified_count += 1;
    }

    pub fn lower_modified(&mut self) {
        if self.modified {
            self.modified_count -= 1;
            if self.modified_count == 0 {
                self.modified = false;
            }
        }
    }

    /// `WwiseBank.generate`: regenerates the bank blob from its hierarchy.
    /// The Python DIDX/DATA regeneration loop (walking `Sound`/`MusicTrack`
    /// sources) is not ported yet — those HIRC types don't exist until a
    /// later phase, and until then `hierarchy.sounds()`/`.music_tracks()`
    /// are always empty, so that loop can never produce output yet either.
    pub fn generate(&self) -> Vec<u8> {
        let mut data = self.bank_header.clone();
        let hierarchy_section = self.hierarchy.get_data();
        data.extend_from_slice(b"HIRC");
        data.extend_from_slice(&(hierarchy_section.len() as u32).to_le_bytes());
        data.extend_from_slice(&hierarchy_section);
        data.extend_from_slice(&self.bank_misc_data);
        data
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
        let mut archive = GameArchive {
            name,
            ..Default::default()
        };
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

        for n in 0..num_files as usize {
            toc.seek(toc_start + n * 80);
            let header = TocHeader::read(&mut toc);

            if header.type_id == WWISE_STREAM {
                let start = header.stream_file_offset as usize;
                let end = start + header.stream_size as usize;
                let mut audio = AudioSource {
                    stream_type: STREAM_TYPE_STREAM,
                    ..Default::default()
                };
                audio.set_data(stream_data[start..end].to_vec(), false);
                audio.resource_id = header.file_id;
                let wstream = WwiseStream {
                    file_id: header.file_id,
                    audio_source: audio,
                    modified: false,
                };
                self.wwise_streams.insert(wstream.id(), wstream);
            } else if header.type_id == WWISE_BANK {
                toc.seek(header.toc_data_offset as usize);
                toc.advance(16);
                let bank_bytes = toc.read((header.toc_data_size - 16) as usize).to_vec();
                let parser = BankParser::load(&bank_bytes);

                let bkhd = parser.get_chunk("BKHD").to_vec();
                let mut bank_header = Vec::with_capacity(8 + bkhd.len());
                bank_header.extend_from_slice(b"BKHD");
                bank_header.extend_from_slice(&(bkhd.len() as u32).to_le_bytes());
                bank_header.extend_from_slice(&bkhd);

                let bank_version_raw = u32::from_le_bytes(bkhd[0..4].try_into().unwrap());
                let version = if (bank_version_raw ^ BANK_VERSION_KEY) == 154 {
                    BankVersion::V154
                } else {
                    BankVersion::V140
                };

                let hierarchy = if parser.chunks.contains_key("HIRC") {
                    WwiseHierarchy::load(version, parser.get_chunk("HIRC"))
                } else {
                    WwiseHierarchy::new(version)
                };
                for (id, entry) in &hierarchy.entries {
                    self.hierarchy_entries.insert(*id, entry.clone());
                }

                if parser.chunks.contains_key("DIDX") {
                    let mut mi = MediaIndex::default();
                    mi.load(parser.get_chunk("DIDX"), parser.get_chunk("DATA"));
                }

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
                    bank_header,
                    bank_misc_data,
                    modified: false,
                    modified_count: 0,
                    dep,
                    hierarchy,
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
                let video = VideoSource::from_original(header.file_id, header.stream_size, stream_data[start..end].to_vec());
                self.video_sources.insert(video.id(), video);
            }
        }
    }

    fn write_type_header(toc: &mut Vec<u8>, entry_type: u64, num_entries: usize) {
        if num_entries > 0 {
            toc.extend_from_slice(&0u64.to_le_bytes());
            toc.extend_from_slice(&entry_type.to_le_bytes());
            toc.extend_from_slice(&(num_entries as u64).to_le_bytes());
            toc.extend_from_slice(&16u32.to_le_bytes());
            toc.extend_from_slice(&64u32.to_le_bytes());
        }
    }

    /// `GameArchive.to_file`: serializes this archive back into `<dir>/<name>`
    /// (plus a `.stream` companion when there's any streamed/video payload).
    pub fn to_file(&self, dir: &Path) -> std::io::Result<()> {
        let deps: Vec<&WwiseDep> = self.wwise_banks.values().map(|b| &b.dep).filter(|d| !d.skip).collect();
        let num_files =
            self.wwise_streams.len() + self.wwise_banks.len() + deps.len() + self.text_banks.len() + self.video_sources.len();
        let num_types = [
            !self.wwise_streams.is_empty(),
            !self.wwise_banks.is_empty(),
            !self.text_banks.is_empty(),
            !self.video_sources.is_empty(),
            !deps.is_empty(),
        ]
        .into_iter()
        .filter(|b| *b)
        .count();

        let mut toc: Vec<u8> = Vec::new();
        toc.extend_from_slice(&self.magic.to_le_bytes());
        toc.extend_from_slice(&(num_types as u32).to_le_bytes());
        toc.extend_from_slice(&(num_files as u32).to_le_bytes());
        toc.extend_from_slice(&self.unknown.to_le_bytes());
        let mut unk4 = self.unk4_data.clone();
        unk4.resize(56, 0);
        toc.extend_from_slice(&unk4);

        Self::write_type_header(&mut toc, WWISE_STREAM, self.wwise_streams.len());
        Self::write_type_header(&mut toc, WWISE_BANK, self.wwise_banks.len());
        Self::write_type_header(&mut toc, WWISE_DEP, deps.len());
        Self::write_type_header(&mut toc, TEXT_BANK, self.text_banks.len());
        Self::write_type_header(&mut toc, BINK_VIDEO, self.video_sources.len());

        let mut toc_data_offset = toc.len() as u64 + 80 * num_files as u64 + 8;
        let mut stream_file_offset = 0u64;

        let mut toc_entries: Vec<TocHeader> = Vec::new();
        let mut toc_data: Vec<u8> = Vec::new();
        let mut stream_data: Vec<u8> = Vec::new();
        let mut entry_index = 0u32;

        for stream in self.wwise_streams.values() {
            let raw = stream.get_data();
            let s_data = pad_to_16_byte_align(raw);
            let mut t_data = hex_decode("D82F767800000000");
            t_data.extend_from_slice(&(raw.len() as u64).to_le_bytes());

            toc_entries.push(TocHeader {
                file_id: stream.id(),
                type_id: WWISE_STREAM,
                toc_data_offset,
                stream_file_offset,
                toc_data_size: 0x0C,
                stream_size: raw.len() as u32,
                entry_index,
                ..Default::default()
            });
            stream_data.extend_from_slice(&s_data);
            toc_data.extend_from_slice(&t_data);
            entry_index += 1;
            stream_file_offset += s_data.len() as u64;
            toc_data_offset += 16;
        }

        for bank in self.wwise_banks.values() {
            let bank_data = bank.generate();
            toc_entries.push(TocHeader {
                file_id: bank.id(),
                type_id: WWISE_BANK,
                toc_data_offset,
                stream_file_offset,
                toc_data_size: bank_data.len() as u32 + 16,
                entry_index,
                ..Default::default()
            });
            let mut wrapped = hex_decode("D82F7678");
            wrapped.extend_from_slice(&(bank_data.len() as u32).to_le_bytes());
            wrapped.extend_from_slice(&bank.id().to_le_bytes());
            wrapped.extend_from_slice(&pad_to_16_byte_align(&bank_data));
            toc_data_offset += wrapped.len() as u64;
            toc_data.extend_from_slice(&wrapped);
            entry_index += 1;
        }

        for text_bank in self.text_banks.values() {
            let text_data = text_bank.generate();
            toc_entries.push(TocHeader {
                file_id: text_bank.id(),
                type_id: TEXT_BANK,
                toc_data_offset,
                stream_file_offset,
                toc_data_size: text_data.len() as u32,
                entry_index,
                ..Default::default()
            });
            let padded = pad_to_16_byte_align(&text_data);
            toc_data_offset += padded.len() as u64;
            toc_data.extend_from_slice(&padded);
            entry_index += 1;
        }

        for dep in &deps {
            let dep_data = dep.get_data();
            toc_entries.push(TocHeader {
                file_id: dep.file_id,
                type_id: WWISE_DEP,
                toc_data_offset,
                stream_file_offset,
                toc_data_size: dep_data.len() as u32,
                entry_index,
                ..Default::default()
            });
            let padded = pad_to_16_byte_align(&dep_data);
            toc_data_offset += padded.len() as u64;
            toc_data.extend_from_slice(&padded);
            entry_index += 1;
        }

        for video in self.video_sources.values() {
            let raw = video.get_data();
            let s_data = pad_to_16_byte_align(raw);
            let t_data = hex_decode("E9030000000000000000000000000000");

            toc_entries.push(TocHeader {
                file_id: video.id(),
                type_id: BINK_VIDEO,
                toc_data_offset,
                stream_file_offset,
                toc_data_size: 0x10,
                stream_size: raw.len() as u32,
                entry_index,
                ..Default::default()
            });
            stream_data.extend_from_slice(&s_data);
            toc_data.extend_from_slice(&t_data);
            entry_index += 1;
            stream_file_offset += s_data.len() as u64;
            toc_data_offset += 16;
        }

        for entry in &toc_entries {
            toc.extend_from_slice(&entry.get_data());
        }
        toc.resize(toc.len() + 8, 0);
        toc.extend_from_slice(&toc_data);

        let min_size = toc_entries.len() * 256;
        if toc.len() < min_size {
            toc.resize(min_size, 0);
        }
        std::fs::write(dir.join(&self.name), &toc)?;

        if !stream_data.is_empty() {
            std::fs::write(dir.join(format!("{}.stream", self.name)), &stream_data)?;
        }

        Ok(())
    }
}
