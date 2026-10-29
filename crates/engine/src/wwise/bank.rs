//! Sound-bank hierarchy and embedded-media serialization.

use super::audio_source::{AudioSource, WwiseDep, STREAM_TYPE_BANK, STREAM_TYPE_PREFETCH};
use super::hierarchy::WwiseHierarchy;
use super::{REV_AUDIO, VORBIS};
use indexmap::IndexMap;
use std::collections::HashSet;

/// Port of `WwiseBank`.
#[derive(Debug, Clone)]
pub struct WwiseBank {
    pub file_id: u64,
    /// Outer resource tag read from the source archive, distinct from BKHD.
    pub resource_tag: u32,
    pub bank_header: Vec<u8>,
    pub bank_misc_data: Vec<u8>,
    pub modified: bool,
    pub(super) modified_count: u32,
    pub dep: WwiseDep,
    pub hierarchy: WwiseHierarchy,
    /// This bank's own DIDX media ids (`None` if it had no DIDX chunk at
    /// all). `generate` uses this to gate which `Sound`/`MusicTrack`
    /// sources it re-embeds — see the comment there for the exact (and
    /// slightly quirky) None/empty/non-empty semantics it replicates from
    /// Python.
    pub media_index: Option<Vec<u32>>,
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

    /// `WwiseBank.import_hierarchy`: merges a patch file's HIRC field
    /// changes into this bank's own hierarchy, then raises this bank's
    /// `modified` flag if the merge actually changed anything — see
    /// [`WwiseHierarchy::import_hierarchy`]'s doc comment for why this
    /// two-step (merge, then check-and-raise) shape replaces real
    /// upstream's per-entry `soundbanks` back-reference callback.
    pub fn import_hierarchy(&mut self, new_hierarchy: &WwiseHierarchy) {
        if self.hierarchy.import_hierarchy(new_hierarchy) {
            self.raise_modified();
        }
    }

    /// `WwiseBank.generate`: regenerates the bank blob from its hierarchy,
    /// including the DIDX/DATA re-embedding loop over `Sound`'s and
    /// `MusicTrack`'s audio sources.
    pub fn generate(&self, audio_sources: &IndexMap<u64, AudioSource>) -> Vec<u8> {
        let mut data = self.bank_header.clone();

        let mut offset: u32 = 0;
        let mut didx_array: Vec<[u8; 12]> = Vec::new();
        let mut data_array: Vec<&[u8]> = Vec::new();
        let mut added_sources: HashSet<u32> = HashSet::new();

        // Matches Python's `get_sounds() + get_music_tracks()`.
        let sound_sources = self.hierarchy.sounds().into_iter().map(|s| &s.sources);
        let track_sources = self.hierarchy.music_tracks().into_iter().map(|m| &m.sources);
        for sources in sound_sources.chain(track_sources) {
            for source in sources {
                if source.plugin_id == VORBIS {
                    // Python: `if self.media_index and source_id not in
                    // self.media_index: continue` — an *empty* (but present)
                    // media_index is falsy in Python, so it does NOT filter
                    // anything out (this only matters for a bank whose DIDX
                    // chunk existed but had zero entries). Replicated as-is.
                    let Some(media_index) = &self.media_index else {
                        continue;
                    };
                    if !media_index.is_empty() && !media_index.contains(&source.source_id) {
                        continue;
                    }

                    let Some(audio) = audio_sources.get(&(source.source_id as u64)) else {
                        continue;
                    };

                    if source.stream_type as u32 == STREAM_TYPE_PREFETCH && !added_sources.contains(&source.source_id) {
                        let mem_size = source.mem_size as usize;
                        data_array.push(&audio.data[..mem_size.min(audio.data.len())]);
                        didx_array.push(didx_entry(source.source_id, offset, source.mem_size));
                        offset += source.mem_size;
                        added_sources.insert(source.source_id);
                    } else if source.stream_type as u32 == STREAM_TYPE_BANK
                        && !added_sources.contains(&source.source_id)
                    {
                        data_array.push(&audio.data);
                        didx_array.push(didx_entry(source.source_id, offset, audio.data.len() as u32));
                        offset += audio.data.len() as u32;
                        added_sources.insert(source.source_id);
                    }
                } else if source.plugin_id == REV_AUDIO {
                    let Some(entry) = self.hierarchy.get_entry(source.source_id) else {
                        continue;
                    };
                    let fx_data = entry.get_data();
                    let Some(media_index_id) = rev_audio_media_index_id(&fx_data) else {
                        continue;
                    };
                    let Some(audio) = audio_sources.get(&(media_index_id as u64)) else {
                        continue;
                    };

                    // Upstream's asymmetry, kept verbatim: this branch tests
                    // membership on `source.source_id` (the CustomFX entry's
                    // id) but records `media_index_id`
                    // (`audio_core.py:316-320`), so a second entry pointing
                    // at the same REV_AUDIO source re-emits its DIDX entry
                    // and payload under a duplicate id. Keying both sides on
                    // `media_index_id` would dedup it and produce a
                    // byte-different bank.
                    if source.stream_type as u32 == STREAM_TYPE_BANK && !added_sources.contains(&source.source_id) {
                        data_array.push(&audio.data);
                        didx_array.push(didx_entry(media_index_id, offset, audio.data.len() as u32));
                        offset += audio.data.len() as u32;
                        added_sources.insert(media_index_id);
                    }
                }
            }
        }

        if !didx_array.is_empty() {
            data.extend_from_slice(b"DIDX");
            data.extend_from_slice(&((didx_array.len() * 12) as u32).to_le_bytes());
            for entry in &didx_array {
                data.extend_from_slice(entry);
            }
            data.extend_from_slice(b"DATA");
            let total: u32 = data_array.iter().map(|d| d.len() as u32).sum();
            data.extend_from_slice(&total.to_le_bytes());
            for chunk in &data_array {
                data.extend_from_slice(chunk);
            }
        }

        let hierarchy_section = self.hierarchy.get_data();
        data.extend_from_slice(b"HIRC");
        data.extend_from_slice(&(hierarchy_section.len() as u32).to_le_bytes());
        data.extend_from_slice(&hierarchy_section);
        data.extend_from_slice(&self.bank_misc_data);
        data
    }
}

fn didx_entry(id: u32, offset: u32, size: u32) -> [u8; 12] {
    let mut out = [0u8; 12];
    out[0..4].copy_from_slice(&id.to_le_bytes());
    out[4..8].copy_from_slice(&offset.to_le_bytes());
    out[8..12].copy_from_slice(&size.to_le_bytes());
    out
}

/// `int.from_bytes(fx_data[19+plugin_param_size:23+plugin_param_size], ...)`:
/// a `REV_AUDIO` source's `source_id` points at an opaque `FxCustom` HIRC
/// entry; the media index id it actually needs lives at a fixed offset past
/// that entry's plugin-param blob, read straight out of its raw
/// `get_data()` bytes (type+size+id+misc header included) rather than a
/// dedicated parsed structure — `FxCustom` (0x11) is one of the many HIRC
/// types that stay opaque even in the real upstream tool.
pub(super) fn rev_audio_media_index_id(fx_data: &[u8]) -> Option<u32> {
    let plugin_param_size = u32::from_le_bytes(fx_data.get(13..17)?.try_into().unwrap()) as usize;
    let bytes = fx_data.get(19 + plugin_param_size..23 + plugin_param_size)?;
    Some(u32::from_le_bytes(bytes.try_into().unwrap()))
}
