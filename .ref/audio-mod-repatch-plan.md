# Repatcher: audio-mod support + dry-run

## Context

`hd2-repatcher` today only repatches **unit** mods. It recursively finds every
`.patch` file under each mod folder and rewrites unit `lod_group` data **in
place** (`engine::update_patch_file` writes back to the same path). Any patch
file that contains no unit resource is bucketed as `no_units` and skipped —
**audio mods fall into that bucket and are silently ignored.**

We want the repatcher to also handle **audio mods**. It cannot repatch audio
itself; instead it delegates each audio mod to an **external audio tool** that
exposes a headless CLI. The tool's real contract (see
`hd2-audio-modder/docs/cli-patch-plan.md`) is:

```
<audio-tool> patch -i <mod folder> -o <output folder> [-g <game data>] [-s/--separate]
```

- `-i` is **one** mod folder, scanned **top-level only (non-recursive)**; every
  `.patch_N` in it is loaded and **merged into a single mod**.
- `-o` receives the output **directly, no subfolders**. Default: **one merged**
  file `9ba626afa44a3aa3.patch_0` (+ `.stream` if applicable). `-s`: one patch
  per touched game archive (`{archive}.patch_0`).
- `-g` optional (the tool falls back to its own `config.pickle`); the repatcher
  passes the same game-data path it already has.
- The tool needs `friendlynames.db` beside it and exits non-zero on any failure —
  the repatcher just surfaces that.

That interface is assumed to already exist on the audio tool's side — this task
only changes the **repatcher**. Two behaviours the repatcher owns:

1. **Stale-file removal.** The tool does **not** delete anything; it only writes
   fresh output into `-o`. Because it *merges* (default output name
   `9ba626afa44a3aa3.patch_0` differs from the original `.patch_N` file names),
   the mod folder's original audio patch files become orphaned. **The repatcher
   removes those stale files** from the destination before moving the freshly
   patched output in.
2. **Dry-run.** Unit patching currently writes in place. Dry-run is a
   repatcher-side mode that **operates on a copy** of the mod files, leaving the
   originals untouched — for both unit patches and audio delegation.

## Interface decisions (assumptions — flag if you disagree)

- **Locating the audio tool:** new `--audio-tool PATH` flag, **cached in
  settings.json** (new `audio_tool_path` field) exactly like `-g/--game`. Omit
  it on later runs once cached.
- **Dry-run / output destination:** new `-o/--output DIR` (copies land there) and
  `-n/--dry-run`. Dry-run without `-o` copies each mod folder to a sibling
  `<name>-repatched/`. Originals are never modified in dry-run.

## Behaviour matrix

| Mod kind | Normal mode | Dry-run mode |
|---|---|---|
| Unit  | `update_patch_file` in place (today) | patch the **copy** in the output dir |
| Audio | audio tool → fresh temp `-o`; repatcher deletes stale audio files in folder, moves patched output in | audio tool `-i <orig> -o <copy dir>`; originals untouched |

A folder may contain both kinds; each patch file is classified independently and
the folder is delegated to the audio tool once if it holds any audio patch. The
audio tool is **non-recursive**, so the repatcher delegates **each directory
that directly contains audio `.patch_N` files** (walking subdirectories to find
them), passing that directory as `-i`.

## Changes

### 1. Engine — classification (`crates/engine/src/`)

- **`lib.rs`**: add audio type-id constants (from the audio tool's `const.py`):
  `WWISE_BANK = 6006249203084351385`, `WWISE_STREAM = 5785811756662211598`,
  `WWISE_DEP = 12624162998411505776`, `TEXT_BANK/STRING = 979299457696010195`,
  `BINK_VIDEO = 6838244362054241717` (alongside existing `UNIT_TYPE_ID`).
- Add `pub enum PatchKind { Unit, Audio, UnitAndAudio, Other, Corrupted }` and
  `pub fn classify_patch_file(path: &Path) -> PatchKind`. This does its **own**
  read + type-table scan (the same 72-byte header → 32-byte type entries walk
  already in `patch.rs::update_patch_file`), collecting the set of `type_id`s and
  testing membership. **Keep it separate from `update_patch_file`** rather than
  refactoring that hot path, so the byte-exact golden tests (`tests/golden.rs`)
  stay untouched. Small, well-contained duplication of the type-table read.
- Extend `PatchResult` (`lib.rs`) with an `audio: Vec<String>` bucket so audio
  patches stop being reported as `no_units`. `process_patch_files` routes a file
  to `audio` when `classify_patch_file` reports Audio/UnitAndAudio (units in a
  UnitAndAudio file are still patched by `update_patch_file`).

### 2. src-tauri — audio delegation (`src-tauri/src/audio.rs`, new)

- `fn repatch_audio_mod(tool: &Path, game_data: &Path, in_dir: &Path, out_dir: &Path) -> Result<(), String>`:
  spawns `Command::new(tool).args(["patch","-i",in_dir,"-o",out_dir,"-g",game_data])`
  (real contract order; `-s` omitted → default merged output), captures
  status/stderr, maps a non-zero exit to a clear error.
- `fn remove_stale_audio_files(folder: &Path, audio_patches: &[PathBuf])`: deletes
  each classified audio patch plus its `.stream` / `.gpu_resources` companions
  from the destination **before** the patched output is moved in. Needed because
  the tool merges to `9ba626afa44a3aa3.patch_0`, a different name than the inputs.
- Orchestration for one mod folder (in-place / normal): run tool into a **fresh
  temp** `-o`, `remove_stale_audio_files`, move temp output into the folder.
- Engine stays subprocess-free (pure/fast tests); all `std::process` + fs
  relocation lives here in the tauri layer.

### 3. src-tauri — CLI wiring (`src-tauri/src/cli.rs`)

- Add clap args: `--audio-tool PATH`, `-o/--output DIR`, `-n/--dry-run`.
- `--audio-tool` validated + cached via settings when provided (mirror the
  existing `-g` caching block).
- New per-folder flow replacing the direct `process_patch_folder` call:
  1. If dry-run/`-o`: copy the mod folder to the output location; treat the copy
     as the working dir. Else work in place.
  2. Run unit patching (`process_patch_folder`) on the working dir.
  3. For each directory in the working dir that directly holds audio patches, if
     `audio_tool_path` is set: run `audio::repatch_audio_mod` into a fresh temp
     `-o`, delete stale audio files in that directory, move the temp output in.
     If audio patches exist but no tool is configured: report + non-zero exit.
- Extend `print_cli_result` to report delegated audio mods and dry-run output
  locations.

### 4. Settings (`crates/engine/src/settings.rs`)

- Add `pub audio_tool_path: Option<String>` (`skip_serializing_if = "Option::is_none"`,
  preserves Python-compat) plus `get_/set_cached_audio_tool_path` helpers mirroring
  the game-data-path ones.

### 5. GUI (follow-up, lighter touch)

- `commands.rs`: surface `audio_tool_path` in `Config`, add a `set_audio_tool`
  command; `run_batch` gains the same audio-delegation + dry-run branch.
- Frontend Settings page: field for the audio tool path; Home batch reporting
  gains an "audio mods delegated" count.
- Can land after the CLI/engine core if we want to keep the first change focused.

## Verification

- **Engine unit tests** (`crates/engine/tests/`): add fixtures for a
  synthetic audio-only patch (WWISE type ids) and a mixed unit+audio patch;
  assert `classify_patch_file` returns `Audio` / `UnitAndAudio`. Confirm existing
  `no_units` fixture still classifies as `Other` and golden tests are unchanged.
- **CLI, manual**: with a real audio mod folder and a stub `--audio-tool` script
  that echoes its args and writes a dummy `-o` output, verify:
  - the tool is invoked as `patch -i <in> -o <out> -g <data>`;
  - the merged `9ba626afa44a3aa3.patch_0` lands in the folder and the original
    `.patch_N` (+ `.stream`/`.gpu_resources`) stale files are removed;
  - `--dry-run` leaves the original folder byte-identical and produces the copy.
- **Regression**: a pure unit mod still repatches in place identically (diff
  against a pre-change run).
- Run `bun run check:agent` (typecheck + `cargo check` + biome) before done, per
  CLAUDE.md. Commit with `jj` only if in auto mode.

## Open assumptions

- Audio-tool CLI is `patch -i <folder> -o <folder> [-g <data>] [-s]` per
  `hd2-audio-modder/docs/cli-patch-plan.md`, exits non-zero on failure, and is a
  single invokable executable (built binary / wrapper) that takes `patch` as its
  first arg. `--audio-tool` points at that executable.
- The repatcher uses the tool's **default merge** (single
  `9ba626afa44a3aa3.patch_0`); we do **not** pass `-s`. (Easy to add a
  `--separate` passthrough later if a mod's per-archive structure must be kept.)
- "Stale files" = the classified audio `.patch_N` files and their
  `.stream`/`.gpu_resources` companions in the destination directory; unit
  patches and unrelated files are left alone.
