# Playing: stable and dev builds

Which build each machine runs, where it comes from, and how a shared session
stays on one commit. Build and test commands live in `AGENTS.md` and
`TESTING.md`; this page is only about getting a build onto a machine and a seat.

## Channels

- **stable** — an annotated `vX.Y.Z` tag on `fork/main`, published as a
  non-prerelease GitHub release. This is the only build people play together.
- **dev** — a worktree build. Runs on the build-host and the laptop, never on the
  Windows seat.
- `fork/main` is the integration branch, and a nightly prerelease is a snapshot
  of it. Neither is a seat's build.

Multiplayer binds the exact engine commit and wire protocol (`MULTIPLAYER.md`),
so "the same stable build" means the same tag on every seat. `robin --version`
prints `<version>+<12-hex commit>`; compare it on every seat before a session. A
mismatch refuses the join — that comparison is the preflight, not an error to
work around. A dev build of a dirty worktree carries its HEAD commit, so it can
report the stable commit while its bytes differ: route play through the stable
profile, never a dev store path.

Cutting stable: tag `fork/main` as `vX.Y.Z`, push the tag, then start the
release on that tag with `gh workflow run native-release.yml --ref vX.Y.Z`. The
workflow treats a `v*` ref as a stable release whichever way the run starts; a
run from a branch is a nightly prerelease. CI then builds the Windows and Linux
release assets (`.github/workflows/native-release.yml`).

## Hosts

| Host | OS | stable | dev |
| --- | --- | --- | --- |
| `build-host` | NixOS (the Windows 11 side is unused) | installed in the profile | run from a worktree's store path; is the build host |
| `laptop` | NixOS | installed in the profile | a store path copied from the build-host; never compiles |
| `player-b-pc` | Windows 11, behind CGNAT | Velopack stable install | none; reached through the reverse tunnel |

### The build-host is the build host

It is often off. Wake it with `wake-host build-host` before a build; the relay, the
MAC and the WOL arming live in nixos-config (`agent/core.nix`, `wake-host`).
`scripts/remote-check.sh` wakes it itself when the host does not answer and
fails if it stays unreachable. Builds do not happen on the laptop or the
runtime.

### Install stable on NixOS

From a checkout of the stable tag, on the build-host:

    nix build .#robin

On the build-host:

    nix profile install ./result

On the laptop, which compiles nothing:

    nix copy --to ssh://laptop ./result
    ssh laptop nix profile install /nix/store/<hash>-robin

The profile is the stable channel; `robin --version` must show the tag's commit.
If the flake has no `packages.robin`, the packaging branch has not landed yet.

### Run dev on the laptop

From the worktree: `scripts/ship-dev.sh` syncs it to the build host, builds
`.#robin`, copies the closure to the laptop and prints the `nix run` line.

Never install a dev build into the profile. Dev lives in the store path of its
own commit, which is what lets stable and one dev build per worktree coexist on
the same machine.

### Two instances on one machine

Saves and the multiplayer identity key both live under `ROBINHOOD_SAVE_DIR`, or
under `dirs::data_dir()` when it is unset. A second instance started without an
override shares the first one's saves and player key, and cannot join it. Give
each instance its own root:

    ROBINHOOD_SAVE_DIR=/tmp/robin-b robin

### The Windows seat

`docs/WINDOWS.md` owns the install and build details. The stable Velopack
install updates itself within the stable channel; when an update breaks, push
the release asset with `~/git/infra/scripts/win-put`. Before a session, read
`robin --version` through `win-run` and compare it with the tag the NixOS hosts
run.

His PC is off when he is not using it and sits behind CGNAT. It dials the
`player-b-tunnel` account into the runtime (nixos-config,
`hosts/relay-vm/core.nix`), which appears there on `localhost:2222`; the
`player-b-pc` alias reaches it. A live tunnel means the PC is awake — there is no
WOL path to his LAN, so waking it is his action.
