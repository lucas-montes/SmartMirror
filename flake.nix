{
  description = "Assistant";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
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

        # Supertonic 3 (sherpa-onnx export): one multilingual model, Croatian
        # selected at runtime with lang = "hr". License: OpenRAIL.
        ttsModelDir = pkgs.fetchzip {
          url = "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/sherpa-onnx-supertonic-3-tts-int8-2026-05-11.tar.bz2";
          hash = "sha256-UNN4akgwwdqLdC/T8avL/JGDhLyZ1Y57kqgzfxeayPA=";
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
        #     export TTS_MODEL_DIR="${ttsModelDir}"
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
            TTS_MODEL_DIR = ttsModelDir;
            TTS_LANG = "hr";

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
