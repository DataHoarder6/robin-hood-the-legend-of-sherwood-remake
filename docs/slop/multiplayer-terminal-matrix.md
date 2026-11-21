# The terminal-choice matrix: what it needs from the game and the harness

Written 2026-11-19 against `fork/main` `35a73e4df`, after the debriefing stopped
asking a discarded Yes/No (#48). The owner's requirement is a *full* matrix:
two seats, a mission that ends on purpose, each seat pressing Continue /
Restart / Load / Abort in every combination, a restart that actually restarts,
and no hang in any cell — plus quick save and quick load under two seats.

The harness exists and owns the shape of a scenario (section 2 of
[HARNESS.md](../multiplayer/HARNESS.md)); what it cannot express today is *ending
a mission on purpose* and *pressing one seat's debriefing button*. Both are gaps
in the game's developer surface, not in the harness format.

## What is missing, precisely

1. **A way to end a mission deliberately.** Missions end when their script
   decides (`Finalize`, objective completion, a blazon lost). No native ends a
   mission (`crates/robin_script_types/src/natives/signatures.rs`: `CompleteObjective`,
   `LoseBlazon`, `WinBlazon` are the closest), and `/console` is
   `not_implemented`, so a scenario cannot say "this run loses now". The abort
   row is reachable today through `/command` `"QuitMissionRequested"`
   (`crates/robin_rs/src/game.rs` sets `LevelInterrupted`); won and lost are not.
2. **A way to press a seat's debriefing button.** `/command` takes an exact
   `PlayerCommand`, and the terminal page's dismissal is
   `ModalDismiss { kind, result }` with the *exact* kind — `FinalDebriefing {
   text_id: Lose { index: n } }`, where `n` is the text index the mission script
   chose. A driver cannot guess it, and `/state` does not say what the peer is
   presenting: its `terminal` block carries only the ended operation and reason
   (`crates/robin_rs/src/observability/state.rs`).
3. **An assertion vocabulary for "nothing hangs".** Every scenario already has a
   deadline (`ScenarioDeadline`, exit 2), so a hang is a failed row by
   construction; what is missing is the row, not the verb.

## The design

**`/state` names the decision a peer is presenting.** The terminal flow's
`current_kind()` is already the authority that `queue_http_result` validates
against, so the state surface publishes exactly that, and the result vocabulary
the page accepts:

```json
"presentation": {
  "kind": {"FinalDebriefing": {"text_id": {"Lose": {"index": 2}}}},
  "results": ["Completed", "Restart", "Aborted", "Load"]
}
```

Published from `drive_tick_exit_modals` while the page is pending and cleared
when it is gone (`crates/robin_rs/src/game_session/terminal_debriefing.rs`), so
it describes what is on screen now, not what the process once did. This is the
same block an agent reads to know what a stuck client is waiting for; the
harness is its first consumer, not its reason.

**A developer trigger ends the mission.** `POST /mission-result {"result":
"won"|"lost"|"aborted"}` on the dev HTTP surface, documented in
`docs/JSON_SERVER.md` beside `/step-forward`. It ends the mission through the
same `Game`/terminal staging the script-driven end uses, so the debriefing, the
leaderboard and the campaign handoff are the real ones. It is a developer tool
like `/native` and `/step-forward`, available only with `--http-server`.

**Two harness verbs.** Section 2's rule is that a new verb bumps `schema` and
changes the runner, never a scenario, so:

- `{"fault": "command", "peer": N, "command": <PlayerCommand JSON>}` — `POST
  /command` on that seat. This is also what section 2 meant by `diverge`
  ("`POST /native` on that peer alone") for the command surface.
- `{"fault": "native", "peer": N, "op": "…", "args": […]}` — `POST /native`,
  the cheat/console path for scenarios that need a blazon or an objective.
- `{"fault": "resolve", "peer": N, "result": "Completed"}` — read that seat's
  `/state.presentation.kind`, post `ModalDismiss { kind, result }` to it. The
  runner interprets nothing: the kind comes from the game.

## The matrix (rows to add)

`H21`–`H26`, process layer, `proves.runner` this suite. Every row is two seats
on one mission, and every row fails by deadline if any seat hangs.

| row | mission ends | host presses | client presses | must assert |
| --- | --- | --- | --- | --- |
| H21 | aborted | Completed | Completed | both seats leave the page; `final-operation=quit` |
| H22 | aborted | Completed | Restart | host wins the disagreement; the session restarts; hashes equal after `begin-sim` |
| H23 | aborted | Restart | Completed | client follows the host's restart; hashes equal |
| H24 | aborted | Restart | Restart | one restart, not two; `count(transition,LoadRestart)` is one |
| H25 | aborted | Completed | (silent) | the host does not wait for the client forever: bounded ack budget, then the session leaves |
| H26 | won/lost | Completed | Completed | the campaign map reopens for both; no exit |

Won and lost variants reuse the same rows once `/mission-result` exists; the
result is the only field that changes.

Quick save / quick load is its own pair of rows — that decision stands. The
two-seat behaviour and every assertion are owned by H29 and H31 in
[HARNESS.md](../multiplayer/HARNESS.md); this page does not repeat them.

## What is deliberately not here

- Driving the *graphical* button with X11 (`scripts/validation/client_x11.py`):
  the click is not the contract, the `ModalDismiss` command is. A row that
  clicked would test cursor hit-testing, which the modal tests already own.
- `netem` faults: the runner refuses them by name until a netem driver exists
  (section 2), unchanged by this work.

## Crash reporting (owner requirement, 2026-11-19)

Every crash must (a) leave a bundle on disk, (b) upload it, and (c) become a
GitHub issue with enough detail to fix it — without a human copying logs.

**Client (`crates/robin_rs/src/crash.rs`, new).** A panic hook and fatal-signal
handler write `<install home>/crashes/<utc>-<fingerprint>/` containing: the panic
message and location, `std::backtrace`, the version line (`CARGO_PKG_VERSION` +
the git rev baked in at build time), platform and arch, the tail of the current
run log, the tail of `events.jsonl`, the last `/state` document, and the current
replay's path. The bundle is written **before** any upload, so an offline crash
still leaves evidence. `fingerprint` is a hash of panic location + version so a
loop cannot file a thousand issues.

**Upload.** `POST` the bundle as JSON to `ROBIN_CRASH_ENDPOINT` (or
`crash_report.endpoint` in the profile config); unset means local-only, which is
the default. Bounded by a short timeout, never blocking shutdown, never panicking
in the crash path.

**Receiver (`tools/crash-receiver/`, new, and the estate's server).** A small
service that authenticates the upload, redacts nothing it does not own, and files
one issue per fingerprint on the fork repo through a token held **server-side** —
a token must never ship in a binary. Issue body: fingerprint, version, platform,
panic line, backtrace, log tail, `/state`, and the bundle attached. A repeat
fingerprint adds a comment with the new occurrence count instead of a new issue.

**Why server-side.** The client only needs a URL. The GitHub credential, the
dedupe state and the rate limit live where they can be revoked, and the binary
stays shippable.
