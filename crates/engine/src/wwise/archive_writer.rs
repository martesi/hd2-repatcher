//! Serializes archive resources using their original outer headers.

use super::archive::{GameArchive, TocHeader};
use super::audio_source::WwiseDep;
use crate::{BINK_VIDEO, TEXT_BANK, WWISE_BANK, WWISE_DEP, WWISE_STREAM};
use std::path::Path;

fn pad_to_16_byte_align(data: &[u8]) -> Vec<u8> {
    let new_len = data.len().div_ceil(16) * 16;
    let mut out = data.to_vec();
    out.resize(new_len, 0);
    out
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

impl GameArchive {
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
        let num_files = self.wwise_streams.len()
            + self.wwise_banks.len()
            + deps.len()
            + self.text_banks.len()
            + self.video_sources.len();
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
            let mut t_data = stream.resource_header.to_vec();
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
            let bank_data = bank.generate(&self.audio_sources);
            toc_entries.push(TocHeader {
                file_id: bank.id(),
                type_id: WWISE_BANK,
                toc_data_offset,
                stream_file_offset,
                toc_data_size: bank_data.len() as u32 + 16,
                entry_index,
                ..Default::default()
            });
            let mut wrapped = bank.resource_tag.to_le_bytes().to_vec();
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
