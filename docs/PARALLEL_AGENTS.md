# Working in parallel: symptoms, rules, commands

Last verified 2026-11-17 against `5a15890c3` on the runtime's checkout; the
sizes and commit counts below are that day's measurements.

Every trap here has cost a session real work. Each section starts with what you
see, then the rule that avoids it and the command that applies the rule.
`just doctor` checks all of them mechanically.

## "I committed on main and now main is divergent"

Rule: the shared checkout's `main` is a **read mirror** of `fork/main`; work
happens in a worktree. Never commit, merge, reset or switch branches on the
shared `main`, and leave uncommitted files there alone — they belong to whoever
is running in that checkout. `fork/main` is the integration branch; a worktree
is `git worktree add .worktrees/<name> -b <name> fork/main`.

```sh
git worktree add .worktrees/my-change -b my-change fork/main
```

`claude-worktree <name>` does the same and launches an agent in it. It forks
`main`, so run `just doctor` in the shared checkout first: a drifted or dirty
`main` would fork the drift.

## "A merged worktree is still 400 MB"

Symptom: `.worktrees/` holds every branch ever opened, the disk fills with
trees whose work already landed, and `git branch` stops telling work in
progress from history.

Rule: a worktree is retired when its branch is merged into the integration
ref, its tree is clean, it is not detached, no build runs inside it, and it is
neither the shared checkout nor the one you are running from. Everything else
is a decision — land it or abandon it — and the plan names which.

```sh
just prune-worktrees            # plan: remove/keep, with a reason per worktree
just prune-worktrees --apply    # remove the planned trees and their branches
```

## "git cherry says those commits are not upstream, but they are"

Rule: `git cherry -v fork/main main` prefixes a commit with `-` when its patch
is **already upstream** under another hash, and `+` when it is genuinely unique.
A `-` commit must not be re-landed: the change already exists, so re-landing it
duplicates it. Unique commits are the only reason to stop.

```sh
git cherry -v fork/main main
```

Recovery from a stale `main`: archive the old tip on a branch (it records what
was tested), then move `main` forward. `just reconcile-main` does exactly that
and refuses a dirty tree, a build in progress, or any unique commit.

```sh
just reconcile-main
```

By hand: `git branch archive/main-$(date +%F) main && git reset --hard fork/main`.

## "InvalidVersionHash in the replay and leaderboard tests on the build host"

Symptom: 27 replay/leaderboard tests fail with `InvalidVersionHash`, and the
binary reports `ROBIN_GIT_HASH=unknown`.

Cause: a worktree's `.git` is a **file** naming
`<main-checkout>/.git/worktrees/<name>`, a path that exists only on the sender.
`rsync` copies the file, the copy has no usable repository, and the build script
stamps `unknown`.

Rule: never `rsync` a worktree to the build host. `scripts/remote-check.sh`
creates the copy directory, excludes `.git`, initialises a real repository
there, pushes this checkout's objects into it, points its `HEAD` at them and
refreshes the index before running the suite under the per-copy lock.

```sh
scripts/remote-check.sh engine
scripts/remote-check.sh --list     # copies and sizes on the build host
scripts/remote-check.sh --prune    # drop every copy but this one
```

## "Connection refused when joining after an update"

Rule: multiplayer binds the engine commit and the wire protocol into the join.
Both seats must run the **same commit**, not "the latest build". One side
updated and the other not is a refused join, not a bug.

```sh
robin --version                    # robin <version>+<12-hex commit>
```

Compare it on every seat before a session. Building, tagging and installing the
shared build is `docs/PLAY.md`'s job; this page only fixes the invariant.

## "The Windows exe exits instantly (0xc0000005)"

Rule: Windows is built **natively with MSVC on a Windows runner**, dispatched
from CI. Never cross-build `windows-gnu`: MinGW `ld` writes TLS `SECREL32`
offsets as `HIGHLOW` relocations inside `.text`, and the exe faults before
`main`. Install the CI artifact instead of compiling.

```sh
gh run download --repo DataHoarder6/robin-hood-the-legend-of-sherwood-remake \
  --name robin-windows-x86_64-exe -D .
```

`docs/WINDOWS.md` owns the toolchain, the ATL component, the crash analysis and
the local MSVC build steps.

## "A copied Linux binary will not start on another host"

Rule: the binary is not portable; its **store closure** is. Build on one host
and copy the closure, never the file. `nix copy --to ssh://host` writes through
the receiver's daemon, which rejects paths unsigned by a trusted key — trust the
copying account (`trusted-users`) or sign with `nix store sign` and list the key.

```sh
nix build .#robin
./result/bin/robin --version       # the commit the artifact carries
nix copy --to ssh://laptop ./result
ssh laptop nix profile install /nix/store/<hash>-robin
```

`docs/NIX.md` owns the package layout, the build identity and the signing
detail.

## "nix-collect-garbage -d before the upgrade deleted my way back"

Rule: never run `nix-collect-garbage -d` on a host that is waiting for, or in
the middle of, an OS upgrade. `-d` deletes every old generation, and the old
generations **are** the rollback path. Delete by age instead; `-d` is only
acceptable after the new system has booted and been verified.

```sh
nix-collect-garbage --delete-older-than 14d
```

## "target/ is 14 GiB and the disk is full"

Measured 2026-11-17 on the runtime: `target/` 13.1 GiB, of which
`target/debug/incremental` is 8.4 GiB. One checkout plus worktrees is the right
shape; a second full source copy made to get a parallel build is not.

Rule: prune `target/debug/incremental` before deleting a whole `target`, and
delete a whole `target` only when nothing cheaper is left. Each checkout owns
its `target/`; `CARGO_TARGET_DIR` is never redirected.

Levers, cheapest first:

```sh
rm -rf target/debug/incremental          # in every checkout holding one
scripts/remote-check.sh --prune-stale 7  # copies nothing has written to for a week
nix-collect-garbage --delete-older-than 14d
rm -rf target                            # last: a full rebuild follows
```

`just doctor` reports both sizes and fails when free disk drops below the floor
(default 15 GiB, `DOCTOR_MIN_FREE_GIB`).

## "The format gate says rustfmt is missing, so I used --no-verify"

Rule: rustfmt **is** available — it ships with the toolchain `rust-toolchain.toml`
pins — and `cargo fmt --all -- --check` is a real gate. Believing otherwise led
to `--no-verify` commits that failed CI later. The pre-commit hook formats only
the `.rs` files the commit touches; skipping it hides a whole-tree failure
instead of avoiding it. Never commit with `--no-verify`.

```sh
bash scripts/check-quality.sh format   # cargo fmt --all -- --check
```

## The checker

`just doctor` (or `scripts/doctor.sh`) is read-only and prints a report with a
`fix:` line per finding. It exits non-zero only for states that bite:

| Check | Fails when |
| --- | --- |
| `main-dirty` | `main` is checked out with uncommitted paths |
| `main-drift` | `main` differs from the integration ref (reports behind count, and how many of its commits are already upstream) |
| `build-running` | a compiler or build driver has a working directory inside this checkout |
| `archive-unique` | an `archive/*` branch holds commits the integration ref does not have |
| `detached-unique` | a detached `HEAD` holds commits no branch or remote contains |
| `disk-low` | free space is below the floor |
| `target-size`, `archive-duplicate`, `worktree-stale`, `upstream-drift`, `no-integration-ref` | reported, never fatal |

`worktree-stale` is the one finding with a command of its own: `just
prune-worktrees` acts on it, `doctor` only reports it. `upstream-drift` counts
what `fork/main` is behind `origin/main` — the fork is worth rebasing only
while its own delta is small, and the note is where that stops being a
surprise.

`--quiet` prints only failures, for a CI or manual gate. The repo's
`.githooks/pre-push` does not call it on purpose: a dirty shared checkout is a
local work state, not a reason to refuse someone else's push. Without
`fork/main`, `origin/main` or a branch upstream the drift check is reported as
skipped rather than failing.

Recipes: `just doctor`, `just reconcile-main`, `just check SUITE`
(`scripts/check-quality.sh` suites; `tooling` is the fixture-free one).
