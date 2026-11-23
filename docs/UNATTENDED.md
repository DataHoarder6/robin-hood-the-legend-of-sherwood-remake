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
  What a landing requires is defined in
  [LANDED requires origin read-back](#landed-requires-origin-read-back).
- **FILED** — a `deferred` row in [DEFECTS.md](DEFECTS.md) names its issue and
  ends the iteration. Three entries reach it: the bounded retry budget is
  exhausted ([Retry and quarantine](#retry-and-quarantine)); the circuit breaker
  hits two failed fixes or one repeated commit
  ([Circuit breaker](#circuit-breaker)); or a full-corpus row fails under load
  but passes in isolation ([Known flake policy](#known-flake-policy)). The row,
  never another rerun, ends the iteration.
- **BLOCKED** — the next step needs a capability the loop cannot supply. Name the
  missing capability and stop; do not ask the owner to provide it.

`HUMAN_GATE` is an annotation on items deferred to a human, not a fourth
terminal state: the iteration still ends LANDED or FILED for the Linux work it
completes. The scope decision and the gated items are in ADR 0010 — see
[Windows as a human gate](#windows-as-a-human-gate). Other capabilities can
still be BLOCKED under [Risk routing](#risk-routing).

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

## Decide before the run

Scope, human gates and authority are decided before a run, not during one. The
decisions are [ADR 0010](adr/0010-linux-first-unattended-goal.md) and the
`HUMAN_GATE` rows in [GAPS.md](GAPS.md). A tick does not dispatch research or
deep-dive subagents: it reads the authoritative sources, selects one assertion,
dispatches builder/reviewer/verifier, and ends in a terminal state. If a needed
decision is missing, the tick ends `BLOCKED` with that decision named — it does
not commission a study of it mid-run. Research that changes scope goes through
[research-before-building](https://github.com/DataHoarder6/home/blob/main/docs/agents/research-before-building.md)
and lands as an ADR or gap row before the next run.

## Regression coverage before `fixed`

The builder's failing check must cover the defect surface the DEFECTS.md row
names, not just its smallest reproducer; a group-command defect needs its
multi-actor case. The reviewer verifies that coverage. The exact rules are in
[`.opencode/agents/builder.md`](../.opencode/agents/builder.md) and
[`.opencode/agents/reviewer.md`](../.opencode/agents/reviewer.md); D12's multi-PC
teleport case is the row that exposed this gap.

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
recorded; start from [Iteration order](#iteration-order). The rules stay here
and in the agent files; the artifact records only where this loop stopped.

## Gate and land path

`scripts/remote-check.sh` and `scripts/land.sh` are the only paths: the first
runs the build-host gate, the second gates and lands the tree — see
[LANDED requires origin read-back](#landed-requires-origin-read-back) for what a
landing requires.
[PARALLEL_AGENTS.md](PARALLEL_AGENTS.md) owns the
build-slot/gate rule and the FIFO landing queue; one gate is in flight across
agents, and a competing gate queues behind the running one. Suite names live in
[TESTING.md](TESTING.md).

The verifier's gate contract lives in
[`.opencode/agents/verifier.md`](../.opencode/agents/verifier.md): changed docs
paths go in `ROBIN_DOCS_CHECK_PATHS`, and the process-layer variables are
forwarded by `scripts/remote-check.sh`. A missing binary or a pruned artifact is
a prerequisite failure, not a red assertion.

A stable tag is human work, not a loop action. The `robin --version` preflight
before a tag is scriptable and owned by [RELEASES.md](RELEASES.md).

## Retry and quarantine

Retries are bounded and quarantine is central.
[PARALLEL_AGENTS.md](PARALLEL_AGENTS.md) owns the budget (CI retries, no local
rerun) and the rule; a surviving failure is FILED in [DEFECTS.md](DEFECTS.md)
with status `deferred` and a row that names its issue. This page does not
restate the CI retry numbers; the per-item ceiling is in
[Circuit breaker](#circuit-breaker).

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
the discussion. This is an exception to the two-attempt
[circuit breaker](#circuit-breaker): a known flake is deferred on
identification, not after two attempts, and is never retried indefinitely or
"fixed". The quarantine rule itself stays with
[PARALLEL_AGENTS.md](PARALLEL_AGENTS.md).

## Risk routing

The loop never edits CI, deploy or landing authority, or host configuration:
`.github/workflows/*`, `scripts/land.sh`, `scripts/remote-check.sh`,
`scripts/release*.sh`, Nix files (`*.nix`, `**/*.nix`) and secret paths
(`.env*`, `**/*.env`, `**/.env*`, `**/*secret*`, `**/secrets/**`). An item that
needs one of them is BLOCKED with that capability named: never attempted, never
asked for. The director's
[`edit` denies](../.opencode/agents/director.md) enforce this path subset;
anything a deny does not cover — a path outside this repo, for example — is
BLOCKED by this rule, not by a permission. Stable tags: see
[Gate and land path](#gate-and-land-path) and [RELEASES.md](RELEASES.md).

## Windows as a human gate

Cross-play needs a real Windows machine to run one seat — issue #49's "one human
step". The scope decision is
[ADR 0010](adr/0010-linux-first-unattended-goal.md); `HUMAN_GATE` is the
annotation used for the deferred items, defined under
[Terminal states](#terminal-states). Wine replay parity
([RELEASES.md](RELEASES.md), [TESTING.md](TESTING.md)) proves only the
toolchain/FP/locale class; the OS-layer class stays unproven until a real
Windows seat runs a recorded session ([WINDOWS.md](WINDOWS.md)).

## Kill switch

`systemctl --user stop director-loop.timer` on `oci-vm` stops a running loop
immediately — the break-glass, not a terminal state. The durable form is a
commit to `~/nixos-config` setting `agent.director-loop.enable = false`, which
that host pulls and activates. The loop itself never edits machine config; a
request to do so is BLOCKED under [Risk routing](#risk-routing), with
`nixos-config` named as the missing capability. Last verified: 2026-11-22.

## One-week pilot metrics

Run `scripts/unattended-metrics.sh [SINCE]` on `oci-vm`. It prints each metric
with its source; the source is the measurement, not a stored report. A cost or
duration the script cannot derive is printed as `not available`, never
estimated. First-pass gate success and false quarantines come from the gate's
`summary.json` evidence indexed by [PARALLEL_AGENTS.md](PARALLEL_AGENTS.md), not
from the script; the script's `not available` rows name the source that must be
extended first.

First run, 2026-11-23, `SINCE=2026-11-22`: 1 LANDED, 1 FILED, 7 BLOCKED, 6
builder / 7 reviewer / 4 verifier dispatches, 16 quota failures, 0 reverts.
`fork/main` tip confirmed with `git ls-remote fork refs/heads/main`.
