{
  description = "Assistant";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    nixpkgs,
    rust-overlay,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [(import rust-overlay)];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rust-bin-custom = pkgs.rust-bin.stable.latest.default.override {
          extensions = ["rust-src"];
        };

        manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
        rustApp = pkgs.rustPlatform.buildRustPackage {
          pname = manifest.name;
          version = manifest.version;
          cargoLock.lockFile = ./Cargo.lock;
          src = pkgs.lib.cleanSource ./.;

          nativeBuildInputs = [pkgs.pkg-config];
          buildInputs = [pkgs.openssl pkgs.libclang pkgs.clang pkgs.cmake];

          # your voskLib, RUSTFLAGS, etc.
          RUSTFLAGS = "-L${voskLib}";
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [pkgs.stdenv.cc.cc voskLib];
        };

        voskVersion = "0.3.45";
        arch = builtins.elemAt (nixpkgs.lib.strings.splitString "-" pkgs.system) 0;

        voskLib = pkgs.fetchzip {
          url = "https://github.com/alphacep/vosk-api/releases/download/v${voskVersion}/vosk-linux-${arch}-${voskVersion}.zip";
          hash =
            {
              "x86_64" = "sha256-ToMDbD5ooFMHU0nNlfpLynF29kkfMknBluKO5PipLFY=";
              "aarch64" = "sha256-Pve91VpfVAXMY3dqbE9U1gkfQtedHOXj4ikGlWKyezQ=";
            }
            .${arch};
        };

        model = pkgs.fetchzip {
          url = "https://alphacephei.com/vosk/models/vosk-model-small-es-0.42.zip";
          hash = "sha256-b79NJOACK8pT+ZQEL+rI5MfqkYfMUGrU2Hj3mGyUNIA=";
        };

        spanishTtsModel = pkgs.fetchurl {
          url = "https://huggingface.co/rhasspy/piper-voices/resolve/main/es/es_ES/sharvard/medium/es_ES-sharvard-medium.onnx";
          sha256 = "sha256-QP6/sWecaaRQX/MR3BNuEh40GaE6KQ7yZP30Pd7dD7E=";
        };
        spanishTtsConfig = pkgs.fetchurl {
          url = "https://huggingface.co/rhasspy/piper-voices/resolve/main/es/es_ES/sharvard/medium/es_ES-sharvard-medium.onnx.json";
          sha256 = "sha256-dDjJtpnHKwwziNrhto0/Nk3GaiFQ/lVKHBHwM3KVeyw=";
        };
      in {
        packages.default = rustApp;

        # devShells.default = pkgs.mkShell {
        #   name = "assistant-dev";

        #   # 2. just pull in your package:
        #   buildInputs = [
        #     myPackage
        #     pkgs.rust-bin.stable.latest.default.override { extensions = [ "rust-src" ]; }
        #     pkgs.alsa-utils
        #   ];

        #   # 3. and any extra tooling you need
        #   shellHook = ''
        #     # you still need model paths, etc.
        #     export MODEL="${model}"
        #     export SPANISH_TTS_MODEL="${spanishTtsModel}"
        #     export SPANISH_TTS_CONFIG="${spanishTtsConfig}"
        #   '';
        # };

        devShells.default = with pkgs;
          mkShell {
            name = "Assistant";
            buildInputs = [
              alsa-utils
              alsa-lib
              #alsa-plugins
              openssl
              pkg-config
              rust-bin-custom

              # nodejs_23
              # electron

              libclang
              clang
              cmake
            ];
            #            ELECTRON_OVERRIDE_DIST_PATH = "${electron}/bin";
            SPANISH_TTS_MODEL = spanishTtsModel;
            SPANISH_TTS_CONFIG = spanishTtsConfig;

            RUSTFLAGS = "-L${voskLib}";
            LD_LIBRARY_PATH = lib.makeLibraryPath [
              pkgs.stdenv.cc.cc
              voskLib
              #              pkgs.alsa-lib
            ];

            MODEL = model;
            LIBCLANG_PATH = "${libclang.lib}/lib";
            CLANG_PATH = "${clang}/bin/clang";
          };
      }
    );
}
