//! Transactional, one-group-at-a-time patch orchestration.
//!
//! The engine contains the byte-level unit/audio implementations. This module
//! owns the filesystem contract around them: a complete patch group is copied
//! to a private staging directory, all work happens there, and only the
//! selected main file and its sidecars are committed back to their original
//! paths after validation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use engine::{
    classify_patch_file, is_audio_type_id, process_audio_patches_to, update_patch_file,
    GameResources, PatchFileGroup, PatchKind, PatchOutcome,
};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectories(Vec<PathBuf>);

impl Drop for TemporaryDirectories {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

/// Outcome of processing one logical patch group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupOutcome {
    Updated(PatchKind),
    Skipped(PatchKind),
}

/// Processes exactly one resolved group transactionally.
pub fn process_patch_group(
    group: &PatchFileGroup,
    resources: &GameResources,
) -> Result<GroupOutcome, String> {
    // Resolve again immediately before staging. This catches a sidecar that
    // disappeared between discovery and execution without touching anything.
    let current = PatchFileGroup::resolve(&group.main).map_err(|e| e.to_string())?;
    let kind = classify_patch_file(&current.main);
    if kind == PatchKind::Corrupted {
        return Err(format!(
            "patch '{}' is malformed or unreadable",
            current.main.display()
        ));
    }
    if kind == PatchKind::Other {
        return Ok(GroupOutcome::Skipped(kind));
    }

    let stage_dir = temporary_directory("stage")?;
    let result = process_staged_group(&current, kind, resources, &stage_dir);
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&stage_dir);
            return Err(error);
        }
    };

    let final_group = match PatchFileGroup::resolve(&result.stage_main) {
        Ok(group) => group,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&stage_dir);
            return Err(format!("staged patch validation failed: {error}"));
        }
    };

    let commit = commit_group(&current, &final_group, &result.stage_main);
    let _ = std::fs::remove_dir_all(&stage_dir);
    commit.map(|()| GroupOutcome::Updated(kind))
}

struct StagedResult {
    stage_main: PathBuf,
}

fn process_staged_group(
    original: &PatchFileGroup,
    kind: PatchKind,
    resources: &GameResources,
    stage_dir: &Path,
) -> Result<StagedResult, String> {
    let staged = copy_group(original, stage_dir)?;

    if matches!(kind, PatchKind::Unit | PatchKind::UnitAndAudio) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            update_patch_file(&staged.main, resources)
        }))
        .unwrap_or(PatchOutcome::Corrupted);
        if outcome == PatchOutcome::Corrupted {
            return Err(format!(
                "unit patching failed for '{}'",
                original.main.display()
            ));
        }
    }

    if matches!(kind, PatchKind::Audio | PatchKind::UnitAndAudio) {
        process_audio_in_stage(original, &staged, kind, resources, stage_dir)?;
    }

    Ok(StagedResult {
        stage_main: staged.main,
    })
}

/// Runs the audio engine with a clean output directory. Keeping its input and
/// output separate is important: if the source has an old `.stream` but the
/// audio patcher does not produce a new one, that old sidecar must remain
/// distinguishable from generated output and must not be treated as new data.
fn process_audio_in_stage(
    original: &PatchFileGroup,
    staged: &PatchFileGroup,
    kind: PatchKind,
    resources: &GameResources,
    stage_dir: &Path,
) -> Result<(), String> {
    let input_dir = temporary_directory("audio-input")?;
    let output_dir = match temporary_directory("audio-output") {
        Ok(path) => path,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&input_dir);
            return Err(error);
        }
    };
    let _cleanup = TemporaryDirectories(vec![input_dir.clone(), output_dir.clone()]);
    let input = copy_group(staged, &input_dir)?;
    let output_name = original
        .main
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            format!(
                "patch '{}' has a non-UTF-8 filename",
                original.main.display()
            )
        })?;
    let generated_main = output_dir.join(output_name);

    let audio_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        process_audio_patches_to(&output_dir, &[input.main.clone()], resources, output_name)
    }))
    .unwrap_or_else(|_| Err("audio patching panicked while parsing the patch group".into()));

    let result = match audio_result {
        Ok(()) => {
            let generated = PatchFileGroup::resolve(&generated_main)
                .map_err(|e| format!("generated audio patch validation failed: {e}"))?;
            let preserve_non_audio =
                kind == PatchKind::UnitAndAudio || has_non_audio_resources(&staged.main)?;
            if preserve_non_audio {
                merge_mixed_patch(&staged.main, &generated.main, stage_dir)?;
            } else {
                std::fs::copy(&generated.main, &staged.main).map_err(|error| {
                    format!(
                        "failed to stage generated patch '{}': {error}",
                        staged.main.display()
                    )
                })?;
                // The audio engine writes a stream only when it has streamed
                // output. If it did not, leave the staged original sidecar
                // alone. If it did, replace/create exactly that sidecar.
                if let Some(generated_stream) = generated.stream {
                    let staged_stream = PatchFileGroup::companion_path(&staged.main, ".stream");
                    std::fs::copy(generated_stream, staged_stream).map_err(|error| {
                        format!("failed to stage generated stream companion: {error}")
                    })?;
                }
            }
            Ok(())
        }
        Err(error) => Err(error),
    };

    result
}

fn has_non_audio_resources(main: &Path) -> Result<bool, String> {
    let stream = PatchFileGroup::companion_path(main, ".stream");
    let gpu = PatchFileGroup::companion_path(main, ".gpu_resources");
    let archive = RawArchive::read(
        main,
        stream.is_file().then_some(stream.as_path()),
        gpu.is_file().then_some(gpu.as_path()),
    )?;
    Ok(archive
        .entries
        .iter()
        .any(|entry| !is_audio_type_id(entry.type_id)))
}

fn copy_group(group: &PatchFileGroup, destination: &Path) -> Result<PatchFileGroup, String> {
    std::fs::create_dir_all(destination).map_err(|error| {
        format!(
            "failed to create staging directory '{}': {error}",
            destination.display()
        )
    })?;
    for source in group.existing_paths() {
        let name = source
            .file_name()
            .ok_or_else(|| format!("path '{}' has no filename", source.display()))?;
        std::fs::copy(source, destination.join(name)).map_err(|error| {
            format!(
                "failed to stage '{}' into '{}': {error}",
                source.display(),
                destination.display()
            )
        })?;
    }
    let main_name = group
        .main
        .file_name()
        .ok_or_else(|| format!("path '{}' has no filename", group.main.display()))?;
    PatchFileGroup::resolve(&destination.join(main_name)).map_err(|error| error.to_string())
}

fn temporary_directory(label: &str) -> Result<PathBuf, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "hd2-repatcher-{label}-{}-{timestamp}-{counter}",
        std::process::id()
    ));
    std::fs::create_dir(&path).map_err(|error| {
        format!(
            "failed to create temporary staging directory '{}': {error}",
            path.display()
        )
    })?;
    Ok(path)
}

/// A deliberately small raw archive reader/writer used only for mixed patches.
/// The audio engine serializes only audio resources; this preserves every
/// non-audio TOC entry from the unit-patched source while taking regenerated
/// audio entries from the audio output.
#[derive(Clone)]
struct RawArchive {
    magic: u32,
    unknown: u32,
    unknown_data: Vec<u8>,
    types: Vec<RawType>,
    entries: Vec<RawEntry>,
    stream: Vec<u8>,
}

#[derive(Clone)]
struct RawType {
    type_id: u64,
    template: [u8; 32],
}

#[derive(Clone)]
struct RawEntry {
    header: [u8; 80],
    type_id: u64,
    toc_data: Vec<u8>,
    stream_data: Vec<u8>,
    gpu_data: Vec<u8>,
}

fn merge_mixed_patch(
    original_main: &Path,
    generated_main: &Path,
    stage_dir: &Path,
) -> Result<(), String> {
    let original_stream_path = PatchFileGroup::companion_path(original_main, ".stream");
    let generated_stream_path = PatchFileGroup::companion_path(generated_main, ".stream");
    let original_gpu_path = PatchFileGroup::companion_path(original_main, ".gpu_resources");
    let generated_gpu_path = PatchFileGroup::companion_path(generated_main, ".gpu_resources");

    let original = RawArchive::read(
        original_main,
        original_stream_path
            .is_file()
            .then_some(original_stream_path.as_path()),
        original_gpu_path
            .is_file()
            .then_some(original_gpu_path.as_path()),
    )?;
    let generated = RawArchive::read(
        generated_main,
        generated_stream_path
            .is_file()
            .then_some(generated_stream_path.as_path()),
        generated_gpu_path
            .is_file()
            .then_some(generated_gpu_path.as_path()),
    )?;

    let mut entries: Vec<RawEntry> = original
        .entries
        .iter()
        .filter(|entry| !is_audio_type_id(entry.type_id))
        .cloned()
        .collect();
    entries.extend(
        generated
            .entries
            .iter()
            .filter(|entry| is_audio_type_id(entry.type_id))
            .cloned(),
    );

    let mut types = Vec::<RawType>::new();
    for entry in &entries {
        if types.iter().all(|ty| ty.type_id != entry.type_id) {
            let template = original
                .types
                .iter()
                .chain(generated.types.iter())
                .find(|ty| ty.type_id == entry.type_id)
                .map(|ty| ty.template)
                .unwrap_or([0; 32]);
            types.push(RawType {
                type_id: entry.type_id,
                template,
            });
        }
    }

    let original_stream_entries = entries
        .iter()
        .filter(|entry| !is_audio_type_id(entry.type_id) && !entry.stream_data.is_empty())
        .count();
    let generated_stream_entries = entries
        .iter()
        .filter(|entry| is_audio_type_id(entry.type_id) && !entry.stream_data.is_empty())
        .count();
    let mut stream_output = if generated_stream_entries > 0 && original_stream_entries == 0 {
        generated.stream.clone()
    } else {
        original.stream.clone()
    };
    let combine_streams = original_stream_entries > 0 && generated_stream_entries > 0;
    let mut rebuilt_stream = Vec::new();
    let mut original_stream_offsets = HashMap::<usize, u64>::new();
    let mut generated_stream_offsets = HashMap::<usize, u64>::new();
    if combine_streams {
        for (index, entry) in entries.iter().enumerate() {
            if !entry.stream_data.is_empty() && !is_audio_type_id(entry.type_id) {
                let offset = append_aligned(&mut rebuilt_stream, &entry.stream_data);
                original_stream_offsets.insert(index, offset);
            }
        }
        for (index, entry) in entries.iter().enumerate() {
            if !entry.stream_data.is_empty() && is_audio_type_id(entry.type_id) {
                let offset = append_aligned(&mut rebuilt_stream, &entry.stream_data);
                generated_stream_offsets.insert(index, offset);
            }
        }
        stream_output = rebuilt_stream;
    }

    let mut inline_data = Vec::new();
    let mut toc_entries = Vec::<[u8; 80]>::with_capacity(entries.len());
    let toc_start = 72usize
        .checked_add(
            types
                .len()
                .checked_mul(32)
                .ok_or("mixed patch type table overflow")?,
        )
        .and_then(|value| {
            value.checked_add(
                entries
                    .len()
                    .checked_mul(80)
                    .ok_or("mixed patch file table overflow")
                    .ok()?,
            )
        })
        .and_then(|value| value.checked_add(8))
        .ok_or("mixed patch TOC offset overflow")?;
    let mut toc_data_offset = toc_start as u64;

    for (index, entry) in entries.iter().enumerate() {
        let mut header = entry.header;
        write_u64(&mut header, 16, toc_data_offset);
        let aligned_size = align16(entry.toc_data.len());
        inline_data.extend_from_slice(&entry.toc_data);
        inline_data.resize(
            inline_data.len() + aligned_size.saturating_sub(entry.toc_data.len()),
            0,
        );
        toc_data_offset = toc_data_offset
            .checked_add(aligned_size as u64)
            .ok_or("mixed patch resource offset overflow")?;

        if !entry.stream_data.is_empty() {
            if combine_streams {
                let offset = if is_audio_type_id(entry.type_id) {
                    generated_stream_offsets.get(&index)
                } else {
                    original_stream_offsets.get(&index)
                }
                .copied()
                .ok_or("mixed patch stream offset missing")?;
                write_u64(&mut header, 24, offset);
            } else if is_audio_type_id(entry.type_id) {
                // The generated stream is copied verbatim below, so its
                // original offsets remain valid. Original-only streams keep
                // their offsets in the unchanged original companion.
            }
        }

        if entry.gpu_data.is_empty() && read_u32(&header, 64).unwrap_or(0) > 0 {
            return Err(format!(
                "mixed patch resource {} references GPU data that cannot be regenerated",
                read_u64(&header, 0).unwrap_or_default()
            ));
        }
        toc_entries.push(header);
    }

    let mut output = Vec::with_capacity(toc_start + inline_data.len());
    output.extend_from_slice(&original.magic.to_le_bytes());
    output.extend_from_slice(&(types.len() as u32).to_le_bytes());
    output.extend_from_slice(&(toc_entries.len() as u32).to_le_bytes());
    output.extend_from_slice(&original.unknown.to_le_bytes());
    let mut unknown_data = original.unknown_data.clone();
    unknown_data.resize(56, 0);
    output.extend_from_slice(&unknown_data);
    for ty in &types {
        let mut template = ty.template;
        write_u64(&mut template, 8, ty.type_id);
        let count = toc_entries
            .iter()
            .filter(|header| read_u64(&header[..], 8) == Some(ty.type_id))
            .count();
        write_u64(&mut template, 16, count as u64);
        output.extend_from_slice(&template);
    }
    for header in &toc_entries {
        output.extend_from_slice(header);
    }
    output.extend_from_slice(&[0; 8]);
    output.extend_from_slice(&inline_data);

    let main_name = original_main
        .file_name()
        .ok_or_else(|| format!("path '{}' has no filename", original_main.display()))?;
    std::fs::write(stage_dir.join(main_name), output)
        .map_err(|error| format!("failed to write mixed patch: {error}"))?;

    let stream_path = PatchFileGroup::companion_path(original_main, ".stream");
    if stream_path.is_file() || generated_stream_path.is_file() {
        std::fs::write(
            stage_dir.join(
                stream_path
                    .file_name()
                    .ok_or("mixed patch stream path has no filename")?,
            ),
            stream_output,
        )
        .map_err(|error| format!("failed to write mixed patch stream: {error}"))?;
    }

    // The original `.gpu_resources` companion is deliberately not rewritten.
    // The audio engine does not regenerate it, and the staged copy already
    // contains the exact original bytes.
    Ok(())
}

impl RawArchive {
    fn read(
        main: &Path,
        stream: Option<&Path>,
        gpu_resources: Option<&Path>,
    ) -> Result<Self, String> {
        let data = std::fs::read(main)
            .map_err(|error| format!("failed to read mixed patch '{}': {error}", main.display()))?;
        if data.len() < 72 {
            return Err(format!(
                "mixed patch '{}' has a truncated header",
                main.display()
            ));
        }
        let num_types = read_u32(&data, 4).ok_or("mixed patch type table is truncated")? as usize;
        let type_end = 72usize
            .checked_add(
                num_types
                    .checked_mul(32)
                    .ok_or("mixed patch type table overflow")?,
            )
            .ok_or("mixed patch type table offset overflow")?;
        if type_end > data.len() {
            return Err("mixed patch type table is truncated".into());
        }
        let mut types = Vec::with_capacity(num_types);
        for index in 0..num_types {
            let start = 72 + index * 32;
            let template: [u8; 32] = data[start..start + 32]
                .try_into()
                .map_err(|_| "mixed patch type entry is truncated")?;
            let type_id = read_u64(&template, 8).ok_or("mixed patch type id is truncated")?;
            types.push(RawType { type_id, template });
        }

        let num_files = read_u32(&data, 8).ok_or("mixed patch file table is truncated")? as usize;
        let file_end = type_end
            .checked_add(
                num_files
                    .checked_mul(80)
                    .ok_or("mixed patch file table overflow")?,
            )
            .ok_or("mixed patch file table offset overflow")?;
        if file_end > data.len() {
            return Err("mixed patch file table is truncated".into());
        }
        let stream_data = stream
            .map(|path| {
                std::fs::read(path)
                    .map_err(|error| format!("failed to read '{}': {error}", path.display()))
            })
            .transpose()?
            .unwrap_or_default();
        let gpu_data = gpu_resources
            .map(|path| {
                std::fs::read(path)
                    .map_err(|error| format!("failed to read '{}': {error}", path.display()))
            })
            .transpose()?
            .unwrap_or_default();

        let mut entries = Vec::with_capacity(num_files);
        for index in 0..num_files {
            let start = type_end + index * 80;
            let header: [u8; 80] = data[start..start + 80]
                .try_into()
                .map_err(|_| "mixed patch TOC entry is truncated")?;
            let toc_offset =
                read_u64(&header, 16).ok_or("mixed patch TOC offset is truncated")? as usize;
            let toc_size =
                read_u32(&header, 56).ok_or("mixed patch TOC size is truncated")? as usize;
            let toc_end = toc_offset
                .checked_add(toc_size)
                .ok_or("mixed patch resource offset overflow")?;
            if toc_end > data.len() {
                return Err(format!(
                    "mixed patch resource {} is outside the main file",
                    index
                ));
            }
            let stream_offset =
                read_u64(&header, 24).ok_or("mixed patch stream offset is truncated")? as usize;
            let stream_size =
                read_u32(&header, 60).ok_or("mixed patch stream size is truncated")? as usize;
            let stream_bytes = if stream_size == 0 {
                Vec::new()
            } else {
                let stream_end = stream_offset
                    .checked_add(stream_size)
                    .ok_or("mixed patch stream offset overflow")?;
                if stream_end > stream_data.len() {
                    return Err(format!(
                        "mixed patch stream resource {} is outside the companion",
                        index
                    ));
                }
                stream_data[stream_offset..stream_end].to_vec()
            };
            let gpu_offset =
                read_u64(&header, 32).ok_or("mixed patch GPU offset is truncated")? as usize;
            let gpu_size =
                read_u32(&header, 64).ok_or("mixed patch GPU size is truncated")? as usize;
            let gpu_bytes = if gpu_size == 0 {
                Vec::new()
            } else {
                let gpu_end = gpu_offset
                    .checked_add(gpu_size)
                    .ok_or("mixed patch GPU offset overflow")?;
                if gpu_end > gpu_data.len() {
                    return Err(format!(
                        "mixed patch GPU resource {} is outside the companion",
                        index
                    ));
                }
                gpu_data[gpu_offset..gpu_end].to_vec()
            };
            let type_id = read_u64(&header, 8).ok_or("mixed patch type id is truncated")?;
            entries.push(RawEntry {
                header,
                type_id,
                toc_data: data[toc_offset..toc_end].to_vec(),
                stream_data: stream_bytes,
                gpu_data: gpu_bytes,
            });
        }

        Ok(Self {
            magic: read_u32(&data, 0).ok_or("mixed patch magic is truncated")?,
            unknown: read_u32(&data, 12).ok_or("mixed patch unknown field is truncated")?,
            unknown_data: data[16..72].to_vec(),
            types,
            entries,
            stream: stream_data,
        })
    }
}

fn commit_group(
    original: &PatchFileGroup,
    staged: &PatchFileGroup,
    staged_main: &Path,
) -> Result<(), String> {
    if original.stream.is_some() && staged.stream.is_none() {
        return Err("staging unexpectedly removed the original `.stream` companion".into());
    }
    if original.gpu_resources.is_some() && staged.gpu_resources.is_none() {
        return Err("staging unexpectedly removed the original `.gpu_resources` companion".into());
    }

    let staged_parent = staged_main
        .parent()
        .ok_or("staged patch has no parent directory")?;
    let files = [
        Some((staged_main.to_path_buf(), original.main.clone())),
        staged.stream.as_ref().map(|path| {
            (
                path.clone(),
                PatchFileGroup::companion_path(&original.main, ".stream"),
            )
        }),
        staged.gpu_resources.as_ref().map(|path| {
            (
                path.clone(),
                PatchFileGroup::companion_path(&original.main, ".gpu_resources"),
            )
        }),
    ];

    let mut pending = Vec::new();
    for pair in files.into_iter().flatten() {
        let (staged_path, target) = pair;
        let new_data = std::fs::read(&staged_path).map_err(|error| {
            format!(
                "failed to read staged output '{}': {error}",
                staged_path.display()
            )
        })?;
        let old_data = if target.exists() {
            if !target.is_file() {
                return Err(format!(
                    "commit target '{}' is not a file",
                    target.display()
                ));
            }
            Some(std::fs::read(&target).map_err(|error| {
                format!("failed to read original '{}': {error}", target.display())
            })?)
        } else {
            None
        };
        pending.push((target, new_data, old_data));
    }

    let mut changed: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
    for (target, new_data, old_data) in pending {
        if old_data.as_ref().is_some_and(|old| old == &new_data) {
            continue;
        }
        let backup = old_data.clone();
        changed.push((target.clone(), backup));
        if let Err(error) = std::fs::write(&target, &new_data) {
            rollback(&changed);
            return Err(format!("failed to commit '{}': {error}", target.display()));
        }
    }

    // Keep this binding meaningful in debug builds and make it explicit that
    // all targets are required to remain alongside the selected main file.
    let _ = staged_parent;
    Ok(())
}

fn rollback(changed: &[(PathBuf, Option<Vec<u8>>)]) {
    for (path, old_data) in changed.iter().rev() {
        match old_data {
            Some(data) => {
                let _ = std::fs::write(path, data);
            }
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn append_aligned(output: &mut Vec<u8>, data: &[u8]) -> u64 {
    let offset = output.len() as u64;
    output.extend_from_slice(data);
    output.resize(align16(output.len()), 0);
    offset
}

fn align16(value: usize) -> usize {
    value.div_ceil(16) * 16
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4)
        .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_u64(data: &[u8], offset: usize) -> Option<u64> {
    data.get(offset..offset + 8)
        .map(|bytes| u64::from_le_bytes(bytes.try_into().unwrap()))
}

fn write_u64(data: &mut [u8], offset: usize, value: u64) {
    data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hd2-repatcher-patching-test-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn mixed_archive_merge_keeps_non_audio_entries() {
        let root = temp_dir();
        let original_path = root.join("preserved.patch_7");
        let generated_path = root.join("generated.patch_0");
        let output_dir = root.join("output");
        std::fs::create_dir(&output_dir).unwrap();

        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/engine/tests/fixtures");
        std::fs::copy(fixtures.join("unit_grow/input.patch"), &original_path).unwrap();
        std::fs::copy(
            fixtures.join("process_audio_patches_multi_archive/expected/9ba626afa44a3aa3.patch_0"),
            &generated_path,
        )
        .unwrap();

        let original = RawArchive::read(&original_path, None, None).unwrap();
        merge_mixed_patch(&original_path, &generated_path, &output_dir).unwrap();
        let output_path = output_dir.join("preserved.patch_7");
        let merged = RawArchive::read(&output_path, None, None).unwrap();

        assert!(merged
            .entries
            .iter()
            .any(|entry| entry.type_id == engine::UNIT_TYPE_ID));
        assert!(merged
            .entries
            .iter()
            .any(|entry| is_audio_type_id(entry.type_id)));
        let original_unit = original
            .entries
            .iter()
            .find(|entry| entry.type_id == engine::UNIT_TYPE_ID)
            .unwrap();
        let merged_unit = merged
            .entries
            .iter()
            .find(|entry| entry.type_id == engine::UNIT_TYPE_ID)
            .unwrap();
        assert_eq!(merged_unit.toc_data, original_unit.toc_data);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unit_group_is_staged_and_committed_without_touching_sibling() {
        let root = temp_dir();
        let patch_path = root.join("kept-name.patch_7");
        let sibling_path = root.join("sibling.patch_0");
        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/engine/tests/fixtures");
        std::fs::copy(fixtures.join("unit_grow/input.patch"), &patch_path).unwrap();
        std::fs::copy(fixtures.join("unit_same/input.patch"), &sibling_path).unwrap();
        let stream_path = PatchFileGroup::companion_path(&patch_path, ".stream");
        let gpu_path = PatchFileGroup::companion_path(&patch_path, ".gpu_resources");
        std::fs::write(&stream_path, b"keep this stream").unwrap();
        std::fs::write(&gpu_path, b"keep this gpu data").unwrap();
        let stream_before = std::fs::read(&stream_path).unwrap();
        let gpu_before = std::fs::read(&gpu_path).unwrap();
        let patch_before = std::fs::read(&patch_path).unwrap();
        let sibling_before = std::fs::read(&sibling_path).unwrap();

        let data = root.join("data");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(data.join(engine::LEGACY_MARKER_FILE), []).unwrap();
        let resources = GameResources::load(&data);
        let group = PatchFileGroup::resolve(&patch_path).unwrap();
        let outcome = process_patch_group(&group, &resources).unwrap();
        assert_eq!(outcome, GroupOutcome::Updated(PatchKind::Unit));
        assert!(patch_path.is_file());
        assert_ne!(std::fs::read(&patch_path).unwrap(), patch_before);
        assert_eq!(std::fs::read(&stream_path).unwrap(), stream_before);
        assert_eq!(std::fs::read(&gpu_path).unwrap(), gpu_before);
        assert_eq!(std::fs::read(&sibling_path).unwrap(), sibling_before);
        assert!(!root.join("9ba626afa44a3aa3.patch_0").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn audio_group_keeps_selected_main_filename() {
        let root = temp_dir();
        let patches = root.join("patches");
        let data = root.join("gamedata");
        std::fs::create_dir(&patches).unwrap();
        std::fs::create_dir(&data).unwrap();
        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/engine/tests/fixtures");
        for name in ["9ba626afa44a3aa3", "aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb"] {
            std::fs::copy(
                fixtures
                    .join("process_audio_patches_multi_archive/gamedata")
                    .join(name),
                data.join(name),
            )
            .unwrap();
        }
        let selected = patches.join("chosen-audio.patch_9");
        let sibling = patches.join("sibling-audio.patch_0");
        std::fs::copy(
            fixtures.join("process_audio_patches_multi_archive/patches/mod_patch_multi_a.patch_0"),
            &selected,
        )
        .unwrap();
        std::fs::copy(
            fixtures.join("process_audio_patches_multi_archive/patches/mod_patch_multi_b.patch_0"),
            &sibling,
        )
        .unwrap();
        let sibling_before = std::fs::read(&sibling).unwrap();
        let resources = GameResources::load(&data);
        let group = PatchFileGroup::resolve(&selected).unwrap();

        assert_eq!(
            process_patch_group(&group, &resources).unwrap(),
            GroupOutcome::Updated(PatchKind::Audio)
        );
        assert!(selected.is_file());
        assert!(!patches.join("9ba626afa44a3aa3.patch_0").exists());
        assert_eq!(std::fs::read(&sibling).unwrap(), sibling_before);

        let _ = std::fs::remove_dir_all(root);
    }
}
