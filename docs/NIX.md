# Nix package and cross-host install

Last verified: 2026-11-16, `nix build .#robin` on the build host (minipc).

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
second probe hit, so the game starts from any working directory; no asset environment
variable is set or read. `std::env::current_exe` resolves `/proc/self/exe`, so the
`bin/robin` wrapper does not change what the binary sees as its own directory.

Retail content is not packaged. `ROBINHOOD_DATA_DIR` still names the GOG install
directory (the one containing `DATA/`), or the folder picker selects it on first start.

## Build identity

`build-support/robin_build.rs` stamps `ROBIN_GIT_HASH` with `git rev-parse HEAD`, and
`robin_replay_format` accepts only 12 lowercase hex. A Nix build has no `.git`, so
`nix/robin.nix` writes a stub ref holding the flake's commit (`self.rev`, or
`self.dirtyRev` minus its `-dirty` suffix) and builds with
`ROBIN_REQUIRE_BUILD_IDENTITY=1`: a missing identity fails the build instead of stamping
`unknown`. Build from a git checkout or a `git+file:`/`git+https:` flake ref; a plain
`path:` flake has no revision and throws.

`robin --version` prints `<package version>+<12-hex commit>`
(`crate::version::CLI_VERSION`), the same prefix compact replays and join tickets carry.

## Building on one host, installing on another

On the builder:

    nix build .#robin
    nix path-info -r ./result | wc -l

Copy and install on a consumer — nothing compiles there and no `result` symlink is
needed:

    nix copy --to ssh://laptop ./result
    ssh laptop nix profile install /nix/store/<hash>-robin-0.0.0

`nix copy` writes through the consumer's daemon, which refuses paths unsigned by a key
it trusts (`require-sigs`, the NixOS default). Two ways through, both machine
configuration and neither owned by this repository: the consumer trusts the account
running the copy (`trusted-users`), or the builder signs with `nix store sign` and the
consumer lists that key in `trusted-public-keys`. Without either, the copy fails with
`cannot add path ... because it lacks a signature by a trusted key`.

A copy into a local store directory proves the closure without a second host:

    nix copy --no-check-sigs --to file:///tmp/robinstore ./result
    nix store verify --store file:///tmp/robinstore --no-trust /nix/store/<hash>-robin-0.0.0

`nix run .#robin` runs the packaged binary on the builder without installing it.

## Layout of the packaging

- `flake.nix` — the single package set for both outputs. `nix/runtime-libs.nix` is the
  library list shared by the devShell's `LD_LIBRARY_PATH` and the package wrapper.
- `nix/robin.nix` — the derivation. `rustPlatform.fetchCargoVendor` vendors from
  `Cargo.lock` because the lock pins two `bitcode-0.6.9` packages (crates.io and a git
  fork) and the same for `bitcode_derive`; `importCargoLock`'s flat vendor directory
  cannot hold both.
- The toolchain is the one `rust-toolchain.toml` pins. `rust-overlay` is a flake input
  only because nixpkgs packages stable Rust, not that nightly.
