# Autonomous work queue

This file is the authoritative work queue for one
[autonomous run](AUTONOMOUS_RUN_CONTRACT.md). The loop works `ready:` items,
files reviewer verdicts, and generates `proposed:` items when nothing is ready.
It is state the loop writes, not a guide; the rules live in the contract and in
[`scripts/autobacklog.sh`](../scripts/autobacklog.sh). Last verified: 2026-11-23.

## Record format

Records are pipe-delimited lines in the block below, one per work item:

```text
# id|class|kind|scale|title|paths
```

- `class`: `proposed` (not workable), `ready` (workable this run), `needs-work`
  or `rejected` (reviewer verdicts).
- `kind`: `docs`, `tests`, `refactor`, `gameplay`, `schema`, `protocol`,
  `release`, `deletion`, `version`. The path→suite signals in
  [`scripts/push-gate.sh`](../scripts/push-gate.sh) decide the kind.
- `scale`: `small` or `large`.

The loop may promote only `docs`, `tests`, and `refactor` with `scale=small`,
and only when `PATHS` names no protected path: `scripts/land.sh`,
`scripts/remote-check.sh`, `scripts/push-gate.sh`, `scripts/gate-stamp.sh`,
`scripts/release*.sh`, `.github/**`, `*.nix` and secrets. `gameplay`, `schema`,
`protocol`, `release`, `deletion`, `version` and all large refactors stay
`proposed` for the owner. Generated proposals are `docs/small` and must be
promoted by the classifier, never by agent judgment.

A deferred defect row is never a proposal source: the known-flake policy in
[`UNATTENDED.md`](UNATTENDED.md) says such a row is never re-fixed. When the
queue is empty, `generate-proposed` takes the first unused
`IDEA|title|paths` line below, where `title` is a real docs or test gap and
`paths` is the file it concerns. With no candidate it fails
`no-proposal-source`, and the driver ends the run with that named reason.

## Commands

```sh
bash scripts/autobacklog.sh next-ready
bash scripts/autobacklog.sh generate-proposed ['title']
bash scripts/autobacklog.sh promote P001
bash scripts/autobacklog.sh verdict P001 needs-work   # or rejected
```

`next-ready` prints the first `ready` record in file order, or fails with
`no-ready-item`. `generate-proposed` takes an explicit title, or derives the
next `docs/small` proposal from the curated `IDEA|title|paths` candidates below;
with none unused it fails `no-proposal-source`. `promote` and `verdict` rewrite
one record's class and fail closed with `unknown-item`, `not-proposed`,
`not-promotable` (including a protected `PATHS` entry) or `unknown-verdict`.
Only the director may promote; the reviewer and verifier may file a verdict
through the same script and nothing else
([`.opencode/agents/`](../.opencode/agents/)).

## Queue

```text
```

## Proposal sources

Curated low-risk docs/test gaps, one per line, that the loop may turn into a
`proposed` item and offer to the promotion classifier. Leave empty when no such
gap is open — the run then ends `RUN-COMPLETE no-proposal-source` and names this
block as what is needed.

```text
```
