# 0013 — Gate copy: Cargo incremental, no sccache

- **Status:** Accepted — Supersedes [0007](0007-no-cargo-incremental-in-gates.md)
- **Date:** 2026-11-27
- **Related:** [`0008`](0008-one-stable-gate-copy.md), [`0009`](0009-batched-agent-builds.md)
- **Evidence:** [`docs/slop/build-time-root-causes.md`](../slop/build-time-root-causes.md)

## Context

ADR 0007 kept `CARGO_INCREMENTAL=0` in gates because sccache refused an
incremental compile and broke a landing. It did not question sccache itself.
On `gate-shared` the wrapper earns nothing: Cargo's own fingerprints in the one
stable copy (ADR 0008) already reuse every unchanged artifact, and the
99 % Rust hit rate sccache reported there on 2026-11-27 was the cache handing
back dependency artifacts that one suite's rustflags had just overwritten for
the next (the thrash this ADR's evidence note describes, now removed).

Meanwhile the work that remains on a warm gate is rechecking and rebuilding
`robin_engine` (18 MB of source) and `robin_rs` (11 MB) from scratch, which is
exactly what rustc's incremental state exists to shorten.

## Decision

- **Gate copy:** the remote command runs with `RUSTC_WRAPPER=` (an empty value
  resets the build host's `build.rustc-wrapper = "sccache"`) and leaves
  `CARGO_INCREMENTAL` unset, so the dev, test and `wasm-dev` profiles use their
  incremental default. Release profiles stay non-incremental by Cargo's default.
- **Pool copies** (interactive checks): unchanged — sccache with one basedir per
  copy and `CARGO_INCREMENTAL=0`, because a pool copy is handed between
  worktrees and sccache cannot cache incremental crates.
- `scripts/test_build_host.sh` asserts both, and that no command sets
  `SCCACHE_ALLOW_INCREMENTAL`.

## Alternatives rejected

- **Incremental with `SCCACHE_ALLOW_INCREMENTAL`.** ADR 0007's failure; it also
  caches nothing for the incremental crates, so it pays the wrapper's cost for
  no hit.
- **Keep both off.** Leaves every warm gate recompiling the two largest crates
  non-incrementally in every configuration.

## Consequences

- A cold gate copy recompiles C/C++ dependencies without sccache; the copy is
  stable, so that happens once per toolchain or lockfile change.
- Incremental state adds disk to `gate-shared`'s `target/`;
  `remote-check.sh`'s free-space floor and `just doctor`'s target report still
  apply, and `target/debug/incremental` is the first thing to prune.
- The first gate after this lands rebuilds the gate copy's workspace crates once
  without the wrapper; measure from the second.
