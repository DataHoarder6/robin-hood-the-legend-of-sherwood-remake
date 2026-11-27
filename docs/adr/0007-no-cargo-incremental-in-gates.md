# 0007 — No Cargo incremental in gate builds

- **Status:** DEPRECATED 2026-11-27 — superseded by [0013](0013-gate-incremental-without-sccache.md)
- **Date:** 2026-11-21
- **Related:** [`0002`](0002-finished-work-lands-in-session.md), [`0004`](0004-nonexpiring-invitations.md)

## Context

The gate used to disable Cargo incremental (`CARGO_INCREMENTAL=0`) so sccache
had one shareable key space across many per-tree copies. Those copies were
replaced by one locked `gate-shared` copy, which removed the cold rebuild and
made the original reason obsolete. A trial then set `CARGO_INCREMENTAL=1` and
`SCCACHE_ALLOW_INCREMENTAL=1`, expecting Cargo's incremental state to make
edited workspace crates cheaper. A real landing gate failed:

```
sccache <toolchain>/clippy-driver <toolchain>/rustc -vV
sccache: incremental compilation is prohibited: Unset CARGO_INCREMENTAL to continue.
```

sccache documents that `CARGO_INCREMENTAL=1` needs its allowance flag; a direct
`cargo clippy --version` probe with both variables succeeded, so the precise
trigger was not pinned down. No conclusion may be drawn from a version probe:
the failing signal was a full gate run, and a gate that can fail this way
blocks every landing (ADR 0002).

## Decision

Gate builds set `CARGO_INCREMENTAL=0` and do not set
`SCCACHE_ALLOW_INCREMENTAL`. The stable warm copy, not incremental, is the
build-speed mechanism. `scripts/test_build_host.sh` asserts both facts so the
decision cannot drift silently. Measurements and the sccache reasoning live in
[`docs/slop/BUILD_CACHE_AND_QUEUE.md`](../slop/BUILD_CACHE_AND_QUEUE.md) and
[`docs/slop/2026-11-20-build-reduction-research.md`](../slop/2026-11-20-build-reduction-research.md).

## Alternatives rejected

- **Keep incremental with the allowance flag.** Observed to fail a real gate;
  the marginal saving on edited workspace crates is small next to a warm copy's
  3–30 s rebuilds.
- **Treat a passing `clippy --version` probe as proof.** It does not exercise
  the compile path that failed.
- **Re-add incremental after only restarting the sccache daemon.** A plausible
  root cause, not a proven one. Reversal requires a full gate run with the exact
  gate environment and a recorded hit-rate/effect measurement.

## Consequences

- `CARGO_INCREMENTAL=0` is now a gate invariant with a fixture, not a
  cross-copy cache hack.
- Future readers must consult this ADR before "fixing" the zero; sccache's own
  docs permit incremental with an allowance flag, which is why the code alone
  does not explain the choice.
- The research note records the original trade-off; this ADR supersedes its
  verdict for the gate.
