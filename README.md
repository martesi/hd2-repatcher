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

## Requirements

- A Helldivers II install (specifically its `data` folder).
- Windows or Linux. Prebuilt installers are attached to each
  [Release](../../releases); no runtime dependencies.

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

The dev environment is a Nix flake providing the Rust toolchain, Bun, and the
Tauri Linux system libraries:

```console
nix develop
bun install
bun run app        # tauri dev — GUI with hot reload
```

Other scripts: `bun run build` (frontend), `bun run app:build` (bundle the app),
`bun run typecheck`, `bun run lint`, `bun run lingui:extract` / `lingui:compile`.

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
bun run app:build
```
