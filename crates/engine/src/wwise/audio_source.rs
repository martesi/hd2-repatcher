//! Port of `AudioSource`, `WwiseStream`, and `WwiseDep` from the Python
//! source.

use crate::memstream::MemoryStream;

/// `stream_type` values an [`AudioSource`]/`BankSourceStruct` can carry
/// (`BANK`/`PREFETCH_STREAM`/`STREAM` in the Python `const.py` — renamed here
/// to avoid clashing with the TOC-type constants of the same names in
/// `crate::lib`).
pub const STREAM_TYPE_BANK: u32 = 0;
pub const STREAM_TYPE_PREFETCH: u32 = 1;
pub const STREAM_TYPE_STREAM: u32 = 2;

/// Port of `AudioSource`. The Python `parents` back-reference set and `muted`
/// flag are dropped: `muted` is a GUI-only mute toggle this headless engine
/// never sets. `parents` is used for two things in real upstream — a
/// dead/commented-out `MusicTrack` duration-sync branch (genuinely dead), and
/// `AudioSource.set_data`'s `notify_subscribers` path, which marks a bank
/// modified when one of its Sound/MusicTrack sources gets new audio bytes
/// (very much alive). `Mod::import_patch` restores that second effect via a
/// live scan instead of a maintained back-reference — see its doc comment.
#[derive(Debug, Default, Clone)]
pub struct AudioSource {
    pub data: Vec<u8>,
    pub resource_id: u64,
    pub short_id: u32,
    pub modified: bool,
    pub data_old: Vec<u8>,
    pub stream_type: u32,
}

impl AudioSource {
    /// `get_id`: banked sources are keyed by their short id, streamed ones by
    /// their resource (TOC file) id.
    pub fn id(&self) -> u64 {
        if self.stream_type == STREAM_TYPE_BANK {
            self.short_id as u64
        } else {
            self.resource_id
        }
    }

    pub fn get_data(&self) -> &[u8] {
        &self.data
    }

    /// `set_data`. `notify_subscribers` is dropped along with `parents`.
    pub fn set_data(&mut self, data: Vec<u8>, set_modified: bool) {
        if !self.modified && set_modified {
            self.data_old = std::mem::take(&mut self.data);
        }
        self.data = data;
        if set_modified {
            self.modified = true;
        }
    }
}

/// Port of `WwiseDep`: the string dependency record (`.bnk` display name)
/// attached to a `WwiseBank`.
#[derive(Debug, Clone)]
pub struct WwiseDep {
    tag: u32,
    pub data: String,
    /// True for a synthesized default dependency (`Bank <id>`) that never
    /// appeared on disk; `GameArchive::to_file` never writes these out.
    pub skip: bool,
    pub file_id: u64,
}

impl WwiseDep {
    /// The default dependency `WwiseBank::load` synthesizes when no
    /// `WWISE_DEP` TOC entry pointed at this bank.
    pub fn synthesized(file_id: u64, bank_id: u64) -> Self {
        WwiseDep {
            tag: 0,
            data: format!("Bank {bank_id}"),
            skip: true,
            file_id,
        }
    }

    pub fn read(stream: &mut MemoryStream, file_id: u64) -> Self {
        let tag = stream.read_u32();
        let data_size = stream.read_u32();
        let data = String::from_utf8_lossy(stream.read(data_size as usize)).into_owned();
        WwiseDep {
            tag,
            data,
            skip: false,
            file_id,
        }
    }

    pub fn get_data(&self) -> Vec<u8> {
        let bytes = self.data.as_bytes();
        let mut out = Vec::with_capacity(8 + bytes.len());
        out.extend_from_slice(&self.tag.to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
        out
    }
}

/// Port of `WwiseStream`: a `WWISE_STREAM` TOC entry wrapping a streamed
/// (not bank-embedded) [`AudioSource`].
#[derive(Debug, Clone)]
pub struct WwiseStream {
    pub file_id: u64,
    pub audio_source: AudioSource,
    pub modified: bool,
}

impl WwiseStream {
    pub fn id(&self) -> u64 {
        self.file_id
    }

    pub fn get_data(&self) -> &[u8] {
        self.audio_source.get_data()
    }
}
