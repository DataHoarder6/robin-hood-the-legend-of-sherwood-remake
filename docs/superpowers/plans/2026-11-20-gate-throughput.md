# Gate Throughput Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** make one landing gate cheap and shareable across parallel worktrees without weakening what the gate proves.

**Architecture:** keep per-copy `target/` dirs and the existing one-copy-one-lock rule; share rustc results with sccache across all copy roots; key gate copies by the pushed tree; wrap the build in a bounded transient scope; bound the slot admission queue.

**Tech Stack:** Bash, `flock`, `systemd-run`, sccache, NixOS/home-manager, `nix flake check`.

**Spec:** `docs/superpowers/specs/2026-11-20-gate-throughput-design.md` and the evidence doc it points at, `docs/slop/BUILD_CACHE_AND_QUEUE.md`.

## Global Constraints

- Never set `CARGO_TARGET_DIR`, `--target-dir`, or a Cargo config that moves `target`.
- One lock class at a time: never hold the admission lock and the copy lock together.
- The gate proves the exact tree the push lands. No suite is removed from `push-gate.sh`.
- `scripts/push-gate.sh` stays the single definition of which suites a landing owes.
- Copy names used by `live-worktrees.sh` and `--prune` remain worktree basenames; gate copies are a separate namespace with a retention cap.
- Host configuration is declarative in `~/nixos-config`, not hand-edited on minipc.

## Review Focus

- `SCCACHE_BASEDIRS` with one copy root — must list every root; test: `sccache-basedirs.sh` fixture with three copy dirs.
- A killed harness leaving a build running — must be bounded by a transient scope; test: scope wrapper present in `remote-check.sh` plus a smoke run whose unit disappears.
- Two agents pushing the same tree — must share one copy; test: `gate-copy-name.sh` stability fixture.
- Gate copies accumulating forever — must be pruned by TTL; test: `--prune` fixture with an old and a fresh gate copy.
- A waiter queued forever behind a dead holder — must exit nonzero at the bound; test: `build-slot.sh` timeout fixture.

---

### Task 1: shared sccache namespace

**Files:**
- Create: `scripts/sccache-basedirs.sh`
- Modify: `scripts/remote-check.sh:264-283`
- Test: `scripts/test_build_host.sh`

**Interfaces:**
- Produces: `sccache-basedirs.sh` — no args, prints colon-separated absolute paths of every existing copy root under `~/build/robin`; consumed by both remote command heredocs.
- Consumes: nothing.

- [ ] **Step 1: add fixture tests to `scripts/test_build_host.sh`**

  In a temp `HOME`, create `build/robin/a`, `build/robin/b` and a file `build/robin/not-a-copy`. Assert `bash "$source_root/scripts/sccache-basedirs.sh"` prints exactly the two dirs joined by `:` and nothing else. Assert `grep -F 'SCCACHE_BASEDIRS=\$(pwd)' "$source_root/scripts/remote-check.sh"` finds no match. Assert both remote command heredocs contain `CARGO_INCREMENTAL=0`.

- [ ] **Step 2: run the fixture to verify it fails**

  Run: `bash scripts/test_build_host.sh`
  Expected: FAIL on the missing helper and the still-present per-invocation basedir.

- [ ] **Step 3: implement the helper and replace the override**

  Create `scripts/sccache-basedirs.sh`: glob `~/build/robin/*/`, keep only directories, join with `:`. Replace both `env SCCACHE_BASEDIRS=\$(pwd)` heredoc lines in `remote-check.sh` with `env SCCACHE_BASEDIRS=\"\$(bash scripts/sccache-basedirs.sh)\" CARGO_INCREMENTAL=0`. Keep the existing `flock` and `build-slot.sh` calls unchanged in this task.

- [ ] **Step 4: run the fixture to verify it passes**

  Run: `bash scripts/test_build_host.sh`
  Expected: PASS, including the four pre-existing slot/live-worktree fixtures.

- [ ] **Step 5: commit**

  `git commit -- scripts/sccache-basedirs.sh scripts/remote-check.sh scripts/test_build_host.sh` with message `build: share sccache across all build-host copies`.

### Task 2: declarative host cache configuration

**Files:**
- Modify: `~/nixos-config/modules/system/build-host.nix`
- Test: `~/nixos-config/tests/build-host.nix`

**Interfaces:**
- Consumes: `services.build-host.cacheSize` (existing option).
- Produces: generated `~/.cargo/config.toml` with `SCCACHE_CACHE_SIZE`, `SCCACHE_IDLE_TIMEOUT=0`, `SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS=CARGO_MANIFEST_DIR,CARGO_MANIFEST_PATH`, and a joined `--remap-path-prefix` rustflag for the build host; host config values from `docs/slop/BUILD_CACHE_AND_QUEUE.md` §1.

- [ ] **Step 1: add assertions to `~/nixos-config/tests/build-host.nix`**

  Assert the generated Cargo config contains `SCCACHE_IDLE_TIMEOUT`, `SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS`, `CARGO_MANIFEST_DIR`, and `--remap-path-prefix`. Assert it does **not** contain a bare `RUSTFLAGS` assignment. Keep the existing assertions.

- [ ] **Step 2: run the check to verify it fails**

  Run: `cd ~/nixos-config && nix flake check`
  Expected: FAIL on the new assertions.

- [ ] **Step 3: extend the module**

  Add the three `[env]` entries to the existing `home.file.".cargo/config.toml".text`, and a `[target.x86_64-unknown-linux-gnu] rustflags` line carrying `--remap-path-prefix=<copy root>=/robin` (use the module's copy-root pattern; do not replace the repo's `cfg(target_arch)` flags). Update the module comment that currently says `SCCACHE_BASEDIRS` is not set here: basedirs are now injected by `remote-check.sh` per invocation.

- [ ] **Step 4: run the check to verify it passes**

  Run: `cd ~/nixos-config && nix flake check`
  Expected: PASS.

- [ ] **Step 5: commit and deploy**

  Commit in `~/nixos-config` by pathspec, then `just deploy-wait`. Verify on minipc that the generated file contains the new entries.

### Task 3: orphan-safe transient scope

**Files:**
- Modify: `scripts/remote-check.sh:264-283`
- Test: `scripts/test_build_host.sh`

**Interfaces:**
- Consumes: `sccache-basedirs.sh` from Task 1.
- Produces: an ephemeral `systemd-run --scope --unit=build-<copy>` wrapping the existing `systemd-inhibit → flock → build-slot → nix develop` chain, with `RuntimeMaxSec=7200` and `TimeoutStopSec=30`.

- [ ] **Step 1: add the fixture assertion**

  In `test_build_host.sh`, assert both remote command heredocs contain `systemd-run --scope`, `--unit=build-`, `RuntimeMaxSec=7200`, and `TimeoutStopSec=30`.

- [ ] **Step 2: run the fixture to verify it fails**

  Run: `bash scripts/test_build_host.sh` — expected FAIL on the scope assertions.

- [ ] **Step 3: wrap the remote command**

  Replace each `exec systemd-inhibit ...` in the two heredocs with `systemd-run --scope --unit=build-${remote_dir##*/} -p RuntimeMaxSec=7200 -p TimeoutStopSec=30 systemd-inhibit ...`. Keep `exec` only on the innermost command. Do not add a userspace reaper timer.

- [ ] **Step 4: run the fixture to verify it passes**

  Run: `bash scripts/test_build_host.sh` — expected PASS.

- [ ] **Step 5: smoke test the scope**

  Run: `ROBIN_BUILD_HOST=minipc scripts/remote-check.sh format tdd-bug-sweep-smoke`. While running, confirm `systemctl list-units 'runbg-*` is irrelevant — check `systemd-cgls` or `systemctl status 'build-tdd-bug-sweep-smoke.scope'` on minipc; after the run confirm the scope is gone and `ps` shows no orphaned `cargo`/`rustc` from it.

- [ ] **Step 6: commit**

  Commit by pathspec with message `build: bound each remote gate with a transient scope`.

### Task 4: tree-keyed gate copies

**Files:**
- Create: `scripts/gate-copy-name.sh`
- Modify: `scripts/remote-check.sh:45,159-172,228-240`, `scripts/push-gate.sh:114-122`, `scripts/live-worktrees.sh`
- Test: `scripts/test_build_host.sh`

**Interfaces:**
- Produces: `gate-copy-name.sh REPOSITORY [TREE_SHA]` prints `gate-<first 12 of tree sha>` when `TREE_SHA` is given, else `basename REPOSITORY`.
- Consumes: push-gate passes the tree of the head it gates via env `ROBIN_GATE_TREE`.

- [ ] **Step 1: add naming fixtures**

  In a synthetic git repo, assert `gate-copy-name.sh <repo>` equals its basename; assert passing a 40-hex tree sha gives `gate-<first 12>`; assert the same sha twice gives the same name and a different sha gives a different name. Assert `live-worktrees.sh` output does not contain a `gate-` entry even when a gate copy dir exists.

- [ ] **Step 2: run the fixture to verify it fails**

  Run: `bash scripts/test_build_host.sh` — expected FAIL (helper missing).

- [ ] **Step 3: implement naming and wire it**

  Create the helper. In `remote-check.sh`, make `copy_name()` call it with `$ROBIN_GATE_TREE` when set. In `push-gate.sh`, export `ROBIN_GATE_TREE=$(git rev-parse "${head}^{tree}")` immediately before the `remote-check --suites` call. In `live-worktrees.sh`, filter lines beginning `gate-`.

- [ ] **Step 4: add retention and verify prune**

  Add `GATE_COPY_TTL_DAYS` (default 7) to `remote-check.sh`; `--prune` removes gate-named remote copies older than that. Extend the fixture with one fresh and one old gate copy dir and assert only the old one is removed. Run `bash scripts/test_build_host.sh` — expected PASS.

- [ ] **Step 5: commit**

  Commit by pathspec with message `build: key gate copies by pushed tree with bounded retention`.

### Task 5: bounded FIFO admission

**Files:**
- Modify: `scripts/build-slot.sh`, `scripts/test_build_host.sh`

**Interfaces:**
- Consumes: existing `SLOT_DIR` / `SLOTS` CLI and `CARGO_BUILD_JOBS` behaviour.
- Produces: FIFO queue via `<slot_dir>/queue`, `BUILD_SLOT_WAIT_SECS` (default 7200), nonzero exit with a named reason on timeout.

- [ ] **Step 1: add queue fixtures**

  Keep the three existing slot fixtures. Add: two waiters released in arrival order when one slot frees; a waiter with `BUILD_SLOT_WAIT_SECS=1` exits nonzero within two seconds while the slot is held.

- [ ] **Step 2: run the fixture to verify it fails**

  Run: `bash scripts/test_build_host.sh` — expected FAIL on ordering and timeout.

- [ ] **Step 3: replace the rotation loop**

  Implement queue files named `<deadline>-<sequence>-<pid>` under `<slot_dir>/queue`; a waiter proceeds when its file is first and a slot is free; all waits bounded by `BUILD_SLOT_WAIT_SECS`. Keep core division and `CARGO_BUILD_JOBS` semantics identical. Never hold the admission lock while acquiring the slot lock.

- [ ] **Step 4: run the fixture to verify it passes**

  Run: `bash scripts/test_build_host.sh` — expected PASS on all fixtures.

- [ ] **Step 5: commit**

  Commit by pathspec with message `build: FIFO slot admission with a bounded wait`.

### Task 6: end-to-end smoke and measurement

**Files:**
- No new source; record results in the design doc's verification section.

- [ ] **Step 1: run the local fixture suite**

  Run: `bash scripts/test_build_host.sh` — expected PASS, zero failures.

- [ ] **Step 2: run one real gated landing**

  Run `ROBIN_BUILD_HOST=minipc scripts/remote-check.sh --suites format lint` from `build/gate-speedup` and confirm PASS.

- [ ] **Step 3: record the measurement checklist**

  Capture `sccache --show-stats` before and after, the two-copy wall time, orphan PID count after a deliberate harness kill, the remapped debug-info path, `CARGO_MANIFEST_DIR` isolation, and queue log rows. Put the numbers in the design doc under a dated `Last verified` line.

- [ ] **Step 4: final commit**

  Commit the measurement update by pathspec.
