{
  description = "Robin Hood: The Legend of Sherwood — Rust port development shell and package";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # `rust-toolchain.toml` pins a nightly, which nixpkgs does not package.
    # rust-overlay resolves that same pinned nightly from its official dist
    # manifest; it adds no second toolchain, only a source for the pinned one.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self, nixpkgs, rust-overlay }:
    let
      lib = nixpkgs.lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      # One package set for every output: the devShell and the package then see
      # the same nixpkgs pin and the same overlays.
      forAllSystems =
        f:
        lib.genAttrs systems (
          system:
          f (import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          })
        );

      # Single source for the compiler: `rust-toolchain.toml` is what rustup
      # honours in the devShell, and the package asks rust-overlay for the same
      # channel instead of compiling with the nixpkgs default.
      pinnedChannel = (builtins.fromTOML (builtins.readFile ./rust-toolchain.toml)).toolchain.channel;
      pinnedNightly = lib.removePrefix "nightly-" pinnedChannel;

      # The client crate's version is the one `--version` prints and the one the
      # release workflow tags; the workspace's 0.0.0 is a placeholder.
      robinVersion = (builtins.fromTOML (builtins.readFile ./crates/robin_rs/Cargo.toml)).package.version;

      # `build-support/robin_build.rs` stamps the commit into the binary so the
      # replay format and multiplayer ticket carry it. A Nix build has no
      # `.git`, so the commit is supplied from the flake's own git metadata
      # (`ROBIN_GIT_COMMIT`) and never guessed: no revision here is an error,
      # not an "unknown" stamp.
      gitHash =
        if self ? rev then
          self.rev
        else if self ? dirtyRev then
          lib.removeSuffix "-dirty" self.dirtyRev
        else
          throw ''
            packages.robin needs the source commit for build identity: build from a
            git checkout or a git flake ref so `self.rev` exists, not a plain path.'';
    in
    {
      devShells = forAllSystems (
        pkgs:
        let
          # `robin` dlopens Xlib/wayland/pulse at startup, and mkShell only
          # puts packages on PATH, so their libraries need an explicit
          # LD_LIBRARY_PATH. Without it winit dies with "Failed to load one of
          # xlib's shared libraries" the moment a window opens.
          runtimeLibs = import ./nix/runtime-libs.nix pkgs;
        in
        {
          default = pkgs.mkShell {
            packages = [
              # `build-support/robin_build.rs` shells out to `git rev-parse
              # HEAD` and falls back to the literal "unknown" when it cannot:
              # the replay format tags every recording with that hash, so a
              # shell without git silently produces replays that fail version
              # validation (27 leaderboard/replay tests, 2026-11-15).
              pkgs.git
              # rust-toolchain.toml pins the compiler, rustfmt and the wasm
              # target; rustup is what honours that pin.
              pkgs.rustup
              # `.cargo/config.toml` points the linker at
              # scripts/clang-wild-linker.sh, which links with clang+wild when
              # both exist and falls back to the (much slower) system cc when
              # they do not.
              pkgs.clang
              pkgs.wild
              pkgs.gcc
              pkgs.pkg-config
              pkgs.openssl
              pkgs.cmake
              pkgs.curl
              pkgs.python3
              pkgs.xvfb
              pkgs.ffmpeg
            ]
            ++ runtimeLibs;
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
          };
        }
      );

      packages = forAllSystems (pkgs: {
        robin = pkgs.callPackage ./nix/robin.nix {
          # The pinned nightly, not nixpkgs' stable rustc: Cargo.toml uses
          # `cargo-features` and `[unstable]`, which stable cargo rejects.
          rustPlatform = pkgs.makeRustPlatform {
            rustc = pkgs.rust-bin.nightly.${pinnedNightly}.minimal;
            cargo = pkgs.rust-bin.nightly.${pinnedNightly}.minimal;
          };
          runtimeLibs = import ./nix/runtime-libs.nix pkgs;
          inherit gitHash robinVersion;
        };
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.robin;
      });

      apps = forAllSystems (pkgs: {
        robin = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.robin}/bin/robin";
          meta.description = "Robin Hood: The Legend of Sherwood (Rust port)";
        };
        default = self.apps.${pkgs.stdenv.hostPlatform.system}.robin;
      });
    };
}
