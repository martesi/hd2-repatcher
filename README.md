# HD2 Repatcher

Repatches Helldivers II unit and audio mods after a game update.

When Helldivers II updates, unit and audio mod `.patch_N` files can go out of
sync with the game's resources, causing them to fail to load. This tool updates
one patch group at a time in place, preserving each main filename and its
optional `.stream` and `.gpu_resources` companions.

It is a single [Tauri](https://tauri.app) app with a native Rust engine:

- **Double-click** it for a GUI — drag mod folders in and watch them repatch.
- **Run it with arguments** for a headless CLI — no window is ever created, so
  it drops into mod managers and scripts. Drag-and-drop onto the executable
  works too.

## Features

- **Batch repatching** — drop mod folders or individual patch files at once;
  folders are processed one patch group at a time with live progress.
- **GUI or headless CLI** from the same binary, so it works both as a
  point-and-click tool and as a step in a mod manager's pipeline.
- **Native Rust engine** — a behaviour-preserving, byte-exact port of the
  original Python unit and audio tools, pinned by differential golden tests (see
  [Testing](#testing)).
- **Cross-platform** — prebuilt Windows and Linux installers, no runtime
  dependencies.
- **Remembers your setup** — the Helldivers II install root, language, theme, and accent
  color persist across runs.
- **In-place safety** — work is staged and validated before only the selected
  group is replaced; unrelated files and existing companions are preserved.
- **Corruption reporting** — corrupted patch groups are surfaced with a
  non-zero exit code / in-UI error instead of being silently skipped.

## Requirements

- A Helldivers II install. The app derives and uses its `data` folder.
- Windows or Linux. Prebuilt installers are attached to each
  [Release](../../releases); no runtime dependencies.

## Installation

1. Grab the installer for your platform from the
   [Releases page](../../releases):
   - Windows: the `.exe` (NSIS) installer.
   - Linux: the `.deb` package or the `.AppImage`.
2. Run it. No other dependencies are required — the Tauri webview uses the
   system's own WebView2 (Windows) or WebKitGTK (Linux, pulled in by the
   `.deb`/`.AppImage`).
3. Launch `hd2-repatcher` and follow the setup wizard to choose your
   Helldivers II install root and language. You can change these later in
   **Settings** (or pass `-g`/`--game` on the CLI).

## Usage

### GUI

Launch with no arguments (double-click, or `hd2-repatcher`). The **Home** page
lets you drop or add mod folders, `.patch_N` files, or their
`.stream`/`.gpu_resources` companions; each folder becomes a batch and each
file selects only its own group. The **Settings** page holds the Helldivers II
install root (the `data` folder is derived internally), language, and theme /
accent color.

### CLI

```console
hd2-repatcher --game "C:\Program Files (x86)\Steam\steamapps\common\Helldivers 2" C:\path\to\mods\SomeMod\mod.patch_0
```

- `-g`/`--game PATH` — path to the Helldivers II install root. The app uses its
  `data` child. It only needs to be passed once; it's cached for future runs.
  Requires at least one `PATCH_PATH`.
- `--no-game-path-caching` — don't save or overwrite the cached game install root.
- `PATCH_PATH [PATCH_PATH ...]` — patch files, `.stream`/`.gpu_resources`
  companions, or folders. A file input selects only its matching group; a
  folder input processes each group independently.

Once the game root is cached you can omit `-g`, and drag-and-drop one or more mod
folders or patch files directly onto the executable. The exit code is non-zero
if any group fails. Patching is always in place; the CLI has no output-copy or
dry-run mode.

On Windows the app is a windowed binary with no console of its own: CLI mode
only activates when it's launched from a terminal that already has a console
attached. Double-click or drag-and-drop onto the executable never spawns a
console — it always opens the GUI, with any dropped paths seeded in exactly
as if they'd been dropped onto a running window.

If you're integrating this into a mod manager, always pass the game install
root explicitly via `-g`/`--game` on every invocation rather than relying on
the cache, and patch the mod source through the manager so its file tracking
stays consistent.

### Patch groups

The main `.patch_N` file is required. A matching `<main>.stream` or
`<main>.gpu_resources` file is included automatically when present; a sidecar
selected directly resolves back to its main file. If the main TOC references a
missing required sidecar, the group is rejected before any file is changed.

Unit, audio, and mixed groups are staged and validated before replacement. The
original main filename, including its `.patch_N` suffix, is retained. Existing
`.gpu_resources` files are never regenerated, and an existing `.stream` is
replaced only when the audio patcher produces a new one. Direct-file operations
do not scan neighbouring groups; folder operations still process each
discovered group independently. No group is renamed or deleted.

## Settings

The game install root, language, theme, and accent are stored in `config.toml`
at the platform config location (`%LOCALAPPDATA%\hd2-repatcher\hd2-repatcher\`
on Windows, `~/.config/hd2-repatcher/` on Linux). This TOML configuration is
separate from the legacy Python tool's JSON settings; existing `settings.json`
files are not migrated.

Settings also includes a guarded game-data action for patching one group
directly inside the configured `data` directory. Mod-manager users should use
their manager's source patching flow instead.

## Development

### Quick start (Nix)

The dev environment is a Nix flake providing the Rust toolchain, Bun, and the
Tauri Linux system libraries:

```console
nix develop
bun install
bun run dev         # tauri dev — GUI with hot reload
```

Other scripts: `bun run build:web` (frontend only), `bun run build` (bundle the
app), `bun run build:cross-compile` (bundle the Windows NSIS installer from
Linux, via the `windows` dev shell), `bun run check:ts`, `bun run check:rs`,
`bun run lint`, `bun run check:biome` (format), `bun run check:agent` (runs
`check:ts` / `check:rs` / `check:biome` concurrently), `bun run lingui:extract`
(scan source for new/changed messages). `.po` catalogs are imported directly
in `src/lib/i18n.ts` and compiled on the fly by `@lingui/vite-plugin`, so
`bun run lingui:compile` isn't part of the normal workflow — it's only useful
to sanity-check that a catalog compiles cleanly outside of Vite.

### Other dev shells

`flake.nix` defines a few extra shells for tasks the default shell doesn't
need to pay for on every `nix develop`:

| Shell | Use it for |
| --- | --- |
| `nix develop` (default) | Everyday development: `tauri dev`, `cargo check`, tests. |
| `nix develop .#e2e` | Headless GUI runs — adds Xvfb, xdotool, and ImageMagick so `tauri dev`/the built app can be driven and screenshotted without a display. See the comments in `flake.nix` for the EGL and fontconfig gotchas this works around. |
| `nix develop .#bundle` | `cargo tauri build -b deb,appimage`. An FHS environment (`buildFHSEnv`), because the AppImage bundler shells out to hardcoded `/usr/bin` paths that plain NixOS doesn't have. |
| `nix develop .#windows` | Cross-compiling the Windows build from Linux via mingw-w64 (see [Building](#building)). |

### Developing without Nix

Nix isn't required — it just mirrors what CI installs. To set up the
equivalent toolchain by hand:

**Windows** (matches `.github/workflows/release.yml`'s `windows-latest` job):

- [Rust](https://rustup.rs) (stable).
- [Bun](https://bun.sh).
- WebView2 — already present on current Windows; Tauri will prompt to install
  it otherwise.

**Linux** (Debian/Ubuntu; matches the same workflow's `ubuntu-latest` job):

```console
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
  librsvg2-dev libayatana-appindicator3-dev patchelf
```

plus [Rust](https://rustup.rs) and [Bun](https://bun.sh). Then `bun install &&
bun run dev` as above.

### Architecture

- `crates/engine` — the native Rust patching engine (unit and audio binary /
  bundle parsing, LZ4, offset rewriting). Pure, no Tauri dependency, so it
  tests fast.
- `src-tauri` — the Tauri app: `main.rs` dispatches to CLI or GUI, `cli.rs`
  mirrors the command-line interface, `commands.rs` exposes the engine to the UI.
- `src` — the React 19 frontend (TanStack Router, Tailwind v4, Lingui, shadcn /
  Base UI style components).
- `reference` — the original Python implementations, kept as specification
  oracles for the engine port (not shipped).

## Testing

```console
cargo test -p engine    # unit tests + differential golden tests
```

The engine is a behaviour-preserving port of the Python `reference/` code. Its
correctness is pinned by **differential golden tests**: `tools/gen_golden.py`
runs the reference Python engine on synthetic patch inputs and commits the
byte-exact output to `crates/engine/tests/fixtures/`; the Rust tests replay the
same inputs and assert identical bytes. Regenerate with `python tools/gen_golden.py`.

There is no automated end-to-end test against real game files (the game data
isn't available in CI), so changes to the patching logic should be verified
manually against a mod that's actually broken by a game update:

1. Find a mod known to break after updates, e.g.
   [Invisible supply pack](https://www.nexusmods.com/helldivers2/mods/7308?tab=files),
   and download an **older** file version.
2. Install it and confirm in-game that it's broken.
3. Run it through the repatcher (GUI, drag-and-drop, or CLI) and confirm it
   reports the patch as updated.
4. Redeploy the mod and confirm it now loads correctly in-game.

## Building

CI builds Windows and Linux installers on tag push
(`.github/workflows/release.yml`). Locally:

```console
bun run build
```

To bundle a `.deb`/`.AppImage` on NixOS, use the `bundle` dev shell (see
[Other dev shells](#other-dev-shells)):

```console
nix develop .#bundle --command cargo tauri build -b deb,appimage
```

To cross-compile the Windows NSIS installer from Linux (see the `windows` dev
shell in `flake.nix`):

```console
bun run build:cross-compile
```
