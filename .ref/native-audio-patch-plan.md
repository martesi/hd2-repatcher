# Repatcher: native Rust audio-mod patching (retire the external audio tool)

## Context

`hd2-repatcher` repatches unit mods natively (`crates/engine`), but audio mods are
delegated to an external Python tool, `hd2-audio-modder` (sibling repo), via subprocess
— see `.ref/audio-mod-repatch-plan.md` for that design. That delegation is wired into
the CLI path only (`src-tauri/src/audio.rs`, `cli.rs`); the GUI's drag-drop path
(`commands.rs::run_batch`) never even classifies patches, so audio mods silently land
in "skipped" there today regardless.

Goal: implement the audio-patch operation natively in Rust so the app never has to
shell out to an external tool at all — for the CLI or the GUI.

Scope is exactly what the audio tool's headless `run_patch_cli`
(`hd2-audio-modder/audio_modder.py:3647-3737`) does, no more:

```
GameArchive.from_file(patch)  →  Mod.import_patch(patch)  →  Mod.write_patch / write_separate_patches  →  GameArchive.to_file
```

`import_patch` (`core.py:1994-2106`) merges, per patch file: changed audio-source
(`.wem`) bytes, HIRC hierarchy entries (`import_wwise_hierarchy`), text-bank strings,
and video sources, into a base `Mod` loaded from the current game data; `write_patch`
serializes the result back into `.patch_N` TOC format. This is pure byte-level
`.wem`-for-`.wem` swapping and binary-format re-serialization — it never transcodes
audio, so no external SDK (ffmpeg, vgmstream, WwiseConsole) is needed either way,
confirmed by tracing every function `run_patch_cli` actually calls.

Explicitly **out of scope**: WAV/MP3 import/export, playback/preview, "dummy bank"
creation, Wwise-project migration — none of that is reachable from `import_patch`/
`write_patch` and none of it is ported.

## Why this is smaller than the raw line counts suggest

The low-level plumbing audio patching needs already exists in Rust, ported for unit
patching, and is directly reusable as-is:

- `crates/engine/src/memstream.rs` — `MemoryStream` binary read/write cursor.
- `crates/engine/src/slim.rs` — DSAR/bundle decompression (`lz4_flex`-based),
  `get_package_toc`/`get_resource_from_package`.
- `crates/engine/src/resources.rs` — game-archive TOC scanning
  (`GameResources::load_game_resources`), currently indexes `UNIT_TYPE_ID` only.

Only the Wwise-specific format layer is new: bank chunk parsing (BKHD/DIDX/DATA/HIRC)
and the HIRC hierarchy object graph for bank versions 140/154. Three findings shrink
that further:

1. **Most HIRC entry types are opaque passthrough, even in the Python source.**
   `import_hierarchy`'s field-level merge (`wwise_hierarchy_154.py:2204-2211`) only
   touches **4 types**: `Sound`, `MusicTrack`, `MusicSegment`,
   `RandomSequenceContainer`. The children-merge paths (`core.py:822-829`, `:1907-1918`)
   need **5 container types** with a `ContainerChildren` list: those same
   `RandomSequenceContainer` plus `ActorMixer`, `SwitchContainer`, `LayerContainer`,
   `MusicSwitchContainer`. That's **8 structured types total** (of ~22 named HIRC
   types) needing real field parsing — everything else (`Action*`, `Event`, `State`,
   `AudioBus`, …) round-trips as opaque `(type, size, id, misc bytes)`, exactly how the
   Python base `HircEntry` already treats unhandled types.
2. **No `Slim`/decompression involvement for patch files.** `.patch_N` files (and
   their `.stream` companions) are always plain/uncompressed; only *base game archive*
   loading touches `Slim`. One small addition: `Slim::get_stream_resource`, which reads
   a `.stream` companion without the legacy-TOC magic check that's correct for a main
   package but wrong for raw `.stream` payload.
3. **No external `friendlynames.db`/network dependency needed.** The Python CLI
   downloads a sqlite DB to map a soundbank id to its containing base archive — exactly
   the problem `resources.rs::load_game_resources` already solves for units by scanning
   every package's TOC. Generalize that scan to also index `WWISE_BANK`; text banks
   don't even need indexing (`run_patch_cli` hardcodes their archive as
   `9ba626afa44a3aa3`); streams/video/deps ride along once their containing bank's
   archive loads.

## Module structure

New submodule tree under `crates/engine/src/wwise/`:

```
wwise/
  mod.rs            — re-exports, shared constants (VORBIS, REV_AUDIO, BANK_VERSION_KEY)
  bank_parser.rs     — BankParser (RIFF-chunk reader), DidxEntry, MediaIndex
  audio_source.rs    — AudioSource, WwiseStream, WwiseDep (no `parents` back-refs — see below)
  text_bank.rs       — TextBank, StringEntry, import_text
  video.rs           — VideoSource
  archive.rs         — GameArchive (from_file/to_file/load), TocHeader
  mod_.rs            — Mod: import_patch, import_wwise_hierarchy, write_patch,
                        write_separate_patches, add_game_archive, load_archive_file
  hierarchy/
    mod.rs           — HircEntry enum, BankVersion, WwiseHierarchy (load/get_data/import_hierarchy)
    base_param.rs    — BaseParam + PropBundle/RangedPropBundle/AuxParams/StateParams/RTPC/FxChunk
                        (appears version-identical between 140/154 — confirm via diff before porting once)
    sound.rs         — Sound, BankSourceStruct (v154 adds a `cache_id` field)
    music_track.rs   — MusicTrack, TrackInfoStruct (largest version divergence — see below)
    music_segment.rs — MusicSegment (v140 = ad hoc raw-blob skips; v154 = BaseParam)
    containers.rs    — RandomSequenceContainer, ActorMixer, SwitchContainer, LayerContainer,
                        MusicSwitchContainer, ContainerChildren
    opaque.rs         — generic (hierarchy_type, size, id, misc-bytes) passthrough entry
audio_resources.rs   — AudioIndex: WWISE_BANK id -> base archive name, generalizing
                        resources.rs's scan pattern
```

`lib.rs` gains `mod wwise;`, `mod audio_resources;`, a `process_audio_patches(...)`
orchestration entry point, and extends `PatchResult`/`process_patch_files` to actually
process (not just bucket) audio-classified files.

## Data model decisions

Deviate from Python's mutable object graph where it's pure editing-session bookkeeping
we don't reach (undo/redo, `modified_children` propagation, GUI parent pointers):

- No `parent`/back-reference fields on `HircEntry` variants; parent lookups go through
  the owning `WwiseHierarchy`'s id map instead of Python object pointers.
- `AudioSource.parents` is dropped — its only consumer is a `MusicTrack`
  duration-sync branch that is **dead/commented-out code** in `core.py:2027-2053`
  (confirmed unreachable).
- Keep `modified: bool` only where `write_patch`/`write_separate_patches` actually
  filter on it (`AudioSource`, `WwiseStream`, `WwiseBank`, `TextBank`, `VideoSource`).
- **Ordering matters for byte-exactness.** Python's `dict[int, HircEntry]` is
  insertion-ordered and that order feeds directly into serialization. Use
  `indexmap::IndexMap` (new dependency) instead of `HashMap` anywhere Python relies on
  dict iteration order for `to_file` output (`WwiseHierarchy::entries`,
  `GameArchive::wwise_banks`/`wwise_streams`/etc.) — this is a correctness requirement
  for golden tests to pass, not a style choice.

`Mod` is trimmed to only what `import_patch`/`write_patch`/`write_separate_patches`/
`add_game_archive`/`load_archive_file` need — no `import_wems`/`import_wavs`/
`import_files`, no `dump_*`, no `create_dummy_bank`, no hierarchy CRUD beyond import, no
`db` constructor argument (GUI undo/redo persistence only).

## HIRC version unification

One struct per HIRC type, each `read`/`write` taking a `BankVersion` parameter and
branching only at the fields that differ — a single implementation, not two mirrored
files. Confirmed deltas (re-verify each with a targeted diff before implementing):

- `BankSourceStruct`: v154 inserts a `cache_id: u32` before `mem_size`
  (`wwise_hierarchy_154.py:1994-2004` vs `_140.py:1918-1928`).
- `MusicSegment`/`MusicTrack`: the largest divergence — v154 reads a `bit_flags: u8`
  then a full `BaseParam`; v140 does ad hoc fixed-width skips with no `BaseParam` at
  all. Model as one struct with a version-tagged inner enum
  (`MusicSegmentParams::V140Raw(Vec<Vec<u8>>) | V154(BaseParam)`) rather than forcing a
  shared representation — give this its own careful pass.
- `BaseParam` and shared sub-structures (`PropBundle`, `RangedPropBundle`, `AuxParams`,
  `StateParams`, `RTPC`, `FxChunk`, `ContainerChildren`) appear byte-identical between
  versions — confirm, then port once.

Dispatch (`HircEntryFactory` equivalent) is a single `match` on the `hierarchy_type`
byte producing one of the 8 structured variants or `HircEntry::Opaque` — dispatch
itself doesn't change between bank versions, only what happens inside each parser.

## Archive index (replacing the external friendlynames DB)

`audio_resources.rs::AudioIndex` mirrors `resources.rs::GameResources`, keyed by
`WWISE_BANK` entries instead of `UNIT_TYPE_ID`, storing the archive's display name (the
whole archive loads via `Mod::load_archive_file`, not a single resource read). Refactor
`resources.rs::load_resources_from_file`'s TOC-scan loop into a shared
`scan_package_toc` walker parameterized by type id, used by both `GameResources`
(units) and `AudioIndex` (audio). Build both indexes in one pass over game packages
(one `Slim` init, one directory walk) by extending `GameResources::load` to also
produce the audio index, exposed via `GameResources::audio_index()`.

## Integration

**`crates/engine/src/lib.rs`**: add `process_audio_patches(dir, files, game_data, resources)`
— resolves touched base archives via `AudioIndex`, builds a `wwise::mod_::Mod`, loads
each touched archive, calls `import_patch` per patch file (sorted by name, matching
Python's `sorted(os.listdir(...))`), then `write_patch(dir, "9ba626afa44a3aa3.patch_0")`.
Direct replacement for the subprocess call in `audio::repatch_audio_dir`. Extend
`process_patch_files`/`process_patch_folder` so `Audio`/`UnitAndAudio`-classified files
actually get processed (grouped by containing directory, reusing
`audio.rs::find_audio_dirs`'s grouping logic, which moves into the engine since it's
pure classification with no subprocess dependency once native). Extend `PatchResult` to
distinguish "classified as audio" from "successfully repatched"
(`audio_updated: Vec<String>`, `audio_failed: Vec<(String, String)>`).

**`src-tauri/src/audio.rs`**: subprocess-calling functions (`repatch_audio_mod`,
`repatch_audio_dir`, `move_dir_contents`, `move_path`, `make_temp_dir`) become dead code
— delete. Keep `remove_stale_audio_files` (native `write_patch` still produces the
fixed filename `9ba626afa44a3aa3.patch_0`, still orphaning the original per-mod
`.patch_N` inputs) and `copy_dir_recursive` (still needed for dry-run staging).
`find_audio_dirs` moves into the engine. Once trimmed, consider folding the remainder
into `commands.rs`/`cli.rs` and deleting this file.

**`src-tauri/src/cli.rs`** (`run_cli`, ~lines 90-176): replace the
`audio::find_audio_dirs` + `match audio_tool { ... }` block with a direct call to
`engine::process_audio_patches` per directory. Delete the `--audio-tool` flag,
`Args.audio_tool`, and the "no audio tool configured" error path — audio patching works
whenever game data is configured, same as units.

**`src-tauri/src/commands.rs`** — fixes the pre-existing GUI wiring bug as a byproduct:
`run_batch` currently calls only `engine::find_patch_files` + `engine::update_patch_file`
(never classifies, so audio silently lands in "skipped" today). Switch to
`engine::process_patch_folder`/`process_patch_files` (a strict superset for unit
patches, plus newly-correct audio handling). Populate `BatchProgress`/
`BatchResult.audio` from `PatchResult.audio_updated.len()` — the frontend TS type
(`src/lib/tauri.ts`) and `batch-list.tsx` **already** render this field
(`"{batch.audio} audio delegated"`), it's just never populated today; relabel to "audio
patched". Delete `set_audio_tool_path`/`clear_audio_tool_path` commands and
`audio_tool_path`/`audio_tool_valid` from `Config`; drop both from `gui.rs`'s
`generate_handler!` list.

**`crates/engine/src/settings.rs`**: drop `audio_tool_path` and its
get/set_cached_audio_tool_path helpers. `Settings` already tolerates unknown/missing
keys, so this is backward-compatible with existing `settings.json` files.

**Frontend** (`src/lib/tauri.ts`, `src/routes/settings.tsx`): remove `audioToolPath`/
`audioToolValid` from `AppConfig`, `setAudioToolPath`/`clearAudioToolPath` from `api`,
and the "Audio tool" `Card` block + handlers in `settings.tsx`. Keep `BatchProgress`/
`BatchResult.audio` and its rendering, just relabel.

**Full removal, not a fallback.** The goal is eliminating the subprocess dependency;
keeping a partial fallback path doubles the surface area to maintain for a case
(external tool not installed) that becomes the common case, not the edge case, once
native processing lands.

## Testing strategy

Follow the existing `tools/gen_golden.py` + `crates/engine/tests/golden.rs` pattern
(byte-exact diff against a trimmed Python oracle) — this repo already vendors trimmed,
dependency-light Python references under `reference/` for exactly this purpose.

1. Vendor trimmed audio oracles into `reference/`: `audio_core.py` (from `core.py`,
   dropping `SoundHandler`/`dump_*`/`import_wems`/`import_wavs`/`create_dummy_bank`/
   everything GUI-only; keeping `GameArchive`, the trimmed `Mod`, `AudioSource`,
   `WwiseBank`, `WwiseStream`, `WwiseDep`, `TextBank`, `StringEntry`, `VideoSource`,
   `BankParser`), `wwise_hierarchy_140.py`/`wwise_hierarchy_154.py` (near-verbatim),
   `audio_util.py`, `audio_const.py`.
2. New `tools/gen_audio_golden.py`, parallel to `gen_golden.py`: hand-build small
   synthetic `GameArchive`s (never real HD2 assets — copyright, and the existing unit
   fixtures already establish the synthetic-filler convention) plus synthetic patch
   files, run them through the vendored oracle, write `input/`/`expected/` fixtures
   under `crates/engine/tests/fixtures/audio_<case>/`.
3. New `crates/engine/tests/audio_golden.rs` mirroring `golden.rs`: run the Rust
   `Mod`/`GameArchive` pipeline against each fixture, byte-diff the written patch
   against `expected/`.
4. Coverage order: (a) TOC/archive round-trip with all-opaque HIRC, (b) `Sound` +
   audio-byte swap, (c) `import_hierarchy` field-merge for the 4 eligible types, (d)
   children-merge across the 5 container types, (e) text-bank import, (f) video both
   branches, (g) merged vs. separate-patch output shape, (h) `AudioIndex`/`Slim`
   end-to-end. Pure-Rust unit tests (no oracle needed) for `BankParser` chunk-splitting
   and `AudioIndex` map-building, following `classify.rs`'s style.

## Phasing (each phase independently reviewable and testable)

0. **Infrastructure, no behavior change**: extend `memstream.rs` with the typed
   readers HIRC parsing needs (`read_u8/i8/u16/i16/i32/i64/u64/f32`); add `indexmap`
   dependency; add `Slim::get_stream_resource`; vendor trimmed Python oracles into
   `reference/`.
1. **TOC/archive skeleton, opaque HIRC only**: `bank_parser.rs`, `audio_source.rs`,
   `text_bank.rs`, `video.rs`, `archive.rs`, `HircEntry::Opaque` only. Golden fixture:
   archive round-trip + one audio-byte swap, proving the serialization machinery before
   touching HIRC semantics.
2. **`BaseParam` + `Sound`**: DIDX/DATA regeneration golden fixture.
3. **`MusicTrack`, `MusicSegment`, `RandomSequenceContainer` + `import_hierarchy`
   field-merge**: budget as the largest chunk given `MusicSegment`'s v140/v154
   divergence.
4. **The five children-container types + cross-bank/cross-archive merge**.
5. **Full `Mod::import_patch`/`write_patch`/`write_separate_patches` orchestration**:
   wires phases 1-4 into the real control flow, including video and text-bank
   branches. End-to-end multi-resource-type golden fixture.
6. **`AudioIndex` + `Slim` integration**.
7. **Engine orchestration entry point** (`lib.rs::process_audio_patches`,
   `PatchResult` extension) — should be small given 1-6 landed correctly.
8. **`src-tauri` integration**: `cli.rs`, `commands.rs::run_batch` fix, `audio.rs`
   trim/removal, `settings.rs` cleanup, frontend cleanup. Finish with a manual smoke
   test against a real (non-committed) HD2 install and a real third-party audio mod,
   since golden fixtures are necessarily synthetic.

## Verification

- `bun run check:agent` (typecheck + `cargo check` + biome format) after each phase,
  per CLAUDE.md.
- `cargo test -p engine` for golden/unit tests at each phase boundary.
- Manual smoke test in phase 8: drag-drop a real audio mod onto the GUI and confirm it
  patches correctly against a real (updated) game install; verify the CLI `patch`
  output matches what the old subprocess-delegated path used to produce, for at least
  one real-world mod, before considering the port complete.
- Commit with `jj` (not bare git — this repo has no `.git` checkout, only `.jj`) once
  `check:agent` passes, per CLAUDE.md.

## Open questions / risks

- `MusicSegment`/`MusicTrack` v140-vs-v154 divergence (phase 3) is the highest-risk
  single piece — the two Python versions don't even build the same intermediate
  representation. Read both `from_memory_stream`/`get_data`/`set_data` pairs in full
  before implementing; don't over-unify if the honest representation is "two shapes
  behind one enum."
- This plan supersedes the delegation half of `.ref/audio-mod-repatch-plan.md` (that
  doc's dry-run/`-o` and stale-file-removal design is still relevant and reused; its
  `--audio-tool` flag, settings field, and subprocess plumbing are removed by this
  plan, not extended).
