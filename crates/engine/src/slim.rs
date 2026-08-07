//! Port of the subset of `reference/slim.py` used by the patching engine:
//! DSAR decompression and bundled/legacy package access. Only the functions the
//! engine actually calls are ported (`get_package_toc`,
//! `get_resource_from_package`, and their helpers); `reconstruct_package_*` and
//! `load_package` from the Python (used only by its standalone extractor) are
//! omitted.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::LEGACY_MARKER_FILE;

// compression types
const UNCOMPRESSED: u8 = 0x00;
const COMPRESSED: u8 = 0x03;
// chunk type flag
const START: u8 = 0x02;

// magics
const DSAR_MAGIC: u32 = 1380012868; // compressed DSAR file
const LEGACY_MAGIC: u32 = 4026531857;

enum PackageType {
    Dsar,
    Legacy,
    Bundled,
}

struct BundleEntry {
    original_archive_offset: u64,
    start_offset: u32,
    bundle_index: u8,
}

struct Package {
    size: u64,
    entries: Vec<BundleEntry>,
}

/// Holds the decoded bundle/package index for a "slim" install (or nothing for a
/// legacy install). Mirrors the module globals in `slim.py`.
pub struct Slim {
    folder: PathBuf,
    pub is_slim: bool,
    package_contents: HashMap<String, Package>,
    bundle_offsets: HashMap<String, HashMap<u64, usize>>,
}

fn u32_at(buf: &[u8], off: usize) -> Option<u32> {
    buf.get(off..off + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}

fn u64_at(buf: &[u8], off: usize) -> Option<u64> {
    buf.get(off..off + 8)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
}

fn read_at(file: &mut File, offset: u64, len: usize) -> Option<Vec<u8>> {
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf).ok()?;
    Some(buf)
}

impl Slim {
    /// Point at a game data folder; index bundles if it is a slim install.
    pub fn init(folder: &Path) -> Self {
        let is_slim = !folder.join(LEGACY_MARKER_FILE).exists();
        let mut slim = Slim {
            folder: folder.to_path_buf(),
            is_slim,
            package_contents: HashMap::new(),
            bundle_offsets: HashMap::new(),
        };
        if is_slim {
            slim.init_bundle_mapping();
        }
        slim
    }

    fn package_type(&self, full_path: &Path) -> PackageType {
        match File::open(full_path) {
            Ok(mut f) => {
                let mut m = [0u8; 4];
                if f.read_exact(&mut m).is_ok() && u32::from_le_bytes(m) == DSAR_MAGIC {
                    PackageType::Dsar
                } else {
                    PackageType::Legacy
                }
            }
            Err(_) => PackageType::Bundled,
        }
    }

    /// Whole-file DSAR decompression. Used for the small `bundles.nxa` manifest.
    fn decompress_dsar(&self, path: &Path) -> Vec<u8> {
        let Ok(mut file) = File::open(path) else {
            return Vec::new();
        };
        let Some(head) = read_at(&mut file, 0, 0x20) else {
            return Vec::new();
        };
        let num_chunks = u32_at(&head, 8).unwrap_or(0) as usize;
        let Some(headers) = read_at(&mut file, 0x20, 0x20 * num_chunks) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for i in 0..num_chunks {
            let base = 0x20 * i;
            let compressed_offset = u64_at(&headers, base + 8).unwrap();
            let uncompressed_size = u32_at(&headers, base + 16).unwrap() as usize;
            let compressed_size = u32_at(&headers, base + 20).unwrap() as usize;
            let compression_type = headers[base + 24];
            let Some(temp) = read_at(&mut file, compressed_offset, compressed_size) else {
                return Vec::new();
            };
            out.extend(decompress_chunk(&temp, compression_type, uncompressed_size));
        }
        out
    }

    /// Returns a resource from a bundle file, following the chunk chain from the
    /// chunk that begins at `resource_file_offset` until the next START chunk or
    /// the end of the bundle.
    fn get_resource_from_bundle(&self, bundle_path: &Path, resource_file_offset: u64) -> Vec<u8> {
        let Ok(mut file) = File::open(bundle_path) else {
            return Vec::new();
        };
        let Some(head) = read_at(&mut file, 0, 12) else {
            return Vec::new();
        };
        let num_chunks = u32_at(&head, 8).unwrap_or(0) as usize;

        let basename = basename(bundle_path);
        let Some(offsets) = self.bundle_offsets.get(&basename) else {
            return Vec::new();
        };
        let Some(&start_chunk) = offsets.get(&resource_file_offset) else {
            return Vec::new();
        };
        let mut chunk_num = start_chunk;

        let mut data: Vec<u8> = Vec::new();
        loop {
            let Some(h) = read_at(&mut file, (0x20 + 0x20 * chunk_num) as u64, 0x20) else {
                return data;
            };
            let compressed_offset = u64_at(&h, 8).unwrap();
            let uncompressed_size = u32_at(&h, 16).unwrap() as usize;
            let compressed_size = u32_at(&h, 20).unwrap() as usize;
            let compression_type = h[24];
            let chunk_type = h[25];

            if chunk_type & START != 0 && !data.is_empty() {
                return data;
            }

            let Some(temp) = read_at(&mut file, compressed_offset, compressed_size) else {
                return data;
            };
            data.extend(decompress_chunk(&temp, compression_type, uncompressed_size));

            if chunk_num == num_chunks - 1 {
                return data;
            }
            chunk_num += 1;
        }
    }

    /// Concatenates consecutive chunk-chains from a bundle file, starting at
    /// `start_offset`, until `size` uncompressed bytes have been collected.
    /// Port of `slim.py::get_resources_from_bundle` (plural — distinct from
    /// [`Self::get_resource_from_bundle`], which returns a single chunk-chain
    /// and is what this repeatedly calls). Needed because a package's data
    /// can span more bytes than one `START`-to-`START` chunk-chain covers.
    fn get_resources_from_bundle(&self, bundle_path: &Path, start_offset: u32, size: u32) -> Vec<u8> {
        let mut current_size = 0u32;
        let mut out = Vec::new();
        while current_size < size {
            let resource = self.get_resource_from_bundle(bundle_path, (start_offset + current_size) as u64);
            if resource.is_empty() {
                break;
            }
            current_size += resource.len() as u32;
            out.extend_from_slice(&resource);
        }
        out
    }

    /// Reconstructs a whole bundled package's contents (not just its first
    /// entry — see [`Self::get_package_toc`]'s narrower single-entry read for
    /// the cheap-header-scan case) by stitching every one of its
    /// [`BundleEntry`]s into a `package.size`-length buffer at each entry's
    /// own `original_archive_offset`, each entry spanning up to the next
    /// entry's offset (or the package's end, for the last one). Port of
    /// `slim.py::reconstruct_package_from_bundles`. `name` must already be a
    /// basename (a `package_contents` key), matching every caller here.
    fn reconstruct_package_from_bundles(&self, name: &str) -> Vec<u8> {
        let Some(package) = self.package_contents.get(name) else {
            return Vec::new();
        };
        let mut package_data = vec![0u8; package.size as usize];
        for (i, item) in package.entries.iter().enumerate() {
            let item_size = match package.entries.get(i + 1) {
                Some(next) => next.original_archive_offset - item.original_archive_offset,
                None => package.size - item.original_archive_offset,
            };
            let bundle = self.folder.join(format!("bundles.{:02}.nxa", item.bundle_index));
            let combined = self.get_resources_from_bundle(&bundle, item.start_offset, item_size as u32);
            let start = item.original_archive_offset as usize;
            if start >= package_data.len() {
                continue;
            }
            let end = (start + combined.len()).min(package_data.len());
            package_data[start..end].copy_from_slice(&combined[..end - start]);
        }
        package_data
    }

    /// Returns the *full* reconstructed TOC blob for a package — unlike
    /// [`Self::get_package_toc`]'s header-only slice (cheap, used for
    /// id-indexing scans), this includes every embedded resource's actual
    /// bytes too, since a `TocHeader::toc_data_offset` for e.g. a
    /// `WWISE_BANK` entry points deep into this same buffer, well past the
    /// header table. Port of `slim.py::load_package`'s `toc_data` return
    /// value (its `gpu_data` isn't ported — no resource type this engine
    /// handles lives in `.gpu_resources`).
    pub fn load_package_toc(&self, package_name: &str) -> Vec<u8> {
        let name = basename_str(package_name);
        let full_path = self.folder.join(&name);
        match self.package_type(&full_path) {
            PackageType::Bundled => self.reconstruct_package_from_bundles(&name),
            PackageType::Dsar => self.decompress_dsar(&full_path),
            PackageType::Legacy => std::fs::read(&full_path).unwrap_or_default(),
        }
    }

    /// Reads a resource (of `resource_size` for legacy packages) at
    /// `resource_file_offset` within a package, dispatching by package type.
    pub fn get_resource_from_package(
        &self,
        package_name: &str,
        resource_file_offset: u64,
        resource_size: usize,
    ) -> Vec<u8> {
        let name = basename_str(package_name);
        let full_path = self.folder.join(&name);
        match self.package_type(&full_path) {
            PackageType::Bundled => {
                let Some(package) = self.package_contents.get(&name) else {
                    return Vec::new();
                };
                for entry in package.entries.iter().rev() {
                    if entry.original_archive_offset <= resource_file_offset {
                        let bundle = self
                            .folder
                            .join(format!("bundles.{:02}.nxa", entry.bundle_index));
                        let off = entry.start_offset as u64
                            + (resource_file_offset - entry.original_archive_offset);
                        return self.get_resource_from_bundle(&bundle, off);
                    }
                }
                Vec::new()
            }
            PackageType::Dsar => self.get_resource_from_bundle(&full_path, resource_file_offset),
            PackageType::Legacy => {
                let Ok(mut f) = File::open(&full_path) else {
                    return Vec::new();
                };
                let Some(head) = read_at(&mut f, 0, 12) else {
                    return Vec::new();
                };
                if u32_at(&head, 0) != Some(LEGACY_MAGIC) {
                    return Vec::new();
                }
                read_at(&mut f, resource_file_offset, resource_size).unwrap_or_default()
            }
        }
    }

    /// Returns the TOC blob (header + type table + file headers) for a package.
    pub fn get_package_toc(&self, package_name: &str) -> Vec<u8> {
        let name = basename_str(package_name);
        let full_path = self.folder.join(&name);
        match self.package_type(&full_path) {
            PackageType::Bundled => {
                let Some(package) = self.package_contents.get(&name) else {
                    return Vec::new();
                };
                let Some(first) = package.entries.first() else {
                    return Vec::new();
                };
                let bundle = self
                    .folder
                    .join(format!("bundles.{:02}.nxa", first.bundle_index));
                self.get_resource_from_bundle(&bundle, first.start_offset as u64)
            }
            PackageType::Dsar => self.get_resource_from_bundle(&full_path, 0),
            PackageType::Legacy => {
                let Ok(mut f) = File::open(&full_path) else {
                    return Vec::new();
                };
                let Some(head) = read_at(&mut f, 0, 12) else {
                    return Vec::new();
                };
                if u32_at(&head, 0) != Some(LEGACY_MAGIC) {
                    return Vec::new();
                }
                let num_types = u32_at(&head, 4).unwrap() as usize;
                let num_files = u32_at(&head, 8).unwrap() as usize;
                let toc_len = 72 + num_types * 32 + num_files * 80;
                read_at(&mut f, 0, toc_len).unwrap_or_default()
            }
        }
    }

    /// Returns the full decompressed `.stream` companion for a package (a
    /// whole-file reconstruction, unlike [`Self::get_resource_from_package`]'s
    /// single-resource read at a known offset). Package type is decided from the
    /// main (no-suffix) file, matching the Python `load_package`'s single
    /// dispatch reused for toc/gpu/stream. Unlike [`Self::get_package_toc`]'s
    /// legacy branch, no legacy-TOC magic check is performed here — a `.stream`
    /// payload is raw audio/media data, not a TOC, so it never carries that
    /// magic. Bundled/DSAR branches reconstruct the *whole* `.stream` package
    /// ([`Self::reconstruct_package_from_bundles`] / whole-file
    /// [`Self::decompress_dsar`]) rather than just its first entry/chunk-chain
    /// — a `.stream` companion can span multiple bundle entries same as a main
    /// TOC file can, matching `load_package`'s own dispatch for `.stream`.
    pub fn get_stream_resource(&self, package_name: &str) -> Vec<u8> {
        let name = basename_str(package_name);
        let full_path = self.folder.join(&name);
        let stream_name = format!("{name}.stream");
        match self.package_type(&full_path) {
            PackageType::Bundled => self.reconstruct_package_from_bundles(&stream_name),
            PackageType::Dsar => self.decompress_dsar(&self.folder.join(&stream_name)),
            PackageType::Legacy => std::fs::read(self.folder.join(&stream_name)).unwrap_or_default(),
        }
    }

    /// Decodes `bundles.nxa` into the package index and per-bundle chunk-offset
    /// maps.
    fn init_bundle_mapping(&mut self) {
        let contents = self.decompress_dsar(&self.folder.join("bundles.nxa"));
        let num_packages = match u32_at(&contents, 0x10) {
            Some(n) => n as usize,
            None => return,
        };

        // Per-bundle map: uncompressed chunk offset -> chunk index.
        if let Ok(entries) = std::fs::read_dir(&self.folder) {
            for entry in entries.flatten() {
                if !entry.path().is_file() {
                    continue;
                }
                let filename = entry.file_name().to_string_lossy().into_owned();
                if filename.contains(".patch") || !is_bundle_like(&filename) {
                    continue;
                }
                let Ok(mut file) = File::open(entry.path()) else {
                    continue;
                };
                let Some(head) = read_at(&mut file, 0, 0x20) else {
                    continue;
                };
                let num_chunks = u32_at(&head, 8).unwrap_or(0) as usize;
                let Some(chunk_headers) = read_at(&mut file, 0x20, 0x20 * num_chunks) else {
                    continue;
                };
                let mut map = HashMap::with_capacity(num_chunks);
                for j in 0..num_chunks {
                    let uncompressed_offset = u64_at(&chunk_headers, 0x20 * j).unwrap();
                    map.insert(uncompressed_offset, j);
                }
                self.bundle_offsets.insert(filename, map);
            }
        }

        // Package table at 0x18: <QIII4x> per package.
        for n in 0..num_packages {
            let base = 0x18 + 24 * n;
            let (Some(bundle_size), Some(name_offset), Some(items_count), Some(items_offset)) = (
                u64_at(&contents, base),
                u32_at(&contents, base + 8),
                u32_at(&contents, base + 12),
                u32_at(&contents, base + 16),
            ) else {
                return;
            };
            let name = cstring_at(&contents, name_offset as usize);
            let mut entries = Vec::with_capacity(items_count as usize);
            for i in 0..items_count as usize {
                let ib = items_offset as usize + 16 * i; // <QI3xB> = 16 bytes
                let original_archive_offset = u64_at(&contents, ib).unwrap();
                let start_offset = u32_at(&contents, ib + 8).unwrap();
                let bundle_index = contents[ib + 15];
                entries.push(BundleEntry {
                    original_archive_offset,
                    start_offset,
                    bundle_index,
                });
            }
            self.package_contents.insert(
                name,
                Package {
                    size: bundle_size,
                    entries,
                },
            );
        }
    }
}

fn decompress_chunk(temp: &[u8], compression_type: u8, uncompressed_size: usize) -> Vec<u8> {
    match compression_type {
        COMPRESSED => lz4_flex::block::decompress(temp, uncompressed_size).unwrap_or_default(),
        UNCOMPRESSED => temp.to_vec(),
        _ => temp.to_vec(),
    }
}

fn is_bundle_like(filename: &str) -> bool {
    match Path::new(filename).extension() {
        None => true,
        Some(ext) => matches!(
            ext.to_string_lossy().as_ref(),
            "stream" | "nxa" | "gpu_resources"
        ),
    }
}

fn basename(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn basename_str(name: &str) -> String {
    basename(Path::new(name))
}

/// Tests for the whole-package/whole-file reconstruction paths
/// ([`Slim::load_package_toc`], [`Slim::get_stream_resource`]'s DSAR/Bundled
/// branches, [`Slim::reconstruct_package_from_bundles`]) added when phase 6
/// wired `Slim` into real base-archive loading. These specifically pin the
/// "read everything, not just the first chunk-chain/entry" behavior those
/// paths need but [`Slim::get_package_toc`]'s deliberately-narrower
/// first-entry-only header scan does not — constructed by hand since no
/// integration test builds real DSAR/bundle files elsewhere in this crate.
#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a DSAR-format blob: a 0x20-byte main header (`num_chunks` @
    /// offset 8), one 0x20-byte chunk header per chunk (`uncompressed_offset`
    /// cumulative @0, `compressed_offset` (absolute file position of its
    /// payload) @8, `uncompressed_size`/`compressed_size` @16/@20 — equal
    /// here since every chunk is stored uncompressed for simplicity —
    /// `compression_type` @24, `chunk_type` @25), then the chunk payloads
    /// back-to-back. Mirrors the layout `decompress_dsar`/
    /// `get_resource_from_bundle` both read.
    fn build_dsar(chunks: &[&[u8]], starts: &[bool]) -> Vec<u8> {
        let num_chunks = chunks.len();
        let mut payload_offset = (0x20 + 0x20 * num_chunks) as u64;
        let mut uncompressed_cum = 0u64;
        let mut headers = Vec::new();
        let mut payloads = Vec::new();
        for (i, chunk) in chunks.iter().enumerate() {
            let mut h = vec![0u8; 0x20];
            h[0..8].copy_from_slice(&uncompressed_cum.to_le_bytes());
            h[8..16].copy_from_slice(&payload_offset.to_le_bytes());
            h[16..20].copy_from_slice(&(chunk.len() as u32).to_le_bytes());
            h[20..24].copy_from_slice(&(chunk.len() as u32).to_le_bytes());
            h[24] = UNCOMPRESSED;
            h[25] = if starts[i] { START } else { 0 };
            headers.extend_from_slice(&h);
            payloads.extend_from_slice(chunk);
            uncompressed_cum += chunk.len() as u64;
            payload_offset += chunk.len() as u64;
        }
        let mut out = vec![0u8; 0x20];
        out[8..12].copy_from_slice(&(num_chunks as u32).to_le_bytes());
        out.extend_from_slice(&headers);
        out.extend_from_slice(&payloads);
        out
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("hd2-slim-test-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn empty_slim(folder: PathBuf) -> Slim {
        Slim {
            folder,
            is_slim: true,
            package_contents: HashMap::new(),
            bundle_offsets: HashMap::new(),
        }
    }

    #[test]
    fn decompress_dsar_reads_every_chunk_not_just_the_first_chain() {
        // Two separate START chunk-chains — `get_resource_from_bundle` would
        // stop after the first; whole-file decompression must return both.
        let a = b"first-resource-AAAA".as_slice();
        let b = b"second-resource-BBBB".as_slice();
        let dir = tmp_dir("dsar-whole");
        let path = dir.join("thing");
        std::fs::write(&path, build_dsar(&[a, b], &[true, true])).unwrap();

        let got = empty_slim(dir.clone()).decompress_dsar(&path);
        assert_eq!(got, [a, b].concat());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_stream_resource_dsar_branch_reads_the_whole_stream_file() {
        let a = b"stream-part-one!!".as_slice();
        let b = b"stream-part-two!!".as_slice();
        let dir = tmp_dir("dsar-stream");
        // `package_type()` classifies by the *main* (no-suffix) file's magic;
        // give it the DSAR magic so `get_stream_resource` takes the Dsar
        // branch for the ".stream" companion.
        std::fs::write(dir.join("pkg"), DSAR_MAGIC.to_le_bytes()).unwrap();
        std::fs::write(dir.join("pkg.stream"), build_dsar(&[a, b], &[true, true])).unwrap();

        let got = empty_slim(dir.clone()).get_stream_resource("pkg");
        assert_eq!(got, [a, b].concat());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reconstruct_package_from_bundles_stitches_every_entry_not_just_the_first() {
        // A package split across two entries in the same bundle file, each
        // its own START chunk-chain — the pre-phase-6 code only ever read
        // `entries[0]`, silently truncating any package needing a second
        // entry.
        let a = b"entryA-bytes".as_slice();
        let b = b"entryB-longer-bytes".as_slice();
        let dir = tmp_dir("bundled");
        std::fs::write(dir.join("bundles.00.nxa"), build_dsar(&[a, b], &[true, true])).unwrap();

        let mut package_contents = HashMap::new();
        package_contents.insert(
            "pkg".to_string(),
            Package {
                size: (a.len() + b.len()) as u64,
                entries: vec![
                    BundleEntry {
                        original_archive_offset: 0,
                        start_offset: 0,
                        bundle_index: 0,
                    },
                    BundleEntry {
                        original_archive_offset: a.len() as u64,
                        start_offset: a.len() as u32,
                        bundle_index: 0,
                    },
                ],
            },
        );
        let mut bundle_offsets = HashMap::new();
        bundle_offsets.insert("bundles.00.nxa".to_string(), HashMap::from([(0u64, 0usize), (a.len() as u64, 1usize)]));

        let slim = Slim {
            folder: dir.clone(),
            is_slim: true,
            package_contents,
            bundle_offsets,
        };
        let got = slim.load_package_toc("pkg");
        assert_eq!(got, [a, b].concat());

        let _ = std::fs::remove_dir_all(&dir);
    }
}

fn cstring_at(buf: &[u8], start: usize) -> String {
    let end = buf[start..]
        .iter()
        .position(|&b| b == 0)
        .map(|p| start + p)
        .unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[start..end]).into_owned()
}
