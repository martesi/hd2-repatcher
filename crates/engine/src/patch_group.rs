//! Resolution of a patch archive and its sidecar resources.
//!
//! A Helldivers II patch is a small group of files: the main `.patch_N` file
//! is required, while `.stream` and `.gpu_resources` files are optional unless
//! the main file's TOC contains a resource that references them.  Keeping this
//! resolution in the engine gives every caller the same rules and, crucially,
//! prevents a selected sidecar from accidentally broadening an operation to
//! neighbouring patch files.

use std::fmt;
use std::path::{Path, PathBuf};

const MAIN_HEADER_SIZE: usize = 72;
const TYPE_ENTRY_SIZE: usize = 32;
const FILE_HEADER_SIZE: usize = 80;

/// One patch archive plus the sidecars that belong to that exact filename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchFileGroup {
    /// The required main `.patch_N` file.
    pub main: PathBuf,
    /// The optional `<main>.stream` sidecar, when it exists.
    pub stream: Option<PathBuf>,
    /// The optional `<main>.gpu_resources` sidecar, when it exists.
    pub gpu_resources: Option<PathBuf>,
}

/// Errors returned while resolving a patch group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchFileGroupError {
    message: String,
}

impl PatchFileGroupError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for PatchFileGroupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(f)
    }
}

impl std::error::Error for PatchFileGroupError {}

impl PatchFileGroup {
    /// Resolves either the main patch or one of its sidecars to the same group.
    pub fn resolve(path: &Path) -> Result<Self, PatchFileGroupError> {
        let main = main_path_for(path)?;
        if !main.is_file() {
            return Err(PatchFileGroupError::new(format!(
                "required main patch '{}' does not exist",
                main.display()
            )));
        }

        let (needs_stream, needs_gpu_resources) = required_companions(&main)?;
        let stream = resolve_companion(&main, ".stream", needs_stream)?;
        let gpu_resources = resolve_companion(&main, ".gpu_resources", needs_gpu_resources)?;
        validate_companion_lengths(&main, stream.as_deref(), gpu_resources.as_deref())?;

        Ok(Self {
            main,
            stream,
            gpu_resources,
        })
    }

    /// Resolves a group when the caller already knows that `main` is the main
    /// patch path. This is useful after staging a group into a temporary folder.
    pub fn from_main(main: &Path) -> Result<Self, PatchFileGroupError> {
        Self::resolve(main)
    }

    /// Returns the exact path of a sidecar for `main`, whether or not it exists.
    pub fn companion_path(main: &Path, suffix: &str) -> PathBuf {
        let mut path = main.as_os_str().to_owned();
        path.push(suffix);
        PathBuf::from(path)
    }

    /// Returns every currently present file in this group.
    pub fn existing_paths(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.main.as_path())
            .chain(self.stream.as_deref())
            .chain(self.gpu_resources.as_deref())
    }
}

/// Shared free-function form of [`PatchFileGroup::resolve`].
pub fn resolve_patch_file_group(path: &Path) -> Result<PatchFileGroup, PatchFileGroupError> {
    PatchFileGroup::resolve(path)
}

/// True when `path` is a main patch file rather than a `.stream` or
/// `.gpu_resources` sidecar.
pub fn is_main_patch_file(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    if extension == "patch" {
        return true;
    }
    let Some(number) = extension.strip_prefix("patch_") else {
        return false;
    };
    !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
}

/// Recursively finds main patch files. Sidecars are intentionally excluded.
pub fn find_patch_files(directory: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_patch_files(directory, &mut out);
    out.sort();
    out
}

/// Recursively resolves all main patch files beneath `directory`.
pub fn find_patch_file_groups(
    directory: &Path,
) -> Vec<Result<PatchFileGroup, PatchFileGroupError>> {
    find_patch_files(directory)
        .iter()
        .map(|path| PatchFileGroup::resolve(path))
        .collect()
}

fn collect_patch_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_patch_files(&path, out);
        } else if is_main_patch_file(&path) {
            out.push(path);
        }
    }
}

fn main_path_for(path: &Path) -> Result<PathBuf, PatchFileGroupError> {
    if path.is_dir() {
        return Err(PatchFileGroupError::new(format!(
            "'{}' is a directory, not a patch file",
            path.display()
        )));
    }

    let main = if is_sidecar(path, ".stream") || is_sidecar(path, ".gpu_resources") {
        path.with_extension("")
    } else {
        path.to_path_buf()
    };

    if !is_main_patch_file(&main) {
        return Err(PatchFileGroupError::new(format!(
            "'{}' is not a `.patch_N` file or a matching companion",
            path.display()
        )));
    }
    Ok(main)
}

fn is_sidecar(path: &Path, suffix: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension == suffix.trim_start_matches('.'))
        .unwrap_or(false)
}

fn resolve_companion(
    main: &Path,
    suffix: &str,
    required: bool,
) -> Result<Option<PathBuf>, PatchFileGroupError> {
    let path = PatchFileGroup::companion_path(main, suffix);
    if path.exists() {
        if !path.is_file() {
            return Err(PatchFileGroupError::new(format!(
                "companion '{}' is not a file",
                path.display()
            )));
        }
        return Ok(Some(path));
    }
    if required {
        return Err(PatchFileGroupError::new(format!(
            "main patch '{}' references missing required companion '{}'",
            main.display(),
            path.display()
        )));
    }
    Ok(None)
}

/// Reads only the main TOC and determines whether sidecars are required. The
/// complete structural walk happens before any caller can stage or mutate the
/// group, so a missing referenced sidecar is reported without touching files.
fn required_companions(main: &Path) -> Result<(bool, bool), PatchFileGroupError> {
    let data = std::fs::read(main).map_err(|error| {
        PatchFileGroupError::new(format!("failed to read '{}': {error}", main.display()))
    })?;
    if data.len() < MAIN_HEADER_SIZE {
        return Err(PatchFileGroupError::new(format!(
            "patch '{}' has a truncated header",
            main.display()
        )));
    }

    let num_types = read_u32(&data, 4).ok_or_else(|| {
        PatchFileGroupError::new(format!(
            "patch '{}' has an invalid type table",
            main.display()
        ))
    })? as usize;
    let type_table_end = MAIN_HEADER_SIZE
        .checked_add(num_types.checked_mul(TYPE_ENTRY_SIZE).ok_or_else(|| {
            PatchFileGroupError::new(format!(
                "patch '{}' has an oversized type table",
                main.display()
            ))
        })?)
        .ok_or_else(|| PatchFileGroupError::new("patch type table offset overflow"))?;
    let num_files = read_u32(&data, 8).ok_or_else(|| {
        PatchFileGroupError::new(format!(
            "patch '{}' has an invalid file table",
            main.display()
        ))
    })? as usize;
    let file_table_end = type_table_end
        .checked_add(num_files.checked_mul(FILE_HEADER_SIZE).ok_or_else(|| {
            PatchFileGroupError::new(format!(
                "patch '{}' has an oversized file table",
                main.display()
            ))
        })?)
        .ok_or_else(|| PatchFileGroupError::new("patch file table offset overflow"))?;
    if file_table_end > data.len() {
        return Err(PatchFileGroupError::new(format!(
            "patch '{}' has a truncated file table",
            main.display()
        )));
    }

    let mut needs_stream = false;
    let mut needs_gpu_resources = false;
    for index in 0..num_files {
        let offset = type_table_end + index * FILE_HEADER_SIZE;
        let toc_data_offset = read_u64(&data, offset + 16).ok_or_else(|| {
            PatchFileGroupError::new(format!(
                "patch '{}' has an invalid TOC entry",
                main.display()
            ))
        })? as usize;
        let toc_data_size = read_u32(&data, offset + 56).ok_or_else(|| {
            PatchFileGroupError::new(format!(
                "patch '{}' has an invalid TOC entry",
                main.display()
            ))
        })? as usize;
        if toc_data_offset
            .checked_add(toc_data_size)
            .is_none_or(|end| end > data.len())
        {
            return Err(PatchFileGroupError::new(format!(
                "patch '{}' has a TOC resource outside the file",
                main.display()
            )));
        }

        let stream_size = read_u32(&data, offset + 60).ok_or_else(|| {
            PatchFileGroupError::new(format!(
                "patch '{}' has an invalid stream entry",
                main.display()
            ))
        })?;
        let gpu_size = read_u32(&data, offset + 64).ok_or_else(|| {
            PatchFileGroupError::new(format!(
                "patch '{}' has an invalid GPU entry",
                main.display()
            ))
        })?;
        needs_stream |= stream_size > 0;
        needs_gpu_resources |= gpu_size > 0;
    }

    Ok((needs_stream, needs_gpu_resources))
}

fn validate_companion_lengths(
    main: &Path,
    stream: Option<&Path>,
    gpu_resources: Option<&Path>,
) -> Result<(), PatchFileGroupError> {
    let data = std::fs::read(main).map_err(|error| {
        PatchFileGroupError::new(format!("failed to read '{}': {error}", main.display()))
    })?;
    let num_types = read_u32(&data, 4).unwrap_or_default() as usize;
    let type_end = MAIN_HEADER_SIZE
        .checked_add(num_types.checked_mul(TYPE_ENTRY_SIZE).ok_or_else(|| {
            PatchFileGroupError::new(format!(
                "patch '{}' has an oversized type table",
                main.display()
            ))
        })?)
        .ok_or_else(|| PatchFileGroupError::new("patch type table offset overflow"))?;
    let num_files = read_u32(&data, 8).unwrap_or_default() as usize;
    let file_end = type_end
        .checked_add(num_files.checked_mul(FILE_HEADER_SIZE).ok_or_else(|| {
            PatchFileGroupError::new(format!(
                "patch '{}' has an oversized file table",
                main.display()
            ))
        })?)
        .ok_or_else(|| PatchFileGroupError::new("patch file table offset overflow"))?;
    if file_end > data.len() {
        return Err(PatchFileGroupError::new(format!(
            "patch '{}' has a truncated file table",
            main.display()
        )));
    }

    let stream_len = stream
        .map(|path| {
            std::fs::metadata(path)
                .map(|metadata| metadata.len())
                .map_err(|error| {
                    PatchFileGroupError::new(format!(
                        "failed to inspect '{}': {error}",
                        path.display()
                    ))
                })
        })
        .transpose()?;
    let gpu_len = gpu_resources
        .map(|path| {
            std::fs::metadata(path)
                .map(|metadata| metadata.len())
                .map_err(|error| {
                    PatchFileGroupError::new(format!(
                        "failed to inspect '{}': {error}",
                        path.display()
                    ))
                })
        })
        .transpose()?;

    for index in 0..num_files {
        let offset = type_end + index * FILE_HEADER_SIZE;
        let stream_offset = read_u64(&data, offset + 24).unwrap_or_default();
        let stream_size = read_u32(&data, offset + 60).unwrap_or_default() as u64;
        if stream_size > 0
            && stream_len.is_none_or(|length| stream_offset.saturating_add(stream_size) > length)
        {
            return Err(PatchFileGroupError::new(format!(
                "patch '{}' references stream data outside its companion",
                main.display()
            )));
        }
        let gpu_offset = read_u64(&data, offset + 32).unwrap_or_default();
        let gpu_size = read_u32(&data, offset + 64).unwrap_or_default() as u64;
        if gpu_size > 0 && gpu_len.is_none_or(|length| gpu_offset.saturating_add(gpu_size) > length)
        {
            return Err(PatchFileGroupError::new(format!(
                "patch '{}' references GPU data outside its companion",
                main.display()
            )));
        }
    }
    Ok(())
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4)
        .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_u64(data: &[u8], offset: usize) -> Option<u64> {
    data.get(offset..offset + 8)
        .map(|bytes| u64::from_le_bytes(bytes.try_into().unwrap()))
}
