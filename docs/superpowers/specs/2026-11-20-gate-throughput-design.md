# Gate throughput design

Date: 2026-11-20

## Intent

A landing gate currently costs about an hour. With 3–5 agents working in
parallel, the build host runs several copies of the same wasm/LLVM graph,
saturates 12 cores, and every gate becomes slower. This design makes one gate
cheap without weakening what a gate proves.

Evidence and primary sources live in
[`docs/slop/BUILD_CACHE_AND_QUEUE.md`](../../slop/BUILD_CACHE_AND_QUEUE.md).
That document owns the sccache mechanics, the orphan-cleanup primitives and the
measurement checklist. This document owns the decision and the order of work.

## Goals

1. One build result per distinct `(git tree, toolchain, target, features, flags)`;
   identical trees share it instead of rebuilding.
2. Different trees share every unchanged rustc invocation through sccache.
3. A gate still proves the exact tree the push lands. No graph weakening.
4. A killed agent leaves no orphaned build and cannot strand a lock.

## Scope calls made here

These resolve the builder check's open questions; they are this design's calls,
not inherited decisions.

- Orphan cleanup is **in scope for phase 1**: goal 4 and the verification
  checklist cannot pass without it. The build runs in the transient scope from
  [`docs/slop/BUILD_CACHE_AND_QUEUE.md`](../../slop/BUILD_CACHE_AND_QUEUE.md#3-no-orphan-after-a-killed-agent-layer-harness-cleanup--os-ttl);
  cancellation signals the process group.
- "Tree" means `HEAD^{tree}` for a push gate, and the worktree basename for an
  interactive `remote-check` on a dirty tree. The two never collide.
- Tree-keyed copies apply **only** to the push-gate path. Interactive copies keep
  their worktree names because
  [`scripts/live-worktrees.sh`](../../../scripts/live-worktrees.sh) and
  `remote-check --prune` match those names. Gate copies get a retention cap so
  trees cannot grow without bound.
- Phase 1 needs one repository edit: `remote-check.sh` currently sets
  `SCCACHE_BASEDIRS=$(pwd)` per invocation, which overrides host config with a
  single basedir. The rest of the host config is machine state and belongs in
  `nixos-config`, not here.
- The host config, the basedir regeneration on copy add/prune, and the sccache
  daemon restart are one scripted operation owned by phase 1, not a manual step.
- Phase 2 fixes the existing
  [`scripts/build-slot.sh`](../../../scripts/build-slot.sh) admission loop
  (try-lock rotation, no total wait bound) rather than adding a third lock. One
  admission point, one copy lock, never held together.

## Non-goals and anti-goals

- No `CARGO_TARGET_DIR` redirect or shared target dir: this repo forbids it, and
  it adds a second lock class with an AB-BA deadlock path.
- No moving wasm/client-release suites off the per-push path in this design.
  A green lightweight push must not let an unproven tree onto `fork/main`.
- No crate split in phase 1. It is parity-sensitive and expensive; do it only if
  phase 3 is still justified by measurements.
- Not a redesign of the suites themselves. `scripts/push-gate.sh` stays the one
  definition of what a landing owes.

## Approaches considered

| Approach | Trade-off | Decision |
| --- | --- | --- |
| One shared `CARGO_TARGET_DIR` with a global lock | Violates repo rules; serialises all agents on 12 cores; AB-BA deadlock with copy lock | Rejected |
| Tree-keyed gate copies + shared sccache | Keeps per-copy targets and one lock class; shares work without sharing mutable output | Selected |
| Tiered gates (heavy suites only on merge) | Cheapest to build, but a push no longer proves the tree it lands | Rejected |
| Slice `robin_engine` into smaller crates | Fixes the root cause of full-graph rebuilds; large parity-sensitive refactor | Deferred to phase 3, data-gated |

## Design

### Phase 1 — share rustc results (build-host configuration)

Apply the configuration in
[`docs/slop/BUILD_CACHE_AND_QUEUE.md`](../../slop/BUILD_CACHE_AND_QUEUE.md#1-what-causes-the-misses-and-the-exact-configuration):
`sccache` basedirs for each copy root, `CARGO_INCREMENTAL=0` for gates,
`--remap-path-prefix` in the host user Cargo config, verbatim manifest-dir env
deps, and the local cache size. Keep per-copy `target/` dirs and the existing
per-copy `flock`. Confirm the before/after checklist in that document; a phase is
done only when the cross-copy hit rate and the debug-info path both check out.

This phase changes build-host configuration plus the one `remote-check.sh`
override named above, and adds the orphan-safe scope wrapper to the gate path.

### Phase 2 — tree-keyed copies and bounded admission

In [`scripts/remote-check.sh`](../../../scripts/remote-check.sh), derive the
copy name from the tree being gated rather than the worktree directory name, so
two agents on the same tree share one copy, one lock and one result. Keep the
source sync, the changed-crate cache purge and the remote read-back as they are.

If measurement shows contention remains, replace the try-lock admission loop in
`scripts/build-slot.sh` with the single FIFO admission point and bounded waits
from
[§4 of the research doc](../../slop/BUILD_CACHE_AND_QUEUE.md#4-a-build-queue-that-cannot-deadlock-or-starve).
Never hold the admission lock and the copy lock at the same time.

### Phase 3 — data-gated crate slicing

Only if `cargo build --timings` still shows one crate dominating after phases 1
and 2, split along existing module seams so a one-line engine change rebuilds a
fraction of the wasm graph. This is a separate decision and gets its own design.

## Verification

Use the checklist in
[`docs/slop/BUILD_CACHE_AND_QUEUE.md#5-measurement-checklist-before--after`](../../slop/BUILD_CACHE_AND_QUEUE.md#5-measurement-checklist-before--after)
unchanged: sccache stats, identical-suite wall time from two copies, zero new
orphan PIDs after a deliberate harness kill, remapped debug-info path,
manifest-dir isolation, and queue log invariants.

### Measured 2026-11-20

- Local fixtures: `bash scripts/test_build_host.sh` — **26 passed, 0 failed**.
- Single-suite gate smoke (`format`): PASS; the user scope
  `build-gate-speedup-smoke-*` was created and gone after the run.
- Multi-suite gate smoke (`--suites format lint`): PASS, **13 m 40 s** wall,
  510 MB peak memory on the runtime; the build host's scope and slot were
  released and no `cargo`/`rustc` from that gate survived.
- sccache after minipc's restart: 6 hits / 3,257 misses (0.18 %) before the
  final smoke, 7 hits / 34 misses (17.07 %) after. The restart reset the
  counters, so this proves cross-copy hits exist but is **not** a steady-state
  hit rate. Take that from a later identical-tree run.
- Not exercised here: deliberate harness kill, remapped debug-info path, and
  `CARGO_MANIFEST_DIR` isolation from two copies. Fixtures pin the
  configuration; those three need a live two-copy session.

## Left to the builder

- Phase 1: apply the host config, record before/after stats.
- Phase 2: implement and test tree-keyed copy naming; add queue logging if an
  admission point is needed; keep the gate contract intact.
- Update the pointers in the root docs only if a new file is added; do not
  duplicate the research doc's content.
