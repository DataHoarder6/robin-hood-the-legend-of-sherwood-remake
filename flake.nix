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
      # (`ROBIN_PACKAGE_COMMIT`) and never guessed: no revision here is an
      # error, not an "unknown" stamp.
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

          # The runtime is not a build host. check-quality.sh alone cannot stop
          # a raw `nix develop --command cargo build`, so the devShell installs
          # a `cargo` ahead of the pinned toolchain: build-like subcommands are
          # refused on either runtime, everything else execs the real cargo.
          # Install the guard and its host-list helper as siblings: the guard
          # resolves its own path through the symlink and sources the helper
          # beside it (a single-file store path would leave no sibling).
          cargoGuard = pkgs.runCommand "robin-cargo-runtime-guard" { } ''
            mkdir -p "$out/bin" "$out/lib"
            install -m 755 "${./scripts/cargo-runtime-guard.sh}" "$out/lib/cargo-runtime-guard.sh"
            install -m 644 "${./scripts/runtime-hosts.sh}" "$out/lib/runtime-hosts.sh"
            ln -s "$out/lib/cargo-runtime-guard.sh" "$out/bin/cargo"
          '';
        in
        {
          default = pkgs.mkShell {
            packages = [
              # Must precede the rustup proxies: this `cargo` refuses runtime
              # builds; the shellHook below pins it to the front of PATH.
              cargoGuard
              # `build-support/robin_build.rs` shells out to `git rev-parse
              # HEAD` and falls back to the literal "unknown" when it cannot:
              # the replay format tags every recording with that hash, so a
              # shell without git silently produces replays that fail version
              # validation (27 leaderboard/replay tests, 2026-11-15).
              pkgs.git
              # rust-toolchain.toml pins the compiler, rustfmt and the wasm
              # target; rustup is what honours that pin.
              pkgs.rustup
              # The `client` and `client-release` suites run under nextest: one
              # process per test, one scheduler across binaries. It runs no
              # doctests, which those suites keep on `cargo test --doc`.
              pkgs.cargo-nextest
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
              # The validation drivers inject input through python-xlib, confined
              # to the local X namespace (scripts/validation/namespace_x11.py).
              # Without it the process-layer suites exit 3 on a missing
              # prerequisite instead of skipping.
              pkgs.python3Packages.python-xlib
              pkgs.xvfb
              pkgs.ffmpeg
              # The wasm gate builds the published wasm-release artifact with
              # the repo's build plan and proves its contract on the optimized
              # bytes: wasm-opt/wasm-strip come from binaryen, and wasm-objdump
              # (wabt) reads the shared-memory import the browser decode pool
              # instantiates against. Without them the check is a machine-local
              # accident rather than something `nix develop` reproduces.
              pkgs.binaryen
              pkgs.wabt
            ]
            ++ runtimeLibs;

            # Prepend rather than trust `packages` ordering: `nix develop
            # --command cargo …` must hit the guard before the rustup proxy.
            shellHook = ''
              export PATH="${cargoGuard}/bin:$PATH"
            '';
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;

            # `pkgs.gcc` above is in `packages`, so its setup hook exports
            # CC=gcc/CXX=g++ into the shell and cc-rs (which builds zstd-sys's
            # bundled C) honours CC for *every* target. A host gcc cannot
            # target wasm32: it defines __linux__, so zstd's cover.c takes its
            # _GNU_SOURCE qsort_r path, which the wasm shim's stdlib.h does not
            # declare (`cover.c:332: implicit declaration of function
            # 'qsort_r'`) — and even a compiling run would archive x86-64
            # objects into the wasm crate. The per-target variables win over
            # CC in cc-rs, so only wasm32 moves to clang, which cc-rs then
            # invokes as `clang --target=wasm32-unknown-unknown`; there
            # __linux__ is absent and zstd takes its C90 qsort fallback, which
            # the same shim maps to rust_zstd_wasm_shim_qsort. Unwrapped on
            # purpose: the nix clang wrapper injects host x86-64 flags
            # (`-fzero-call-used-regs`) that a wasm32 compile rejects.
            CC_wasm32_unknown_unknown = "${pkgs.llvmPackages.clang-unwrapped}/bin/clang";
            CXX_wasm32_unknown_unknown = "${pkgs.llvmPackages.clang-unwrapped}/bin/clang++";
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
