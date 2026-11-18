# Releases and channels

Two channels, one tag each. A branch is never a channel.

| Channel | Source | Published as | Nix pin |
| --- | --- | --- | --- |
| `stable` | a `v*` tag | GitHub release, Velopack packages on the `win` and `linux` package channels, update feed = that tag | the tag's commit, `services.robin-hood.channel = "stable"` |
| `dev` | the nightly schedule in `.github/workflows/native-release.yml` | rolling prerelease tagged `nightly-<date>`, version `<base>-nightly.<timestamp>+<sha>`, same package shape | a `fork/main` commit, `services.robin-hood.channel = "dev"` |

The daily run first compares the current commit with the last published
nightly and stops when they match, so an unchanged tree never replaces the
rolling release. `workflow_dispatch` takes an optional explicit version for a
release candidate.

The nix pin is always a commit, never a branch: a rebuild reproduces exactly the
build the launcher names, and the launcher prints the channel word beside the
short commit so two installs can be told apart. Nightly packages live in their
own release, so a stable install is never offered a nightly update and the
reverse holds too.

## Before a stable tag

The push gate proves the tree compiles and its suites pass; it cannot prove a
session. A stable tag therefore needs the evidence below, in this order, and a
tag pushed without it is a bet rather than a release.

1. `git fetch fork && git log --oneline <previous tag>..fork/main` — what ships,
   and which issues it closes.
2. `bash scripts/remote-check.sh --suites format client-release engine tooling`
   on the build host: one synced tree, all suites, one verdict. Paste the counts
   into the release notes. The scenario corpus validator runs in `tooling`.
3. `bash scripts/remote-check.sh parity-cross` on the same tree: the Windows
   asset replayed under Wine against the native build must produce an identical
   hash chain. It proves the toolchain/FP/locale class; a real Windows seat is
   still the only proof of the OS-layer class (`docs/TESTING.md`).
4. Build the release binary on the build host, then
   `ROBIN_E2E_BINARY=<binary> ROBINHOOD_DATA_DIR=<game root> bash
   scripts/check-quality.sh multiplayer-e2e`; the suite must print `observed`
   with exit 0 and leave its evidence directory.
5. Install the same tag on two seats and run `robin --version` on both
   (`docs/PLAY.md`); a multiplayer session is joined only by seats naming the
   same commit. This is the step a runner cannot do.
6. `git tag -a vX.Y.Z -m … && git push fork vX.Y.Z`, then
   `gh workflow run native-release.yml --ref vX.Y.Z` — the tag push is verified
   by a dispatch until the push-run cause (#26) is known. Check the release's
   assets against the previous tag before announcing it.
7. Pin the tag's commit in the machine repo (`services.robin-hood.channel =
   "stable"`), which is how the seats receive it.

Cross-OS replay parity is covered for one class only (step 3): Wine is not
Windows, so MSVC runtime, WinDivert, Known-Folder APIs and the Windows scheduler
remain unproven until a real Windows seat runs a recorded session.


