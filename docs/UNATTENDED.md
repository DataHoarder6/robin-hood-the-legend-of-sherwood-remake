# Unattended orchestration

One page for the loop that drives [issue #49](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/49)
toward a full co-op campaign. The standing goal and the current gap index are
[GAPS.md](GAPS.md); the loop never redefines or narrows them, and does not close
the issue until a full campaign has been played end to end on the recorded build.

The four loop agents live in [`.opencode/agents/`](../.opencode/agents/):
[director](../.opencode/agents/director.md) (primary) dispatches
[builder](../.opencode/agents/builder.md),
[reviewer](../.opencode/agents/reviewer.md), and
[verifier](../.opencode/agents/verifier.md). This page owns the loop's policy;
the agent files own only their role and permissions.

Launch the loop explicitly with `opencode run --agent director ...`; no
`default_agent` is configured here, so a bare `opencode run` does not select it.

The evidence behind these rules, and the mitigation each one adopts, is in
[unattended-loop-research.md](slop/unattended-loop-research.md).

## Terminal states

Every iteration ends in exactly one of three states and reports it. There is no
fourth state and no prompt to the owner to choose, confirm, or continue.

- **LANDED** — the fix passed its suites and was landed by `scripts/land.sh`.
  What a landing requires is defined in LANDED requires origin read-back below.
- **FILED** — the failure survived its bounded retry budget and was filed in
  [DEFECTS.md](DEFECTS.md) with status `deferred`: the row names its issue.
  The row, never another rerun, ends the iteration.
- **BLOCKED** — the next step needs a capability the loop cannot supply. Name the
  missing capability and stop; do not ask the owner to provide it.

## LANDED requires origin read-back

A `git push` exit status of `0` is not evidence of a landing. `scripts/land.sh`
gates the tree, pushes to the integration ref, then reads that ref back with
`git ls-remote` before it reports success. LANDED is claimed only when the
read-back shows the pushed commit on the integration ref. A green push with no
read-back is an unverified push, not LANDED; the script owns the remote and ref.

Gate evidence, not agent narration, decides success. A builder, reviewer or
verifier reporting "tests pass" is not a gate result; only the suite verdict
and the origin read-back count. [PARALLEL_AGENTS.md](PARALLEL_AGENTS.md) owns
the gate rule; this section owns the definition of a landed commit.

## Iteration order

The director reads the authoritative sources — issue #49,
[HARNESS.md](multiplayer/HARNESS.md), and [DEFECTS.md](DEFECTS.md) — and selects
the first failing assertion in this fixed order:

1. Issue #49's nine "What done means" criteria, in the order written in the issue.
2. Within a criterion, the failing [HARNESS.md](multiplayer/HARNESS.md) corpus
   rows in `H01`→`Hnn` order (each row's `asserts:` clause).
3. Then any failing [DEFECTS.md](DEFECTS.md) row in `D01`→`Dnn` order (its `check`).

One assertion per iteration. A criterion or row with no failing assertion is
skipped; the order is recomputed each iteration from the sources above, never
from a cached copy.

## Handoff artifact

Every tick is a fresh context and `TimeoutStartSec` can kill a tick mid-work.
Before selecting anything, the director reads
`/home/matt/.local/state/robin-director/handoff.json` on `oci-vm` — outside the
repo, never committed, no git churn. It is state, not a second source of rules.

Write the file at selection and after every state transition. It records exactly
five fields: selected item, worktree path, gate unit/result, current terminal
state or `pending`, and next action. An absent file means no prior selection was
recorded; start from Iteration order. The rules stay here and in the agent
files; the artifact records only where this loop stopped.

## Gate and land path

`scripts/remote-check.sh` and `scripts/land.sh` are the only paths: the first
runs the build-host gate, the second gates and lands the tree — see LANDED
requires origin read-back above for what a landing requires.
[PARALLEL_AGENTS.md](PARALLEL_AGENTS.md) owns the
build-slot/gate rule and the FIFO landing queue; one gate is in flight across
agents, and a competing gate queues behind the running one. Suite names live in
[TESTING.md](TESTING.md).

A stable tag is human work, not a loop action. The `robin --version` preflight
before a tag is scriptable and owned by [RELEASES.md](RELEASES.md).

## Retry and quarantine

Retries are bounded and quarantine is central.
[PARALLEL_AGENTS.md](PARALLEL_AGENTS.md) owns the budget (CI retries, no local
rerun) and the rule; a surviving failure is FILED in [DEFECTS.md](DEFECTS.md)
with status `deferred` and a row that names its issue. This page does not
restate the CI retry numbers; the per-item ceiling is in Circuit breaker below.

## Circuit breaker

Two fix attempts per item, then FILED — never a third. The director keeps an
attempt count for the selected assertion; a second failed attempt ends the
iteration in FILED with a `deferred` row in [DEFECTS.md](DEFECTS.md). This is a
per-item ceiling on top of the retry budget, not a replacement for it.

Repeat detection is part of the breaker: never emit the same commit twice for
the same item. If the builder produces a commit already recorded for that
assertion, the loop is stuck, not retrying — stop and end the iteration in
FILED. Two
failed fixes or one repeat both exhaust the item; the row, not another rerun,
ends it. An explicit termination contract and a hard ceiling are what keep a
fresh-context loop from spinning or declaring false success.

## Known flake policy

A corpus row that fails in a full-gate run but passes in isolation is a known
load flake, not a defect to keep fixing. Record one `deferred` row in
[DEFECTS.md](DEFECTS.md): class `release`, reproduction = full corpus run on
the build host, check = isolated row recheck, and a link to the issue that owns
the discussion. The row is the end of the item — do not retry it indefinitely
and do not land a "fix" for it. The quarantine rule itself stays with
[PARALLEL_AGENTS.md](PARALLEL_AGENTS.md).

## Risk routing

The loop never edits `.github/workflows/*`, `scripts/land.sh`,
`scripts/remote-check.sh`, release scripts (`scripts/release*.sh`), or any
machine config or secret (`*.nix` included). Those files are CI, deploy and
landing authority, or host configuration — human capabilities. If an item's fix
needs one of them, the item is BLOCKED with that capability named: never
attempted, never asked for. Stable tags: see
[Gate and land path](#gate-and-land-path) and [RELEASES.md](RELEASES.md). The
director's `edit` denies enforce the file subset
([director.md](../.opencode/agents/director.md)).

## Windows as BLOCKED

Cross-play needs a real Windows machine to run one seat — issue #49's "one human
step". The loop cannot provision one, so any item whose check requires a Windows
seat is BLOCKED with that capability named, never a question to the owner. Wine
replay parity ([RELEASES.md](RELEASES.md), [TESTING.md](TESTING.md)) proves only
the toolchain/FP/locale class; the OS-layer class stays unproven until a real
Windows seat runs a recorded session ([WINDOWS.md](WINDOWS.md)).

## Kill switch

`systemctl --user stop director-loop.timer` on `oci-vm` stops a running loop
immediately — the break-glass, not a terminal state. The durable form is a
commit to `~/nixos-config` setting `agent.director-loop.enable = false`, which
that host pulls and activates. The loop itself never edits machine config; a
request to do so is BLOCKED under Risk routing above, with `nixos-config`
named as the missing capability. Last verified: 2026-11-22.

## One-week pilot metrics

Collect these for one week from the loop's first unattended run. Each row names
its source; the source is the measurement, not a stored report.

| Metric | Command / source |
| --- | --- |
| Tasks attempted, LANDED, FILED, BLOCKED | `journalctl --user -u director-loop.service` — one terminal state per iteration |
| First-pass gate success | Gate evidence (`scripts/remote-check.sh` verdict / `summary.json`, indexed by [PARALLEL_AGENTS.md](PARALLEL_AGENTS.md)) — first gate versus retries |
| Revert rate | `git log fork/main --grep=revert -i`; tip confirmed with `git ls-remote fork refs/heads/main` |
| Review iterations | Director journal, reviewer rounds per item; cross-check `gh issue view 49` |
| Time-to-land p50/p90 | Director journal timestamps: iteration start → LANDED |
| Blocked dwell | Director journal timestamps: BLOCKED → clear; capability issue in `gh issue` |
| Cost per landed task | `opencode stats` (tokens/cost per session) ÷ LANDED count from the journal |
| False-quarantine count | [DEFECTS.md](DEFECTS.md) `deferred` rows whose check later passes without a fix |

A cost or duration that cannot be produced by one of these sources is not
reported; do not fill the table with estimates.
