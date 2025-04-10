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
      in {
        devShells.default = with pkgs;
          mkShell {
            name = "Assistant";
            buildInputs = [
              alsa-utils
              alsa-lib
              openssl
              cargo-cache
              cargo-expand
              cargo-watch
              pkg-config
              rust-bin-custom
              sqlitestudio
            ];
          };
      }
    );
}
