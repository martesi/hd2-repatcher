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
      });
}
