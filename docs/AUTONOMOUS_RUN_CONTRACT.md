# Autonomous run contract

The pre-run contract for one autonomous run of the robin-hood-remake loop. A
separate pre-run session — not the director, which keeps `question: deny` —
interviews the owner and commits this file before the run. The tick re-reads it
at startup with `scripts/run-bounds.sh check docs/AUTONOMOUS_RUN_CONTRACT.md`;
the loop may not edit it. A new interview happens only when the owner changes
the contract or asks for one. Last verified: 2026-11-23.

## Scope and authority

- Scope is `robin-hood-remake` only. No other repo, machine or seat.
- Authority is full autonomy: gate, land, tag and publish, under
  [ADR 0011](adr/0011-autonomous-release.md). This is the explicit, ADR-backed
  deviation from [UNATTENDED.md](UNATTENDED.md)'s "a stable tag is human work"
  rule; everything else in that page still holds.
- The loop may execute `scripts/release-orchestrator.sh` and the human-authored
  web-promote entry point. It never edits those scripts, workflows or Nix files;
  execution authority is pre-granted here, never widened mid-run.
- The loop asks no question mid-run. An irreversible action the contract does
  not authorise, two conflicting clauses, a missing external permission, or an
  ambiguity that would change the contract's meaning halts the run in
  `NEEDS-DECISION`; routine gaps are contract defects fixed at the next
  interview.
- Observation is read-only: manual start, read-only attach, stop via
  `runbg stop autonomy`. No takeover in v1.
- [`scripts/autonomy-driver.sh`](../scripts/autonomy-driver.sh) is the run-level
  entry point. `runbg run autonomy -- bash scripts/autonomy-driver.sh` starts or
  resumes a run; `attach` is read-only, `status` prints the recorded state,
  `stop` sets the run's stop flag, and `resume <run-id>` continues an interrupted
  run from its recorded cursor. The driver is repo-local: it never edits Nix
  files, timer units or a provider setting.

## Work queue

[`docs/AUTONOMOUS_BACKLOG.md`](AUTONOMOUS_BACKLOG.md) is authoritative. The loop
works `ready:` items, files reviewer verdicts as `needs-work` or `rejected`, and
may promote only docs, tests and small refactors. Gameplay, schema, protocol,
release, deletion and version items stay `proposed:` for the owner, as does
any item whose `PATHS` names a protected path. The mechanical next-MINOR bump
and its generated notes are `ready:` when produced by
`scripts/release/prepare-next-minor.sh` from landed sources — never from agent
judgment. When the queue is empty, the
driver asks [`scripts/autobacklog.sh`](../scripts/autobacklog.sh) to generate
one `proposed:` item from a curated docs/test gap and applies the same
classifier; a deferred defect row is never a source. A proposal that fails the
classifier, or no gap to generate from, ends the run in `RUN-COMPLETE` with the
named reason `no-promotable-proposal` or `no-proposal-source`. Generation is
attempted once per run, so a refusal cannot spin.

## Bounds

These keys are machine-read by
[`scripts/run-bounds.sh`](../scripts/run-bounds.sh); a missing, duplicated or
changed key fails the tick closed with a named reason.

```text
wall_clock_minutes: 240
max_batches: 3
consecutive_red_gates: 2
bisect_no_landing_halts: true
# 90 min: a build-host gate takes about an hour. 900 s would FILED every real
# gate and trip the two-red circuit breaker before any batch could land.
gate_timeout_secs: 5400
cost_cap: none
contract_immutable: true
```

- 4 hours wall clock and at most 3 batches; a bound is hit after the current
  batch, ending the run in `RUN-COMPLETE`.
- Two consecutive red gates halt the run; so does a bisect that yields no
  landing. A per-gate timeout fails the batch closed.
- No cost cap and no paid-model fallback: quota exhaustion is a named `BLOCKED`
  sub-reason with fail-closed admission, never a provider switch.
- The contract is immutable by the loop. The director's frontmatter denies both
  `edit` and `shell` access to this path, so a `sed -i` or append cannot bypass
  the edit deny. The run driver also calls `scripts/run-bounds.sh guard-edit`
  before each path it writes; that guard refuses this path with
  `contract-is-immutable`.

## Release policy

MINOR bumps only (`0.x.0`); MAJOR is human-only and new tags are created, never
moved. The bump and "releasable?" decision come from Conventional Commits in the
batch, not agent judgment. [`scripts/auto-release.sh`](../scripts/auto-release.sh)
and [ADR 0011](adr/0011-autonomous-release.md) own the tag path. The loop may
execute `scripts/release-orchestrator.sh` to verify the tag and its assets,
publish the browser artifact and record the release outcome; it never edits the
orchestrator, the workflow or the promote wrapper. The machine-repo pin,
`apps-sync`, installs and seat launches stay with the owner.
