{
  description = "HD2 Repatcher — Tauri app dev shell (Rust engine + React frontend)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    bun2nix = {
      url = "github:nix-community/bun2nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, bun2nix }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        packageRustToolchain = pkgs.rust-bin.stable.latest.default;
        rustToolchain = packageRustToolchain.override {
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

        devPackages = [
          rustToolchain
          pkgs.cargo-tauri
          pkgs.bun
          pkgs.nodejs_24
          pkgs.python3 # dev-only: regenerating golden fixtures via reference engine
        ] ++ buildTools ++ tauriLibs;

        # Tauri's webkitgtk needs these at runtime; the compositing workaround
        # avoids the known blank-window issue on NixOS/Wayland.
        #
        # __EGL_VENDOR_LIBRARY_DIRS is what makes headless (Xvfb) runs work.
        # webkitgtk resolves libEGL through its own RPATH, but libglvnd looks
        # for vendor ICD json in /usr/share/glvnd/egl_vendor.d, which does not
        # exist on NixOS. With no ICD, the webview aborts on startup with
        # "Could not create default EGL display: EGL_BAD_PARAMETER" and paints
        # a blank white window - it looks like an app bug, not a missing
        # library, so it costs a while to track down. Pointing glvnd at mesa
        # gets a software (llvmpipe) context. The :- guard leaves a real
        # desktop alone: NixOS already exports this to /run/opengl-driver/...,
        # and clobbering it would drop hardware accel for e.g. nvidia users.
        #
        # Shared by `default` and `e2e` rather than copy-pasted: the two must
        # not drift, and a headless run missing the EGL line fails in a way
        # that looks nothing like a missing environment variable.
        tauriHook = ''
          export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath tauriLibs}:$LD_LIBRARY_PATH"
          export __EGL_VENDOR_LIBRARY_DIRS="''${__EGL_VENDOR_LIBRARY_DIRS:-${pkgs.mesa}/share/glvnd/egl_vendor.d}"
          export XDG_DATA_DIRS="${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}:${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:$XDG_DATA_DIRS"
          export GIO_MODULE_DIR="${pkgs.glib-networking}/lib/gio/modules/"
          export WEBKIT_DISABLE_COMPOSITING_MODE=1
        '';

        winCrossToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" ];
          targets = [ "x86_64-pc-windows-gnu" ];
        };
        mingw = pkgs.pkgsCross.mingwW64;

        bun2nixPkg = bun2nix.packages.${system}.default;
        appVersion = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

        hd2-repatcher = pkgs.stdenv.mkDerivation {
          pname = "hd2-repatcher";
          version = appVersion;
          src = ./.;

          cargoDeps = pkgs.rustPlatform.importCargoLock {
            lockFile = ./Cargo.lock;
          };
          bunDeps = bun2nixPkg.fetchBunDeps {
            bunNix = ./bun.nix;
          };

          nativeBuildInputs = [
            packageRustToolchain
            pkgs.rustPlatform.cargoSetupHook
            bun2nixPkg.hook
            pkgs.bun
            pkgs.pkg-config
            pkgs.wrapGAppsHook3
          ];
          buildInputs = tauriLibs;

          buildPhase = ''
            runHook preBuild
            bun run build:web
            cargo build --release --locked -p hd2-repatcher
            runHook postBuild
          '';

          installPhase = ''
            runHook preInstall
            install -Dm755 target/release/hd2-repatcher $out/bin/hd2-repatcher
            runHook postInstall
          '';

          preFixup = ''
            gappsWrapperArgs+=(
              --set-default WEBKIT_DISABLE_COMPOSITING_MODE 1
              --set-default __EGL_VENDOR_LIBRARY_DIRS ${pkgs.mesa}/share/glvnd/egl_vendor.d
              --set-default GIO_MODULE_DIR ${pkgs.glib-networking}/lib/gio/modules/
            )
          '';

          meta = {
            description = "Repatches Helldivers II unit and audio mods after a game update";
            license = pkgs.lib.licenses.mit;
            mainProgram = "hd2-repatcher";
            platforms = pkgs.lib.platforms.linux;
          };
        };
      in
      {
        packages = pkgs.lib.optionalAttrs (system == "x86_64-linux") {
          default = hd2-repatcher;
          inherit hd2-repatcher;
        };

        apps = pkgs.lib.optionalAttrs (system == "x86_64-linux") {
          default = flake-utils.lib.mkApp { drv = hd2-repatcher; };
        };

        devShells.default = pkgs.mkShell {
          packages = devPackages;
          shellHook = tauriHook;
        };

        # Driving the GUI headlessly (Xvfb + screenshots + synthetic input).
        # Deliberately a sibling shell rather than extra packages on `default`:
        # this closure is dead weight for everyday development, and only
        # end-to-end runs should pay for it. Declaring it here rather than
        # relying on preinstalled tools keeps the capability reproducible for
        # anyone who clones the repo.
        devShells.e2e = pkgs.mkShell {
          packages = devPackages ++ [
            pkgs.xvfb # the X server itself
            pkgs.xdpyinfo # readiness check - Xvfb takes a moment to accept clients
            pkgs.xdotool # synthetic mouse/keyboard input
            pkgs.imagemagick # `import -window root shot.png`
          ];

          # DISPLAY is preset so an app launched after Xvfb comes up needs no
          # per-command wrapping. Until you start a server it points at a
          # display that doesn't exist, which fails exactly like an unset
          # DISPLAY - so start Xvfb and wait for it first. The :- guard keeps
          # a real desktop's display, or one you picked yourself.
          #
          # FONTCONFIG_FILE matters more than it looks. A slim container has no
          # /etc/fonts and no font files, so fontconfig finds nothing and the
          # webview renders every glyph as tofu - screenshots come out
          # structurally correct and completely unreadable, which is easy to
          # misread as a rendering bug. makeFontsConf writes a self-contained
          # config naming these store paths; the env var is required because
          # fontconfig's compiled-in default (/etc/fonts/fonts.conf) is absent.
          # DejaVu + Liberation cover the app's locales (en, fr); add noto-fonts
          # variants here if it ever ships non-Latin catalogs.
          shellHook = tauriHook + ''
            export DISPLAY="''${DISPLAY:-:99}"
            export FONTCONFIG_FILE="''${FONTCONFIG_FILE:-${
              pkgs.makeFontsConf {
                fontDirectories = with pkgs; [ dejavu_fonts liberation_ttf ];
              }
            }}"
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
