//! `AudioIndex`: soundbank id -> containing base archive's display name.
//! Generalizes `resources.rs`'s TOC-scan pattern
//! ([`crate::resources::scan_package_toc`]) to index `WWISE_BANK` entries
//! instead of `UNIT_TYPE_ID` ones. This replaces the external friendlynames
//! db real upstream's headless CLI downloads to resolve the same mapping —
//! see `.ref/native-audio-patch-plan.md`'s "Archive index" section.
//!
//! Built alongside `GameResources`'s unit index in a single directory pass
//! (`GameResources::load` produces both); text banks/streams/deps/video
//! don't need their own index since they ride along once their containing
//! bank's archive loads (`run_patch_cli` hardcodes the text-bank archive as
//! `9ba626afa44a3aa3`).

use std::collections::HashMap;

/// Soundbank id -> the display name (basename) of the base game archive
/// package that contains it. The *whole* archive is what gets loaded (via
/// `wwise::Mod::load_base_archive`), not a single resource read, so unlike
/// `resources::ResourceLoc` no toc offset/size is tracked here.
#[derive(Debug, Default)]
pub struct AudioIndex {
    mapping: HashMap<u64, String>,
}

impl AudioIndex {
    pub(crate) fn from_mapping(mapping: HashMap<u64, String>) -> Self {
        AudioIndex { mapping }
    }

    /// The base archive's display name that contains soundbank `bank_id`, if
    /// indexed.
    pub fn archive_name(&self, bank_id: u64) -> Option<&str> {
        self.mapping.get(&bank_id).map(String::as_str)
    }

    pub fn bank_count(&self) -> usize {
        self.mapping.len()
    }
}
