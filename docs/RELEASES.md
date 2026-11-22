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
4. Build the release binary on the build host, then run
   `bash scripts/release-gate.sh <candidate> <release-binary-on-host>` from this
   checkout (the script ssh-es to `${ROBIN_BUILD_HOST:-minipc}` itself). It is
   release-time only — `push-gate.sh` never selects `multiplayer-e2e`, so this is
   the only enforcement of D06's H23 half. The gate checks the candidate against
   HEAD and the binary's `--version`, runs `check-quality.sh multiplayer-e2e` at
   `--jobs 2` with no `--only`, and refuses the tag unless the suite exits 0 and
   `summary.json` reports `.verdict == "observed"`, `.verdicts.H23 == "observed"`
   and a `.selected` covering every process row. It prints the evidence path;
   copy that path and the H23 verdict into
   [releases/0.3.0.md](releases/0.3.0.md). Do not flip D06 until the gate is green.
5. Run a scriptable `robin --version` preflight before the tag: over SSH on the
   Linux seats and over `win-run` on the Windows seat, compare the 12-hex commit
   on every seat, and reject the tag unless all match. A real two-seat
   multiplayer session (`docs/PLAY.md`) is an explicit opt-out of this automated
   step — a runner skips it unless a human asks for one. The concrete
   `robin --version` / `win-run` check is in [PLAY.md](PLAY.md).
6. `git tag -a vX.Y.Z -m … && git push fork vX.Y.Z`, then
   `gh workflow run native-release.yml --ref vX.Y.Z` — the tag push is verified
   by a dispatch until the push-run cause (#26) is known. Check the release's
   assets against the previous tag before announcing it.
7. Pin the tag's commit in the machine repo (`services.robin-hood.channel =
   "stable"`), which is how the seats receive it.

Cross-OS replay parity is covered for one class only (step 3): Wine is not
Windows, so MSVC runtime, WinDivert, Known-Folder APIs and the Windows scheduler
remain unproven until a real Windows seat runs a recorded session.


