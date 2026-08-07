//! Port of `core.py::Mod`, trimmed per `.ref/native-audio-patch-plan.md`'s
//! phase 5 scope to exactly what `import_patch`/`write_patch`/
//! `write_separate_patches`/`add_game_archive`/`load_archive_file` need —
//! no `import_wems`/`import_wavs`/`import_files`, no `dump_*`, no
//! `create_dummy_bank`, no hierarchy CRUD beyond `import_wwise_hierarchy`,
//! no per-resource `*_count` dicts (only needed by `remove_game_archive`,
//! out of scope), no `db` constructor argument (GUI undo/redo persistence
//! only).
//!
//! **Aliasing vs. cloning.** Real upstream's `Mod`/`GameArchive` share a
//! single mutable Python object graph: once `add_game_archive` adopts a
//! bank/stream/text-bank/audio-source into the pooled `Mod.*` dict, that
//! `GameArchive`'s own dict entry *is* the same object (dict assignment is a
//! reference, not a copy), so later mutation through either map is visible
//! from both. This port clones instead (the same tradeoff `archive.rs::load`
//! already made for cross-bank hierarchy merges — see its module doc), so
//! [`Mod::write_separate_patches`], which reads `modified`/content state
//! through each `GameArchive`'s own copy rather than the canonical pool,
//! needs an explicit backfill pass first ([`Mod::sync_archives_from_pool`])
//! to reproduce what Python gets for free via aliasing.

use std::path::Path;

use indexmap::IndexMap;

use crate::slim::Slim;
use crate::wwise::archive::{GameArchive, WwiseBank};
use crate::wwise::audio_source::{AudioSource, WwiseStream, STREAM_TYPE_BANK};
use crate::wwise::hierarchy::{ActorMixer, HircEntry};
use crate::wwise::text_bank::TextBank;
use crate::wwise::video::VideoSource;

const COMBINED_PATCH_NAME: &str = "9ba626afa44a3aa3.patch_0";
const COMBINED_UNK4_DATA_HEX: &str =
    "CE09F5F4000000000C729F9E8872B8BD00A06B02000000000079510000000000000000000000000000000000000000000000000000000000";

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Port of `Mod`. See the module doc for what's trimmed and for the
/// cloning-vs-aliasing note that motivates [`Self::sync_archives_from_pool`].
#[derive(Debug, Default)]
pub struct Mod {
    pub wwise_streams: IndexMap<u64, WwiseStream>,
    pub wwise_banks: IndexMap<u64, WwiseBank>,
    pub audio_sources: IndexMap<u64, AudioSource>,
    pub text_banks: IndexMap<u64, TextBank>,
    pub video_sources: IndexMap<u64, VideoSource>,
    pub hierarchy_entries: IndexMap<u32, HircEntry>,
    pub game_archives: IndexMap<String, GameArchive>,
}

impl Mod {
    pub fn new() -> Self {
        Mod::default()
    }

    /// `Mod.get_audio_source`: short-id lookup, falling back to a linear
    /// scan by `resource_id` (matches Python's own fallback).
    fn get_audio_source_mut(&mut self, audio_id: u64) -> Option<&mut AudioSource> {
        if self.audio_sources.contains_key(&audio_id) {
            return self.audio_sources.get_mut(&audio_id);
        }
        self.audio_sources.values_mut().find(|s| s.resource_id == audio_id)
    }

    /// `Mod.load_archive_file`: loads a mod `.patch_N` file (or any other
    /// archive already in the plain/uncompressed on-disk shape) and adopts it
    /// via [`Self::add_game_archive`]. Returns `false` if the file couldn't
    /// be read/parsed or an archive of the same name is already loaded
    /// (matching Python's early-return semantics). For a genuine *base* game
    /// archive, which may be DSAR/bundle-compressed, use
    /// [`Self::load_base_archive`] instead.
    pub fn load_archive_file(&mut self, path: &Path) -> bool {
        let path = if matches!(path.extension().and_then(|e| e.to_str()), Some("stream" | "gpu_resources")) {
            path.with_extension("")
        } else {
            path.to_path_buf()
        };
        let Some(new_archive) = GameArchive::from_patch_file(&path) else {
            return false;
        };
        if self.game_archives.contains_key(&new_archive.name) {
            return false;
        }
        self.add_game_archive(new_archive);
        true
    }

    /// [`Self::load_archive_file`]'s counterpart for a genuine *base* game
    /// archive: loads it by display name through `Slim` (transparently
    /// decompressing DSAR/bundled packages, or reading a legacy install's
    /// plain file — see [`Slim::load_package_toc`]/[`Slim::get_stream_resource`]),
    /// then adopts it via [`Self::add_game_archive`]. Mirrors real upstream's
    /// `GameArchive.from_file` when it's pointed at the base install (via
    /// `load_package`) rather than a plain-file mod patch; `archive_name` is
    /// resolved by the caller through `AudioIndex`, which replaces the
    /// external friendlynames db real upstream's headless CLI consults for
    /// the same lookup. Returns `false` under the same conditions as
    /// [`Self::load_archive_file`].
    pub fn load_base_archive(&mut self, slim: &Slim, archive_name: &str) -> bool {
        if self.game_archives.contains_key(archive_name) {
            return false;
        }
        let toc_data = slim.load_package_toc(archive_name);
        let stream_data = slim.get_stream_resource(archive_name);
        let Some(new_archive) = GameArchive::from_toc_and_stream(archive_name.to_string(), &toc_data, &stream_data) else {
            return false;
        };
        self.add_game_archive(new_archive);
        true
    }

    /// True when at least one pooled resource carries the `modified` flag —
    /// i.e. [`Self::write_patch`] would emit an archive with resources in it
    /// rather than a bare header. Callers that consume a successful write by
    /// deleting the inputs it merged (see
    /// [`crate::process_audio_patches`]'s callers) check this first, so that
    /// a mod whose audio never matched anything pooled fails loudly instead
    /// of replacing the user's patch files with an empty one.
    pub fn has_modified_resources(&self) -> bool {
        self.wwise_streams.values().any(|s| s.modified)
            || self.wwise_banks.values().any(|b| b.modified)
            || self.text_banks.values().any(|t| t.modified)
            || self.video_sources.values().any(|v| v.modified)
    }

    /// `Mod.import_wwise_hierarchy`.
    fn import_wwise_hierarchy(&mut self, soundbank_id: u64, new_hierarchy: &crate::wwise::hierarchy::WwiseHierarchy) {
        if let Some(bank) = self.wwise_banks.get_mut(&soundbank_id) {
            bank.import_hierarchy(new_hierarchy);
        }
    }

    /// `Mod.add_game_archive`, trimmed of `parents`/count-dict bookkeeping
    /// (see module doc): video/hierarchy-entry/bank/stream/text-bank/
    /// audio-source de-dup against the already-pooled state, including the
    /// real, narrower-than-`GameArchive.load`'s-cross-bank-union
    /// ActorMixer-only children merge (`core.py:1901-1918`).
    pub fn add_game_archive(&mut self, mut game_archive: GameArchive) {
        let key = game_archive.name.clone();
        if self.game_archives.contains_key(&key) {
            return;
        }

        for (id, entry) in game_archive.video_sources.iter_mut() {
            if let Some(existing) = self.video_sources.get(id) {
                *entry = existing.clone();
            } else {
                self.video_sources.insert(*id, entry.clone());
            }
        }

        let mut replacements: IndexMap<u32, HircEntry> = IndexMap::new();
        for (id, entry) in game_archive.hierarchy_entries.iter() {
            if let Some(existing) = self.hierarchy_entries.get_mut(id) {
                if let (HircEntry::ActorMixer(incoming), HircEntry::ActorMixer(existing_mixer)) = (entry, &mut *existing)
                {
                    merge_actor_mixer_children(existing_mixer, incoming);
                }
                replacements.insert(*id, existing.clone());
            } else {
                self.hierarchy_entries.insert(*id, entry.clone());
            }
        }
        for bank in game_archive.wwise_banks.values_mut() {
            for (id, replacement) in &replacements {
                if bank.hierarchy.entries.contains_key(id) {
                    bank.hierarchy.entries.insert(*id, replacement.clone());
                }
            }
        }
        for (id, replacement) in replacements {
            game_archive.hierarchy_entries.insert(id, replacement);
        }

        for (id, bank) in game_archive.wwise_banks.iter_mut() {
            if let Some(existing) = self.wwise_banks.get(id) {
                *bank = existing.clone();
            } else {
                self.wwise_banks.insert(*id, bank.clone());
            }
        }
        for (id, stream) in game_archive.wwise_streams.iter_mut() {
            if let Some(existing) = self.wwise_streams.get(id) {
                *stream = existing.clone();
            } else {
                self.wwise_streams.insert(*id, stream.clone());
            }
        }
        for (id, tb) in game_archive.text_banks.iter_mut() {
            if let Some(existing) = self.text_banks.get(id) {
                *tb = existing.clone();
            } else {
                self.text_banks.insert(*id, tb.clone());
            }
        }
        for (id, audio) in game_archive.audio_sources.iter_mut() {
            if let Some(existing) = self.audio_sources.get(id) {
                *audio = existing.clone();
            } else {
                self.audio_sources.insert(*id, audio.clone());
            }
        }

        self.game_archives.insert(key, game_archive);
    }

    /// `Mod.import_patch`: loads a mod's `.patch_N` file and merges its
    /// changed audio bytes, HIRC hierarchy fields, text-bank strings, and
    /// video sources into this `Mod`'s pooled state. `import_hierarchy`
    /// (the parameter, not the method) matches Python's own naming clash —
    /// it gates whether the HIRC field-merge step runs at all, independent
    /// of [`crate::wwise::hierarchy::WwiseHierarchy::import_hierarchy`].
    pub fn import_patch(&mut self, patch_file: &Path, import_hierarchy: bool) -> bool {
        let patch_file = if matches!(patch_file.extension().and_then(|e| e.to_str()), Some("stream" | "gpu_resources")) {
            patch_file.with_extension("")
        } else {
            patch_file.to_path_buf()
        };
        let Some(mut patch_archive) = GameArchive::from_patch_file(&patch_file) else {
            return false;
        };

        let mut swapped_ids: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for new_audio in patch_archive.audio_sources.values() {
            let short_id = new_audio.short_id as u64;
            let new_data = new_audio.data.clone();
            let Some(old_audio) = self.get_audio_source_mut(short_id) else {
                continue;
            };
            let changed = if !old_audio.modified {
                new_data != old_audio.data
            } else {
                new_data != old_audio.data_old
            };
            if changed {
                old_audio.set_data(new_data.clone(), true);
                let resource_id = old_audio.resource_id;
                let stream_type = old_audio.stream_type;
                swapped_ids.insert(new_audio.short_id);

                // Real upstream's `self.audio_sources[short_id]` and
                // `self.wwise_streams[resource_id].audio_source` are the
                // *same* Python object for a stream-backed source
                // (`_create_audio_source_type_stream` returns the
                // `WwiseStream`'s own `AudioSource`, not a copy), so
                // `old_audio.set_data` above already mutates both there.
                // `archive.rs::resolve_audio_sources` clones instead (see
                // its doc comment), so this port needs an explicit
                // write-through here, plus the `modified` flag real
                // upstream's dropped `notify_subscribers` path would have
                // raised on the owning `WwiseStream` — same substitute this
                // module already applies to banks, just for streams.
                if stream_type != STREAM_TYPE_BANK {
                    if let Some(stream) = self.wwise_streams.get_mut(&resource_id) {
                        stream.audio_source.set_data(new_data, true);
                        stream.modified = true;
                    }
                }
            }
        }

        // Real upstream discovers which banks to mark modified via
        // `AudioSource.parents`/`HircEntry.soundbanks` back-references
        // (`AudioSource::set_data`'s `notify_subscribers` path) that this
        // port drops (see `audio_source.rs`'s doc comment). Behaviorally
        // equivalent substitute: a live scan for which banks' Sound/
        // MusicTrack sources reference a swapped id — same mechanism this
        // module already uses for the hierarchy-merge case below.
        if !swapped_ids.is_empty() {
            for bank in self.wwise_banks.values_mut() {
                let touched = bank
                    .hierarchy
                    .sounds()
                    .iter()
                    .any(|s| s.sources.iter().any(|src| swapped_ids.contains(&src.source_id)))
                    || bank
                        .hierarchy
                        .music_tracks()
                        .iter()
                        .any(|m| m.sources.iter().any(|src| swapped_ids.contains(&src.source_id)));
                if touched {
                    bank.raise_modified();
                }
            }
        }

        if import_hierarchy {
            for bank in patch_archive.wwise_banks.values() {
                self.import_wwise_hierarchy(bank.id(), &bank.hierarchy);
            }
        }

        for text_bank in patch_archive.text_banks.values() {
            if let Some(target) = self.text_banks.get_mut(&text_bank.id()) {
                target.import_text(text_bank);
            }
        }

        let mut add_patch = false;
        let known_banks = &self.wwise_banks;
        patch_archive.wwise_banks.retain(|id, _| !known_banks.contains_key(id));
        if !patch_archive.wwise_banks.is_empty() {
            add_patch = true;
        }

        // Python re-opens `patch_file+".stream"` on disk and reads the
        // known-video branch's replacement bytes from it via
        // `Mod.import_video` (`core.py:2096-2098`); this port's `VideoSource`
        // already holds those exact bytes in memory (`video.get_data()`,
        // unmodified at this point — see `video.rs`'s eager-load deviation
        // note), so both branches below reduce to "install the freshly
        // parsed patch video's bytes as the target's replacement".
        let incoming_videos = std::mem::take(&mut patch_archive.video_sources);
        for (id, mut video) in incoming_videos {
            if let Some(existing) = self.video_sources.get_mut(&id) {
                let bytes = video.get_data().to_vec();
                existing.set_data(bytes);
            } else {
                let bytes = video.get_data().to_vec();
                video.set_data(bytes);
                patch_archive.video_sources.insert(id, video);
                add_patch = true;
            }
        }

        if add_patch {
            patch_archive.text_banks.clear();
            self.add_game_archive(patch_archive);
        }

        true
    }

    /// `Mod.write_patch`: serializes every `modified` resource across the
    /// whole pooled `Mod` state into a single combined `.patch_N` archive.
    /// Reads directly from the canonical pool, so — unlike
    /// [`Self::write_separate_patches`] — no pre-write sync is needed.
    pub fn write_patch(&self, output_dir: &Path, output_filename: Option<&str>) -> std::io::Result<()> {
        let mut patch = GameArchive {
            name: output_filename.unwrap_or(COMBINED_PATCH_NAME).to_string(),
            magic: 0xF000_0011,
            unk4_data: hex_decode(COMBINED_UNK4_DATA_HEX),
            audio_sources: self.audio_sources.clone(),
            ..Default::default()
        };
        for (id, stream) in &self.wwise_streams {
            if stream.modified {
                patch.wwise_streams.insert(*id, stream.clone());
            }
        }
        for (id, bank) in &self.wwise_banks {
            if bank.modified {
                patch.wwise_banks.insert(*id, bank.clone());
            }
        }
        for (id, tb) in &self.text_banks {
            if tb.modified {
                patch.text_banks.insert(*id, tb.clone());
            }
        }
        for (id, video) in &self.video_sources {
            if video.modified {
                patch.video_sources.insert(*id, video.clone());
            }
        }
        patch.to_file(output_dir)
    }

    /// Backfills every loaded `GameArchive`'s own resource copies from the
    /// canonical pooled maps — see the module doc's aliasing-vs-cloning
    /// note. Only [`Self::write_separate_patches`] needs this: it's the only
    /// reader that goes through `game_archives[*].*` instead of `self.*`.
    fn sync_archives_from_pool(&mut self) {
        for archive in self.game_archives.values_mut() {
            for (id, bank) in archive.wwise_banks.iter_mut() {
                if let Some(canonical) = self.wwise_banks.get(id) {
                    *bank = canonical.clone();
                }
            }
            for (id, stream) in archive.wwise_streams.iter_mut() {
                if let Some(canonical) = self.wwise_streams.get(id) {
                    *stream = canonical.clone();
                }
            }
            for (id, tb) in archive.text_banks.iter_mut() {
                if let Some(canonical) = self.text_banks.get(id) {
                    *tb = canonical.clone();
                }
            }
            for (id, video) in archive.video_sources.iter_mut() {
                if let Some(canonical) = self.video_sources.get(id) {
                    *video = canonical.clone();
                }
            }
            for (id, audio) in archive.audio_sources.iter_mut() {
                if let Some(canonical) = self.audio_sources.get(id) {
                    *audio = canonical.clone();
                }
            }
        }
    }

    /// `Mod.write_separate_patches`: one `<archive-name>.patch_0` per
    /// originally-touched base archive, each containing only that archive's
    /// own `modified` resources.
    pub fn write_separate_patches(&mut self, output_dir: &Path) -> std::io::Result<()> {
        self.sync_archives_from_pool();
        for archive in self.game_archives.values() {
            let mut patch = GameArchive {
                name: format!("{}.patch_0", archive.name),
                magic: 0xF000_0011,
                unknown: archive.unknown,
                unk4_data: archive.unk4_data.clone(),
                audio_sources: archive.audio_sources.clone(),
                ..Default::default()
            };
            for (id, stream) in &archive.wwise_streams {
                if stream.modified {
                    patch.wwise_streams.insert(*id, stream.clone());
                }
            }
            for (id, bank) in &archive.wwise_banks {
                if bank.modified {
                    patch.wwise_banks.insert(*id, bank.clone());
                }
            }
            for (id, tb) in &archive.text_banks {
                if tb.modified {
                    patch.text_banks.insert(*id, tb.clone());
                }
            }
            for (id, video) in &archive.video_sources {
                if video.modified {
                    patch.video_sources.insert(*id, video.clone());
                }
            }
            patch.to_file(output_dir)?;
        }
        Ok(())
    }
}

/// `Mod.add_game_archive`'s ActorMixer-only children union
/// (`core.py:1907-1912`) — narrower than `GameArchive.load`'s five-type
/// cross-bank union (see `containers.rs`'s module doc): dedup by value,
/// `size += 4` per newly-appended id.
fn merge_actor_mixer_children(existing: &mut ActorMixer, incoming: &ActorMixer) {
    for &child in &incoming.children.children {
        if !existing.children.children.contains(&child) {
            existing.children.children.push(child);
            existing.size += 4;
        }
    }
}
