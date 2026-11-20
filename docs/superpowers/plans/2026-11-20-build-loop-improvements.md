# Build-loop improvements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut the number of full Rust builds this estate pays for, and make the remaining ones cheaper, without weakening the landing gate.

**Architecture:** Keep one locked, stable `gate-shared` copy on the 12-core build host with Cargo incremental; stop triggering builds that no gated path owes; fix the two sccache/remap defects in the gate command; measure before any profile or linker change.

**Tech Stack:** Bash, Cargo, sccache 0.19, Nix devShells, existing `check-quality.sh`/`push-gate.sh`/`remote-check.sh`.

**Spec:** `docs/slop/2026-11-20-build-reduction-research.md` and `docs/slop/2026-11-20-build-freshness-and-profiling-research.md`.

## Global Constraints

- One locked `gate-shared` copy; never a per-tree copy or a shared `CARGO_TARGET_DIR`.
- The full gate still runs on the exact tree that lands; `cargo check` is a preflight only.
- Every claim cites a primary source; every measured change reports an effect, not an exit code.
- Docs follow `~/home/DOC-STYLE.md`; volatile numbers are replaced by the command that prints them.

## Review Focus

- A docs-only diff owing a Rust build — pinned by the `docs` suite in `push-gate.sh`.
- A stale gate copy being pruned mid-build — pinned by the shared copy living outside the prune root.
- A literal `$(pwd)` reaching rustc in the remap flag — pinned by the rendered env line in `test_remote_check.sh`.
- A metric reported as success without evidence — pinned by the measurement commands in `docs/TESTING.md`.

---

### Task 1: Gate-work hardening (in flight, land first)

**Files:** `scripts/check-quality.sh`, `scripts/push-gate.sh`, `scripts/remote-check.sh`, `scripts/prune-copies.sh`, `scripts/build-status.sh`, JS package files.

- [ ] Rebase `retro-gate-1120` onto current `fork/main`; drop the obsolete fresh/locked prune commit — the shared copy now lives outside the prune root.
- [ ] Verify `check`, `docs`, `web`, `editor`, `tooling`, `client-check` suites and the toolchain preflight; run their local suites.
- [ ] Land with `scripts/land.sh`; read the remote tip back.

### Task 2: Fix sccache and remap in the gate command

**Files:** `scripts/remote-check.sh:317-326,360-392`.

- [ ] Set `SCCACHE_CLIENT_SIDE=1` in both env blocks (upstream-recommended in 0.19).
- [ ] Write the remap prefix as `"--remap-path-prefix=\$PWD=/robin"` so the remote shell expands the copy root after `cd`; the old single-quoted `\$(pwd)` left a literal string in rustc's argv.
- [ ] Verify the rendered line expands to an absolute path; run `bash -n`.
- [ ] Commit and land.

### Task 3: Make measurements repeatable

**Files:** `docs/TESTING.md`.

- [ ] Replace stale "one copy per branch / build-host" guidance with the `gate-shared` design.
- [ ] Add the five measurement commands (fingerprint diagnostic, `--timings`, self-profile, `--print link-args`, env+sccache audit) and the `cargo tree -i` reverse-dep hint.
- [ ] Link both research notes; verify relative links resolve; commit and land.

### Task 4: Measure, then decide the trials

- [ ] On `gate-shared`, run `cargo build --timings -p robin_rs --bin robin` cold then warm; record the report path and dominant crate.
- [ ] If dependency `opt-level=3` dominates, trial a gate profile with Cargo's default `opt-level=0` for deps and build deps before changing `Cargo.toml`.
- [ ] Trial Kache for interactive worktrees and mold against Wild only if the timing report points at linking; neither is a committed change without a measured win.

## Deferred deliberately

- Shared `CARGO_TARGET_DIR` — Cargo's relative-path hashes collide, no locking.
- `cargo check` as a gate replacement — misses codegen-only errors; sccache cannot cache metadata-only emits.
- Cargo's built-in CAS cache (`S-needs-design`) and distributed sccache (one host).
- Whole-workspace linker/backend swaps and `debug=0` without timing evidence.
