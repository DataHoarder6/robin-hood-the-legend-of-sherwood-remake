# Nix package and cross-host install

Last verified: 2026-11-17, `nix build .#robin` and a headless mission in the packaged
binary on the build host (build-host).

`flake.nix` exposes `packages.<system>.robin` (also `default`) and
`apps.<system>.robin` for `nix run`. One host builds it; consumers receive the store
closure, so no second host compiles and no ELF interpreter, RPATH or asset path points
into the builder's working tree.

## Contents

| Path | What it is |
| --- | --- |
| `bin/robin` | wrapper; puts the dlopened X11/Wayland/ALSA/GL/Vulkan libraries on `LD_LIBRARY_PATH` |
| `libexec/robin/robin` | release binary (`--release --features desktop,multiplayer -p robin_rs --bin robin`) |
| `libexec/robin/assets` | symlink to `../../share/robin/assets` |
| `share/robin/assets/core-datadir` | validated engine core overlay, copied from this repository's `assets/core-datadir` |

The binary finds the overlay through `main_entry::init::resolve_install_resource_dir`,
which probes the working directory, then `<exe dir>/assets/core-datadir`, then
`<exe dir>/../../assets/core-datadir`. The `libexec/robin/assets` symlink makes the
second probe hit, so the game starts from any working directory; the package sets no
asset path variable. `std::env::current_exe` resolves `/proc/self/exe`, so the
`bin/robin` wrapper does not change what the binary sees as its own directory.

Retail content is not packaged. `ROBINHOOD_DATA_DIR` still names the GOG install
directory (the one containing `DATA/`), or the folder picker selects it on first start.

## Build identity

`build-support/robin_build.rs` stamps `ROBIN_GIT_HASH` with `git rev-parse HEAD`, and
`robin_replay_format` accepts only 12 lowercase hex. A Nix build has no `.git` to ask: the
source is a store copy of the tracked tree, so the build script also takes an explicit
`ROBIN_GIT_COMMIT`, which `flake.nix` fills from the revision it resolved (`self.rev`, or
`self.dirtyRev` without its `-dirty` suffix). The value is validated as a full lowercase
hex object ID and panics when malformed; the package also sets
`ROBIN_REQUIRE_BUILD_IDENTITY=1`, so a missing identity fails the build instead of
stamping `unknown`. Build from a git checkout or a `git+file:`/`git+https:` flake ref; a
plain `path:` flake resolves no revision and throws.

`robin --version` prints `robin <package version>+<12-hex commit>`
(`crate::version::CLI_VERSION`), the same prefix compact replays and join tickets carry.
The package version is `crates/robin_rs/Cargo.toml`'s, so the store path, the install
path and the binary name one build the same way.

## Building on one host, installing on another

On the builder:

    nix build .#robin
    ./result/bin/robin --version        # robin 0.1.0+<12-hex>
    nix path-info -r ./result | wc -l

Copy and install on a consumer — nothing compiles there and no `result` symlink is
needed:

    nix copy --to ssh://laptop ./result
    ssh laptop nix profile install /nix/store/<hash>-robin-0.1.0

`nix copy` writes through the consumer's daemon, which refuses paths unsigned by a key
it trusts (`require-sigs`, the NixOS default). Two ways through, both machine
configuration and neither owned by this repository: the consumer trusts the account
running the copy (`trusted-users`), or the builder signs with `nix store sign` and the
consumer lists that key in `trusted-public-keys`. Without either, the copy fails with
`cannot add path ... because it lacks a signature by a trusted key`.

A copy into a binary cache proves the closure without a second host; `compression none`
keeps it to the time it takes to read the paths, since nothing is being published:

    nix copy --no-check-sigs --option compression none --to file:///tmp/robinstore ./result
    nix path-info --store file:///tmp/robinstore -r /nix/store/<hash>-robin-0.1.0
    du -sh /tmp/robinstore

Copying a source *tree* to the build host is a different problem with a different
route: `scripts/remote-check.sh`, never `rsync`. See `PARALLEL_AGENTS.md`.

`nix run .#robin` runs the packaged binary on the builder without installing it.

## Layout of the packaging

- `flake.nix` — the single package set for both outputs. `nix/runtime-libs.nix` is the
  library list shared by the devShell's `LD_LIBRARY_PATH` and the package wrapper.
- `nix/robin.nix` — the derivation. `rustPlatform.fetchCargoVendor` vendors from
  `Cargo.lock` because the lock pins two `bitcode-0.6.9` packages (crates.io and a git
  fork) and the same for `bitcode_derive`; `importCargoLock`'s flat vendor directory
  cannot hold both.
- The toolchain is the one `rust-toolchain.toml` pins. `rust-overlay` is a flake input
  only because nixpkgs packages stable Rust, not that nightly; it resolves the same
  `nightly-2026-10-21` the devShell gets through rustup, so there is one toolchain in the
  repository and one source for it.
