# 0008 — One stable gate copy, not one per tree

- **Status:** Accepted
- **Date:** 2026-11-21
- **Related:** [`0002`](0002-finished-work-lands-in-session.md), [`0007`](0007-no-cargo-incremental-in-gates.md)

## Context

Every worktree used to sync its own `gate-<tree sha>` copy to the build host,
so a day produced 17 fresh copies with ~3.9 GB `target/` trees each. `sccache`
keys include the working directory and the compiler's out-dir, so a byte-identical
source in a new copy missed every Rust entry: measured 0.36 % Rust hits. A shared
`CARGO_TARGET_DIR` was tried instead and recompiled 511 crates — Cargo's unit
hashes use relative workspace paths, and the directory has no locking against
concurrent gates. Neither approach kept the cache that makes a repeated gate
cheap.

## Decision

All worktrees sync into one locked copy, `~/build/robin-gate/gate-shared` on
`minipc`, held from the first rsync until the last suite finishes. It lives
outside the `~/build/robin/` root that `--prune` scans, so no agent's cleanup can
delete another gate's cwd. The copy is infrastructure, not an abandoned worktree
cache. Build identity, sccache and remap facts are in
[`docs/slop/BUILD_CACHE_AND_QUEUE.md`](../slop/BUILD_CACHE_AND_QUEUE.md); the
measurements are in
[`docs/slop/2026-11-20-build-reduction-research.md`](../slop/2026-11-20-build-reduction-research.md).

## Alternatives rejected

- **Tree-keyed copies.** A fresh cwd and out-dir give `sccache` new keys even
  for identical source; 17 cold builds a day.
- **Shared `CARGO_TARGET_DIR`.** Relative-path unit hashes collide, there is no
  locking, and Cargo's maintainers do not endorse sharing a target dir across
  workspaces. The repo forbids redirecting it.
- **Waiting for Cargo's built-in content-addressed cache.** `S-needs-design`,
  not accepted.
- **Kache or a distributed build service.** No measurement yet shows a remaining
  cost that a third-party daemon would remove; the stable copy took the cold
  rebuild away.

## Consequences

- Only one gate builds at a time; other agents queue on the copy lock rather
  than racing three cold builds on a 12-core host.
- The gate result describes the tree synced under that lock, so a second sync
  cannot swap the source mid-suite (ADR 0002's exact-tree rule).
- Cargo incremental stays off in gates: ADR 0007. The speed comes from reusing
  the copy's artifacts, not from changing compiler modes.
