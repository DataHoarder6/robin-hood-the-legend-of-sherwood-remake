# 0009 — One gate per batch of ready worktrees

- **Status:** Accepted
- **Date:** 2026-11-22
- **Related:** [`0002`](0002-finished-work-lands-in-session.md), [`0007`](0007-no-cargo-incremental-in-gates.md), [`0008`](0008-one-stable-gate-copy.md)
- **Research:** [`docs/research/batched-agent-builds.md`](../research/batched-agent-builds.md)

## Context

Gating each finished worktree separately pays the full gate cost per branch, and
gating concurrently races the single `gate-shared` copy (ADR 0008). Agents finish
at uneven times, and a runtime restart replays jobs. What is wanted is Bors-style
batched validation: merge approved work into one tree, gate that tree once, and
fast-forward the target to the exact tested tree; on red, bisect the batch.

## Decision

A controller owns a staging tree and lands batches; agents only edit, commit and
declare readiness. One gate validates the combined tree, using the repo's
existing `land.sh`/`remote-check.sh` machinery and the locked `gate-shared` copy.

- **Controller-owned staging tree.** Agents keep their per-agent worktrees. Only
  the controller assembles ready commits into a controller-only staging tree, so
  a sync cannot swap source under a running gate (ADR 0008's lock invariant).
- **Readiness marker keyed by head SHA.** An agent publishes readiness with a
  compare-and-swap ref update keyed by its commit SHA. Same SHA is a no-op; a new
  commit needs a new marker. State is keyed by content, never by attempt count.
- **FIFO epochs.** One ticket per ready branch, ordered by arrival, min 4 / max 5
  members, with a wait timeout before departing short. Freeze member head SHAs
  and the base tip at cut; late finishers go to the next epoch.
- **Conflict preflight.** Before assembly, run `git merge-tree --write-tree` per
  member in ticket order (no worktree or index touched). Park a conflicting
  member and report its paths; the controller never resolves conflicts.
- **One gate, keyed by tree.** Stamp the gate result under the batch tree (and
  suite set). A replay of the exact tree returns the stored result instead of
  rebuilding; a stale or absent result fails closed into "gate again".
- **Replay defence.** Idempotency keys, fd locks with PID reaping, and a
  transient-scope TTL with process-group kill keep a killed replay from
  stranding a lock or leaving an orphan build. Landing is confirmed by remote
  read-back, not push exit status.
- **Bisect on red.** A failing batch is split in half and each half re-gated;
  green halves land, and the single bad member is quarantined to its owner.

This intentionally bends ADR 0002 (finished work lands in the same session): a
ready branch may wait one epoch. Epochs stay short — the controller lands the
batch in the turn the last member becomes ready — so the deviation is bounded.
No platform merge queue, no force push, no hand push, and no weakening of the
exact-tree rule.

## Consequences

- A batch pays one gate instead of one per branch, with the warm `gate-shared`
  cache reused across attempts (ADRs 0007 and 0008 still hold).
- A ready branch can wait up to one epoch before landing, a bounded exception to
  ADR 0002.
- Replays are safe because all state is keyed by content (member head SHAs, batch
  tree, suite set) and locks are PID-reaped and TTL-bounded.
- A red batch costs a bisection sequence rather than a single failure diagnosis,
  but isolates the bad member without discarding the green ones.
