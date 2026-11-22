# Full co-op campaign proof — design

Date: 2026-11-22. Status: proposed, awaiting approval.

## Intent

Prove that a full campaign can be played co-op from the first mission to
completion in one host-owned session, with no error states, and with every
choice made on the Sherwood campaign map observed and checked against the
game's own next-mission selection.

Why: the campaign flow (map selection, start, transitions, restart, replay,
debriefing, leaderboards) is where the reported bugs are. The user wants every
edge case covered by a test, every issue found fixed, and a multiplayer full
campaign that works reliably. Issue #49 criterion 2 requires "every mission of
the campaign, in order, transitioned by the game's own debriefing — not by a
dev trigger."

## Goals

1. A gated process-layer row drives a Linux host and Linux guest through the
   full campaign in one `--mp-campaign` session.
2. `/state` publishes campaign selection state: current mission, legal
   accessible choices, the host's committed choice, and whether the flow is at
   Sherwood awaiting a choice.
3. `/events` publishes a campaign-choice event when the host commits
   `PlayerCommand::CampaignSelectNextMission`, making each Sherwood choice
   durable evidence.
4. A driver `choose-next` verb reads the published state and sends the game's
   own command through `/command` on the host — no per-mission relaunch, no
   out-of-band dev trigger.
5. A repeating driver loop: end via `/mission-result` as either a win or a
   loss -> answer the debriefing with a typed `/step-forward` dismissal, which
   may be `Restart` or `Load` from a save slot -> if the flow lands at Sherwood,
   commit a legal choice -> next mission, until campaign completion.
6. Assertions across the whole run: `mission(count=N)`,
   `hash-equal(every=25)`, a terminal assertion at completion, zero `Desync`
   events, no session-failure/fatal/panic strings, no terminal error state,
   and a bounded wait at every step.
7. Each Sherwood choice is recorded in scenario evidence and asserted to match
   the mission actually entered next.
8. Failed missions are part of the run: at least one ends in `lost`, and from
   that debriefing both the Restart button and the Load-from-save button are
   exercised, by either seat, with no hang and no seat left on a page the other
   seat has left.
9. Cancellation is exercised after selection: a mission is picked on the
   Sherwood map and then cancelled from the mission-description modal, with no
   mission started and the map navigation restored.
10. Loading from an existing save is exercised: a save slot made before or
    during the run is loaded from the debriefing, and the campaign position and
    both seats' state hashes are restored.
11. Every edge case in the campaign flow has a named test: every decision
    point, every outcome, every seat, every availability state, from a
    code-derived case matrix.
12. Every issue found is fixed. Tests are red-first, then the fix; nothing
    lands red. The full campaign runs reliably end to end.
13. Any newly found defect gets a `docs/DEFECTS.md` row with a failing check
    before its fix (repo rule).

## Failure and recovery paths

The row is not a victory lap. It drives these decisions, each from either seat
where the game allows it, and records each one in the scenario evidence:

- A mission ends in `lost` through `/mission-result`; the debriefing is
  answered with `Restart`. The same mission is re-entered on the same
  transport, both seats at equal hashes, no page left behind. This exercises
  the restart button and its `restart_snapshot_exists` gate, so the
  already-known enabled-but-refusing case gets a failing check before a fix.
- A mission ends and the debriefing is answered with `Load` into an existing
  save slot. The restored campaign position and state hashes are asserted.
- A mission is selected on the Sherwood map, then cancelled at the
  mission-description modal. No mission starts; the map stays navigable.
- A seat stays silent on a page while the other decides; the wait is bounded
  and the session settles, as H23 already shows for Restart.

Lost missions and cancelled selections do not advance the campaign count; the
row asserts attempts separately from distinct missions entered.

## Non-goals

- Cross-play directions and packaging (criteria 3 and 9): BLOCKED, needs a
  Windows seat.
- Mid-campaign rejoin (4) and multi-session resume (8): separate rows.
- Landing with known failures: every defect the row surfaces is filed and fixed
  before the gate. The row lands green.
- Changing the game's mission-selection rules.
- Reusing another agent's unmerged worktree.

## Anti-goals

- Forcing each mission with `--mission` per launch.
- Using `/engine-dump` as the loop's authority: it is a debug surface, not a
  harness contract.
- A driver that computes the next mission itself. The game decides; the driver
  only makes the host's choice through the same command the UI sends.
- Any choice that bypasses host authority.

## Constraints

- Integration ref `fork/main`; worktree `.worktrees/full-campaign-proof`; land
  with `scripts/land.sh` under `runbg`; commits by pathspec.
- One DEFECTS row is one unit of work; the check is written before the fix and
  lives in a suite a gate runs.
- The process layer runs only on the provisioned Linux build host with
  licensed full-game data, a pinned binary, and loopback namespace isolation.
- The driver reads `/state` and `/events`, not log text.
- Full game edition: genesis `H01_Lin_VL`, finale `H12_Not_MP` (id `0x4948`);
  Sherwood is the HQ, not a mission. The walkthrough sequence is in
  `crates/robin_rs/src/main_menu/multiplayer_menu.rs` `campaign_mission_number`.
- `--mp-campaign` requires `--server` or `--connect` and `--mp-expected-players`.
- The corpus stops at H36, which proves `mission(count=2)`.
- `docs/multiplayer/HARNESS.md` still has a stale paragraph saying the
  handoff has no process row; correct it as part of this work.

## Approaches

**A — Gated harness row (chosen).** Add the two observability surfaces, add
the `choose-next` driver verb, and add one repeating scenario row that walks
the whole campaign. It uses the repo's own contract, is repeatable and gated,
and matches issue #49 criterion 2. Cost: an HTTP contract addition, a driver
change, and a new corpus row.

**B — Standalone repro spike (rejected).** A separate script reading
`/engine-dump` and looping on its own would be faster, but it is not gated,
depends on an ad-hoc debug surface, and gets frozen rather than landed. Not
worth a second implementation.

**C — Game-side auto-select flag (rejected).** Production code that picks the
first accessible mission at Sherwood makes the driver trivial, but changes game
behavior and weakens the claim that the game's own flow advanced the campaign.

## Case matrix

The exhaustive code-derived inventory is
[`docs/slop/campaign-flow-case-inventory.md`](../../slop/campaign-flow-case-inventory.md).
Every decision point and outcome in it is a matrix row: control, state, acting
seat, expected result, and the named test that proves it. Existing tests bind
to rows; a row without a test gets a new named test written red first. All rows
must be green before landing.

The first RED set is the inventory's remaining unproved list: the restart
enablement gate, the live debriefing phase chain, the Sherwood outcome arms and
non-host refusal, `Redisplay` and the map's Inspect/leaderboard actions,
leaderboard widget mapping, native replay-watch success, completed-mission
replay entry, quick-load host-only refusal, debriefing `Load` restore,
`YesOnly` leave prompt, and the missing link from a debriefing answer to the
mission entered next.

## What is decided here

- `/state` gains a `campaign` object sourced from the host's authoritative
  campaign: current mission identity, accessible mission indices, committed
  next mission index, `awaiting_choice`, the game's `suggested_choice`, a
  `complete` flag, and the catalogue mission count. Completion comes from the
  engine's canonical boundary (`achievement_envelope_complete`), not from a
  driver-side guess.
- The loop stops when the game publishes `complete`. It also asserts that the
  number of distinct missions entered equals the catalogue count; no mission
  list is hardcoded in the driver.
- The driver commits `suggested_choice` exactly. It never computes the order.
  Assertions require entered mission == committed choice == suggested choice,
  so a skipped or replay-looping mission cannot satisfy the row.
- The loop is an imperative `--campaign-walk` mode in `multiplayer_e2e.py`.
  The declarative corpus stays as it is for H01-H36; no schema repeat construct
  is added. New row `H37` binds to this mode.
- Each defect gets a `docs/DEFECTS.md` row and a failing named test first, then
  the fix. H37 lands green and the gate must pass.
- `/events` gains a campaign-choice event carrying the mission index and the
  committing seat, emitted once per commit rather than once per seat.
- Failed missions, Restart, Load from an existing save, cancel after selection
  and a silent seat are all rows in the matrix, not one-off checks.
- The chosen approach is A; B and C are not built.

## Left to the builder

- Exact field names on the new `/state` object and which state builder supplies
  them, without letting a peer read another seat's campaign.
- Exact event kind name and where it is emitted.
- Which suite gates the new row and how H37 binds its prover.
- The terminal assertion after the H12 outro, decided from an observed run
  rather than guessed.
- Per-step wait budgets and the page-answer policy when a peer stays silent.
- How each matrix row binds to its named test and how the end-to-end row relates
  to them.
