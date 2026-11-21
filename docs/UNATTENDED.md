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

## Terminal states

Every iteration ends in exactly one of three states and reports it. There is no
fourth state and no prompt to the owner to choose, confirm, or continue.

- **LANDED** — the fix passed its suites and was landed by `scripts/land.sh`,
  whose remote read-back confirms it on the integration ref. Landing is the only
  path to this state; a push's exit status is not proof of it.
- **FILED** — the failure survived its bounded retry budget and was quarantined
  as a [DEFECTS.md](DEFECTS.md) row with an owner and a bug link. Quarantine ends
  in a row, never another rerun.
- **BLOCKED** — the next step needs a capability the loop cannot supply. Name the
  missing capability and stop; do not ask the owner to provide it.

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

## Gate and land path

`scripts/remote-check.sh` and `scripts/land.sh` are the only paths: the first
runs the build-host gate, the second gates the tree it will land, pushes, and
reads the remote back. [PARALLEL_AGENTS.md](PARALLEL_AGENTS.md) owns the
build-slot/gate rule and the FIFO landing queue; one gate is in flight across
agents, and a competing gate queues behind the running one. Suite names live in
[TESTING.md](TESTING.md).

A stable tag is human work, not a loop action. The `robin --version` preflight
before a tag is scriptable and owned by [RELEASES.md](RELEASES.md).

## Retry and quarantine

Retries are bounded and quarantine is central.
[PARALLEL_AGENTS.md](PARALLEL_AGENTS.md) owns the budget (CI retries, no local
rerun) and the rule; a surviving failure is FILED in [DEFECTS.md](DEFECTS.md)
with an owner and bug link. This page does not restate the numbers.

## Windows as BLOCKED

Cross-play needs a real Windows machine to run one seat — issue #49's "one human
step". The loop cannot provision one, so any item whose check requires a Windows
seat is BLOCKED with that capability named, never a question to the owner. Wine
replay parity ([RELEASES.md](RELEASES.md), [TESTING.md](TESTING.md)) proves only
the toolchain/FP/locale class; the OS-layer class stays unproven until a real
Windows seat runs a recorded session ([WINDOWS.md](WINDOWS.md)).
