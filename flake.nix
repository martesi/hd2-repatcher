{
  description = "HD2 Repatcher — Tauri app dev shell (Rust engine + React frontend)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" "rustfmt" ];
        };

        # Native libraries Tauri's webview needs at build + run time on Linux.
        tauriLibs = with pkgs; [
          glib
          gtk3
          cairo
          gdk-pixbuf
          pango
          harfbuzz
          librsvg
          libsoup_3
          webkitgtk_4_1
          openssl
          dbus
        ];

        buildTools = with pkgs; [
          pkg-config
          wrapGAppsHook3
        ];

        winCrossToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" ];
          targets = [ "x86_64-pc-windows-gnu" ];
        };
        mingw = pkgs.pkgsCross.mingwW64;
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            rustToolchain
            pkgs.cargo-tauri
            pkgs.bun
            pkgs.nodejs_24
            pkgs.python3 # dev-only: regenerating golden fixtures via reference engine
          ] ++ buildTools ++ tauriLibs;

          # Tauri's webkitgtk needs these at runtime; the compositing workaround
          # avoids the known blank-window issue on NixOS/Wayland.
          shellHook = ''
            export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath tauriLibs}:$LD_LIBRARY_PATH"
            export XDG_DATA_DIRS="${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}:${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:$XDG_DATA_DIRS"
            export GIO_MODULE_DIR="${pkgs.glib-networking}/lib/gio/modules/"
            export WEBKIT_DISABLE_COMPOSITING_MODE=1
          '';
        };

        # `cargo tauri build -b deb,appimage` needs its own shell: the AppImage
        # bundler (linuxdeploy, downloaded on demand by tauri-bundler) shells
        # out to hardcoded FHS paths like `/usr/bin/xdg-open`, which plain
        # NixOS doesn't have. buildFHSEnv sandboxes a real-looking /usr for
        # this to work. `deb` bundling needs no external tools (tauri-bundler
        # writes the .deb itself) and works fine here too.
        devShells.bundle = (pkgs.buildFHSEnv {
          name = "hd2-repatcher-bundle";
          targetPkgs = pkgs: [
            rustToolchain
            pkgs.cargo-tauri
            pkgs.bun
            pkgs.nodejs_24
            pkgs.xdg-utils # linuxdeploy's AppImage bundling shells out to xdg-open
          ] ++ buildTools ++ tauriLibs;
          profile = ''
            export WEBKIT_DISABLE_COMPOSITING_MODE=1
          '';
        }).env;

        # Cross-compiles + bundles the Windows NSIS installer from Linux, via
        # mingw-w64 (not MSVC/xwin) - confirmed webview2-com/tao/wry all link
        # fine against it once winpthreads' static lib is on the search path
        # (nixpkgs' mingw-w64 gcc doesn't ship it by default). `makensis`
        # itself is a portable Linux-native binary, no Wine needed.
        devShells.windows = pkgs.mkShell {
          packages = [
            winCrossToolchain
            mingw.stdenv.cc
            pkgs.cargo-tauri
            pkgs.bun
            pkgs.nodejs_24
            pkgs.nsis
          ];

          shellHook = ''
            export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=${mingw.stdenv.cc.targetPrefix}cc
            export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-L ${mingw.windows.pthreads}/lib"
          '';
        };
      });
}
