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

        spanishTtsModel = pkgs.fetchurl {
          url = "https://huggingface.co/rhasspy/piper-voices/resolve/main/es/es_ES/sharvard/medium/es_ES-sharvard-medium.onnx";
          sha256 = "sha256-QP6/sWecaaRQX/MR3BNuEh40GaE6KQ7yZP30Pd7dD7E=";
        };
        spanishTtsConfig = pkgs.fetchurl {
          url = "https://huggingface.co/rhasspy/piper-voices/resolve/main/es/es_ES/sharvard/medium/es_ES-sharvard-medium.onnx.json";
          sha256 = "sha256-dDjJtpnHKwwziNrhto0/Nk3GaiFQ/lVKHBHwM3KVeyw=";
        };
      in {
        devShells.default = with pkgs;
          mkShell {
            name = "Assistant";
            buildInputs = [
              alsa-utils
              alsa-lib
              openssl
              # cargo-cache
              # cargo-expand
              # cargo-watch
              pkg-config
              rust-bin-custom

              # sqlitestudio
              nodejs_23
              electron

              libclang
              clang
              # stdenv.cc
              cmake
              # gcc
              # onnxruntime
              # espeak-ng
            ];
            ELECTRON_OVERRIDE_DIST_PATH = "${electron}/bin";
            SPANISH_TTS_MODEL = "${spanishTtsModel}";
            SPANISH_TTS_CONFIG = "${spanishTtsConfig}";

            LIBCLANG_PATH = "${libclang.lib}/lib";

            #             # Ensure libraries are available at runtime
            # LD_LIBRARY_PATH = "${pkgs.onnxruntime}/lib:${pkgs.alsa-lib}/lib:${pkgs.openssl}/lib:${pkgs.espeak-ng}/lib:${pkgs.llvmPackages.libclang.lib}/lib";
            # # Set LIBCLANG_PATH for bindgen
            # LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

            # # Ensure headers are found
            # C_INCLUDE_PATH = "${pkgs.espeak-ng}/include:${pkgs.stdenv.cc.libc}/include:${pkgs.llvmPackages.libclang.lib}/include";
            # # Pass include paths to bindgen and clang
            # BINDGEN_EXTRA_CLANG_ARGS = "-I${pkgs.espeak-ng}/include -I${pkgs.stdenv.cc.libc}/include -I${pkgs.llvmPackages.libclang.lib}/include";
            # Ensure clang uses the correct sysroot
            CLANG_PATH = "${clang}/bin/clang";
          };
      }
    );
}
