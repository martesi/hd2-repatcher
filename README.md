# HD2 Repatcher

Repatches Helldivers II unit mods after a game update.

When Helldivers II updates, unit mod `.patch` files can go out of sync with the
game's unit data, causing them to fail to load. This tool scans a folder of
patch files, finds the ones containing unit resources, and updates those
resources in place using the current game data.

It is a single [Tauri](https://tauri.app) app with a native Rust engine:

- **Double-click** it for a GUI — drag mod folders in and watch them repatch.
- **Run it with arguments** for a headless CLI — no window is ever created, so
  it drops into mod managers and scripts. Drag-and-drop onto the executable
  works too.

## Features

- **Batch repatching** — drop any number of mod folders at once; each becomes
  its own batch with live per-file progress.
- **GUI or headless CLI** from the same binary, so it works both as a
  point-and-click tool and as a step in a mod manager's pipeline.
- **Native Rust engine** — a behaviour-preserving, byte-exact port of the
  original Python tool, pinned by differential golden tests (see
  [Testing](#testing)).
- **Cross-platform** — prebuilt Windows and Linux installers, no runtime
  dependencies.
- **Remembers your setup** — the Helldivers II data path, theme, and accent
  color persist across runs.
- **Corruption reporting** — corrupted patch files are surfaced with a
  non-zero exit code / in-UI error instead of being silently skipped.
- **Localized UI** — English and French, via Lingui `.po` catalogs
  (`src/locales/`).

## Requirements

- A Helldivers II install (specifically its `data` folder).
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
3. Launch `hd2-repatcher` and point it at your Helldivers II `data` folder in
   **Settings** (or pass `-g`/`--game` on the CLI) the first time you use it.

## Usage

### GUI

Launch with no arguments (double-click, or `hd2-repatcher`). The **Home** page
lets you drop or add mod folders; each folder becomes a batch that repatches
immediately with live progress. The **Settings** page holds the Helldivers II
`data` folder (cached for future runs) and theme / accent color.

### CLI

```console
hd2-repatcher --game "C:\Program Files (x86)\Steam\steamapps\common\Helldivers 2\data" C:\path\to\mods\SomeMod
```

- `-g`/`--game PATH` — path to the Helldivers II `data` folder. Only needs to be
  passed once; it's cached for future runs. Requires at least one `PATCH_FOLDER`.
- `--no-game-path-caching` — don't save or overwrite the cached game data path.
- `PATCH_FOLDER [PATCH_FOLDER ...]` — one or more folders of patch files to update.

Once the game path is cached you can omit `-g`, and drag-and-drop one or more mod
folders directly onto the executable. The exit code is non-zero if any corrupted
patch files were found.

On Windows the app is a windowed binary, so in CLI mode it attaches to the
terminal it was launched from; when launched by double-click / drag-and-drop it
opens its own console and waits for a keypress so the result stays readable.

If you're integrating this into a mod manager, always pass the game data path
explicitly via `-g`/`--game` on every invocation rather than relying on the
cache.

## Settings

The game data path, theme, and accent are stored in `settings.json` at the
platform config location (`%LOCALAPPDATA%\hd2-repatcher\hd2-repatcher\` on
Windows, `~/.config/hd2-repatcher/` on Linux). The `game_data_path` key is
compatible with the earlier Python version of this tool.

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
| `nix develop .#e2e` | Headless GUI runs — adds Xvfb, xdotool, and ImageMagick so `tauri dev`/the built app can be driven and screenshotted without a display. See the `headless-gui` skill / `flake.nix` comments for the EGL and fontconfig gotchas this works around. |
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

- `crates/engine` — the native Rust patching engine (binary/bundle parsing, LZ4,
  offset rewriting). Pure, no Tauri dependency, so it tests fast.
- `src-tauri` — the Tauri app: `main.rs` dispatches to CLI or GUI, `cli.rs`
  mirrors the command-line interface, `commands.rs` exposes the engine to the UI.
- `src` — the React 19 frontend (TanStack Router, Tailwind v4, Lingui, shadcn /
  Base UI style components).
- `reference` — the original Python implementation, kept as the specification
  oracle for the engine port (not shipped).

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
