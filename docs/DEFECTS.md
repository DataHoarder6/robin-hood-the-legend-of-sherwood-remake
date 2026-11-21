# Defect ledger

One row per diagnosed defect, in the order it was diagnosed. A row is the unit
of work: "fix all the bugs" is not a task, "empty this table" is.

This file is the index, not the evidence. A row links to the document or test
that owns the detail; the audits (`docs/slop/multiplayer-unbounded-waits.md`),
the harness matrix (`docs/multiplayer/HARNESS.md`) and the session handovers
(`docs/HANDOVER.md`) stay the place where a class of defect is described. A row
that repeats their content is a second source for one fact.

## Classes

Every row names one. The class decides the shape of the fix, which is why it
comes before the patch:

| Class | The defect is | The fix is a mechanism, not a patch |
| --- | --- | --- |
| `wait` | progress depends on a message a peer may never send | a deadline with a named consequence (`docs/slop/multiplayer-unbounded-waits.md`) |
| `authority` | two participants author the same state | one author per event, replicated as a command |
| `desync` | two simulations disagree at a frame | deterministic tick, one RNG source, per-frame state hash |
| `generation` | a transition applies to a stale transport generation | a sequence on every transition; stale ones are refused |
| `identity` | two participants disagree about what session they are in | refuse at join with a named reason; adopt host-authored context |
| `presentation` | the screen and the simulation disagree about a page | presentation reads state, never mutates it |
| `release` | the artifact is wrong, not the code | a build step that proves the artifact before it ships |
| `data` | the content is wrong (mission, asset, save) | a check over the data, not a runtime guess |

## Statuses

`open` — reproduced, no fix. `fixed` — the check exists and fails without the
fix. `deferred` — reproduced, deliberately not fixed yet; the row names the
issue. `out-of-scope` — the behaviour is the original's, or the platform's.

## Rows

| id | class | symptom | reproduction | check | status |
| --- | --- | --- | --- | --- | --- |
| D01 | release | the shipped Windows exe exits `0xc0000005` before `main`, even for `--version`; MinGW wrote TLS `SECREL32` offsets as `.text` base relocations | ship a `windows-gnu` release and run it on Windows | `native-release.yml`: the built exe starts and carries no `.text` base relocations (`docs/WINDOWS.md`) | fixed |
| D02 | identity | a client reconnecting across a mission start was refused as an incompatible seat, so every session died at the first win or loss | finish a mission while a client is reconnecting | `reconnect_rejects_identity_change_and_adopts_host_context` (`docs/HANDOVER.md` §5) | fixed |
| D03 | generation | a snapshot-transition prepare was applied to the transport generation that did not receive it, so a lost commit suppressed the replacement mission's `begin-sim` and the restart deadlocked | drop a client's `CommitSnapshotTransition` at the `transition-commit` barrier, then restart the mission (#43) | `H20` | fixed |
| D04 | presentation | the Sherwood perks panel showed the literal `%i` as the clover price when game data was loaded, because the row and balance lines used the `%u/%d/%s`-only trading substitute on `"Amulets: %i"` | launch with full-game data (`Level.res` ships `"Amulets: %i"`; the Rust fallback ships `%d`) and open the Sherwood perks panel | `perk_row_renders_numeric_cost_from_the_loaded_i_template` and `balance_renders_numeric_clovers_from_the_loaded_i_template` (`client`) | fixed |
| D05 | wait | every process-layer multiplayer scenario hung waiting for the host's connect string after children moved into systemd service units: the driver polls loopback HTTP inside its `unshare --net` namespace, but a systemd service starts in the manager's namespace, so `/state` was unreachable and no sample was ever recorded | run `multiplayer-e2e` on `fork/main` after `b5ccc6bdd`; the host logs show the HTTP server listening and a live iroh endpoint, while `samples/host.jsonl` stays empty | `supervised_test.py`: the child command uses `--scope` (caller's namespace) and carries the TTL and control-group kill; the gate reaches `observed` for the connect string | fixed |
| D06 | generation | the joining client aborted with "mission-end leaderboard was captured more than once" (`terminal_debriefing.rs:1099`) on the second terminal transition after a host mission-result + restart fault: the one-shot `MissionLeaderboardRuntime::preparation` was captured on the first mission end and never re-armed for the replacement mission, so `capture_terminal` panicked and the process aborted | run the full `multiplayer-e2e` gate under load; H23 fails with the client at SIGABRT while H23 alone passes | a unit test that drives two terminal transitions through a snapshot restart and asserts each capture gets a fresh preparation; H23 `observed` in a full gate run | open |

## How a row gets here

1. A session ends with evidence: a `bug-reports/<utc>-<origin>-<n>/` bundle, a
   desync report, or a log tail. `scripts/triage_bug_bundle.py <bundle>`
   prints the row to paste and the `--replay` command that reproduces it.
2. The class is chosen from the table above, by what the evidence shows, not by
   where the code happens to be.
3. The check is written before the fix, and it fails. A harness row (`Hnn`) is
   preferred when the defect is a session; a named test when it is one process.
4. `fixed` is claimed only when the check is in a suite the push gate runs.

A defect with no reproduction stays `open` and says so; it does not get a
"probably fixed" row.

## What this file is not

- Not the wait audit: `docs/slop/multiplayer-unbounded-waits.md` owns which
  waits exist and which budgets they carry.
- Not the coverage matrix: `docs/multiplayer/HARNESS.md` owns which transition
  and fault each scenario proves.
- Not the issue tracker: a `deferred` row names its issue, and the issue keeps
  the discussion.
