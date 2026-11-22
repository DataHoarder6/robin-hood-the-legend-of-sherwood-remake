# Releases

A release is a tag in this repo. Seats do not select a channel: they install
whatever commit the machine repo's `robin-hood` flake input pins. There is no
`services.robin-hood.channel` option — the machine repo's `services.robin-hood`
options are only `enable` and `browserUrl`. Do not add a channel option to the
machine repo; the pin is the whole delivery mechanism.

| Release | Source | Published as | How a seat receives it |
| --- | --- | --- | --- |
| stable | a `vX.Y.Z` tag on `fork/main` | GitHub release, Velopack packages on the `win` and `linux` package channels, update feed = that tag | set the tag's commit as `?rev=` on `inputs.robin-hood` in the machine repo's `flake.nix`, land that change through `land`, then run `just apps-sync` on each seat |
| dev | the nightly schedule in `.github/workflows/native-release.yml` | rolling prerelease tagged `nightly-<date>`, version `<base>-nightly.<timestamp>+<sha>`, same package shape | a `fork/main` commit in that same `?rev=` pin; the nightly workflow publishes a package, it does not install anything on a seat |

The daily run first compares the current commit with the last published
nightly and stops when they match, so an unchanged tree never replaces the
rolling release. `workflow_dispatch` takes an optional explicit version for a
release candidate.

The pin is always a commit, never a branch: `apps.txt` lists `robin`, and
`just apps-sync` installs it by flake reference from that input, reading the
installed store path back after the install (ADR 0026). A rebuild therefore
reproduces exactly the build the launcher names. `robin --version` prints
`<version>+<12-hex commit>`; the commit is what distinguishes two installs —
there is no channel word in the output. Nightly packages live in their own
release, so a stable install is never offered a nightly update and the reverse
holds too. To read the current pin, open `flake.nix` in the machine repo rather
than copying a commit into a doc.

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
   fetch that `summary.json` and generate the note with
   `scripts/release-notes.sh <summary.json> <version>`, which writes
   `docs/releases/<version>.md` ([0.3.0](releases/0.3.0.md) is one instance).
   Do not hand-write the note, and do not flip D06 until the gate is green.
5. Run a scriptable `robin --version` preflight before the tag: over SSH on the
   Linux seats and over `win-run` on the Windows seat, compare the 12-hex commit
   on every seat, and reject the tag unless all match. A real two-seat
   multiplayer session (`docs/PLAY.md`) is an explicit opt-out of this automated
   step — a runner skips it unless a human asks for one. The concrete
   `robin --version` / `win-run` check is in [PLAY.md](PLAY.md).
6. Record the opt-in live issue-smoke result with the release (#51). A normal
   gate never sets `ROBIN_ISSUE_SMOKE=1`: the smoke writes to the tracker, so a
   runner opts in deliberately, files one report against the real receiver and
   records the issue number, its labels and the reproduction facts/bundle link
   it contains. When the harness is not yet available, record that instead of
   claiming a pass; the contract and endpoint are in
   [BUG_REPORTS.md](BUG_REPORTS.md#provisioning-the-endpoint-operator).
7. `git tag -a vX.Y.Z -m … && git push fork vX.Y.Z`, then
   `gh workflow run native-release.yml --ref vX.Y.Z` — the tag push is verified
   by a dispatch until the push-run cause (#26) is known. Check the release's
   assets against the previous tag before announcing it.
8. Pin the tag's commit in the machine repo: change `?rev=` on the `robin-hood`
   flake input in `flake.nix` to the tag's commit, land that change through
   `land`, then run `just apps-sync` on every seat. That pin — not a channel
   option — is how the seats receive the release.

Cross-OS replay parity is covered for one class only (step 3): Wine is not
Windows, so MSVC runtime, WinDivert, Known-Folder APIs and the Windows scheduler
remain unproven until a real Windows seat runs a recorded session.
