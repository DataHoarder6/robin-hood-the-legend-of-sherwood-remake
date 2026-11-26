# Campaign endgame plan, reconciled with `fork/main`

Status of [issue #55](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/55)
against `fork/main` `6a86371f3930` on 2026-11-26. This is a plan, not a claim
that any gate ran. It supersedes the phase checklist in #55, which was written
against an older tree; the issue body itself is owner-owned and was not edited
by this work. Keep [#49](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/49)
open until the owner's checks are done. Last verified: 2026-11-26.

## Reconciliation

| #55 text | Tree at `6a86371f3930` | Consequence |
| --- | --- | --- |
| Phase 0: H36 uncommitted, no gate, no evidence | H36 is in the corpus and [DEFECTS.md](../DEFECTS.md) D07 is `fixed` with retained evidence | Do not re-land phase 0 |
| "Next row H37" | H37 is the cooperative-pause core row ([HARNESS.md](HARNESS.md)) | Never use `H37` for the mission-three walk; reserve a fresh id |
| "Phase 1: exploratory run past mission 2" | H36 and H38 both stop at `mission(count=2)`; no row goes beyond mission two | The walk is still unproved |
| "Three found-not-fixed terminal defects" | The `WIN`, same-identity restart and `LOOSE` rows are covered by H33, H35 and H34; [campaign-coop.md](../slop/campaign-coop.md) no longer calls them unfixed | Do not re-file them |
| D06 "fix" task | D06 is `fixed` with gate evidence | Do not retry |
| "BLOCKED — not an agent task" for Windows | [ADR 0010](../adr/0010-linux-first-unattended-goal.md) reclassifies those items as `HUMAN_GATE` annotations | Keep working on Linux; do not emit a Windows `BLOCKED` or claim cross-play |
| "Unproved: the host does not open a session (#44)" | #44 closed; H30, H36 and H38 run campaign sessions | The stale sentence in #49 is a tracker-edit item for the owner |

## Work order after reconciliation

One failing assertion per iteration, in the order in
[UNATTENDED.md](../UNATTENDED.md). The first three items below are available to
the loop; the owner-only items are annotations and are never claimed complete
by an agent.

1. **Mechanical checks.** `python3 scripts/validation/scenario_corpus.py --check`
   passes at this ref; `python3 -m unittest discover -s scripts/validation -p
   campaign_proof_test.py` passes with 28 tests. Retain both outputs in the
   issue evidence when the loop re-checks this plan.
2. **CP01–CP04, deterministic checker.** The claim fixtures and the checker are
   in [CAMPAIGN_PROOF.md](CAMPAIGN_PROOF.md). They prove that broken evidence is
   refused at the exact tick, mission, action or seat. They do not prove a game
   behaviour. Add their unittest to the `tooling` suite in
   `scripts/check-quality.sh` before treating them as gate-backed; that file is
   outside this slice's ownership. **Owner-owned edit:** add one line to the
   tooling suite and run the gate at the exact tree.
3. **Mission-three process row.** Reserve a fresh id (not H37) for a short
   handoff chain that goes through the game's own debriefing and asserts
   `mission(count>=3)`, hash equality at the declared cadence and no developer
   trigger. The current runner binds faults to a frame: `due()` reads the
   host's frame counter, which restarts with the mission, so two
   `mission-result` faults at frame 40 would both fire in mission one.
   **Owner-owned edit:** bind faults, resolves and expectations to a mission
   index in `scripts/validation/multiplayer_e2e.py`, with per-mission counters
   and per-mission deadlines (`docs/multiplayer/HARNESS.md` section 2 is the
   contract). Do not write the row before that binding exists; a row the runner
   cannot drive is a red suite, not evidence.
4. **Real observations.** Run the process suite on the Linux build host, then
   feed its evidence to the CP claims. A CP verdict from a fixture is never a
   substitute for an observation from two seats. One gate in flight, exact-tree
   verification and origin read-back follow
   [PARALLEL_AGENTS.md](../PARALLEL_AGENTS.md).
5. **Ledger hygiene.** D11 and D15 in [DEFECTS.md](../DEFECTS.md) record what
   is unreproved at this ref. Neither is marked `fixed` by this plan; a green
   gate at the exact tree is required first.

## Owner gates (annotations, not terminal states)

- **HUMAN_GATE — cross-play, criterion 3:** four directions on real Windows
  hardware (`player-b-pc` behind `player-b-tunnel`). Wine replay parity covers
  the toolchain/FP/locale class only. No agent run may claim it passed.
- **HUMAN_GATE — Windows installer and menu launch, criterion 9:** see
  [WINDOWS.md](../WINDOWS.md). Packaging artefacts building is not an install
  proof.
- **HUMAN_GATE — full-campaign play on the recorded build:** #56. Harness wins
  are sufficient for the handoff-chain proof but not for this play proof.
- **Owner-owned repository edits:** the `tooling` suite line, the runner's
  mission-index binding, and the issue bodies of #49/#55. These are named
  capabilities, not human-play steps; this slice does not perform them.

## Definition of done (unchanged)

#49 closes only when a full campaign has been played end to end on the recorded
build, the Linux criteria are observed, and all three owner gates above are
recorded as passed. A Linux fixture proof does not close it.
