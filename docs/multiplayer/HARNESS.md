# Multiplayer harness decisions

Six decisions for the co-op end-to-end harness, each ending in a testable rule.
Behaviour claims name the file that owns them; a `.../` path is relative to
`crates/robin_rs/src/`. Written 2026-11-18 against `fork/main` `f9f594dfc`; the
corpus bindings in section 1 were made executable on 2026-11-18.

## 1. Coverage matrix (#5)

`Hnn` = scenario in the corpus below. `#nn` = deferred to that issue, because a
red test now would pin behaviour nobody chose. `Ox` = out of scope, reason `x`
below. Failure modes: **BA** drops before ack, **AA** after ack, **MT**
mid-transfer, **AC** after commit, **DR** desyncs then returns, **NR** never
returns, **SM** stays at a modal, **DC** two peers pick one character, **HX**
host exits. Barriers named below: `content-ready` and `snapshot-adopt` are
transport admission (`.../multiplayer/native/server.rs`); `begin-sim` is
`ReadyBarrier` and `transition-commit` is `SnapshotTransitions`
(`.../multiplayer/native/server_protocol.rs`); `modal-ack` is the shared-story
barrier. A barrier is *reached* when a peer is at it and *released* when it is
past it, so `running` never satisfies `content-ready` or `snapshot-adopt`. The
process layer schedules a fault only on a fact that persists — a frame counter, a
set `begin_sim_frame`, a seat — because both pre-acknowledgement admission
phases (`peer_waiting_for_snapshot`, `peer_waiting_for_begin`) are shorter than
one `/state` sample at 100 ms: a process row that anchored a fault on either
would wait until its deadline, and a row that claimed one `reached` would claim
what the sampler cannot see. Those drops are proved at transport, where the same
seat is dropped inside one process.

| Transition | BA | AA | MT | AC | DR | NR | SM | DC | HX |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mission start | #25 | #25 | #25 | Oa | H08 | #25 | #25 | H02 | H12 |
| mission success | #25 | #25 | H13 | H14 | H16 | #25 | H15 | Ob | #25 |
| mission failure | #25 | #25 | H13 | H14 | H16 | #25 | H15 | Ob | #25 |
| retry | #25 | #25 | H13 | H07 | H16 | #25 | H15 | Ob | #25 |
| abort | #25 | #25 | H13 | H07 | H16 | #25 | H15 | Ob | Oc |
| map transfer (`LevelNext`) | #25 | #25 | H13 | H07 | H16 | #25 | H15 | Ob | Oc |
| mid-mission join | #25 | #25 | #25 | #25 | #25 | #25 | #25 | #25 | H12 |
| mid-mission drop | Oe | Oe | Oa | Oe | H08 | #25 | #25 | Ob | Oc |
| reconnect | H17 | H17 | H18 | H07 | H08 | #25 | #25 | Ob | #25 |
| return to menu | Oc | Oc | Oc | Oc | Oc | Oc | Oc | Oc | Oc |
| server shutdown | #25 | #25 | #25 | #25 | #25 | #25 | #25 | Ob | H19 |
| content mismatch | H11 | #25 | H18 | #25 | Od | #25 | Od | Od | H19 |

Map transfer is retry's outer-mission rebuild
(`crates/robin_rs/src/game_session/mod.rs`) with a campaign mutation.

- **Oa** no distinct after-commit or mid-transfer window; the mid-mission drop or
  reconnect row proves the fault.
- **Ob** no gameplay input is admitted after the terminal frame, so a simultaneous
  selection cannot occur (`.../game_session/terminal_debriefing.rs`).
- **Oc** leaving ends the session; no per-peer resolution remains. The decided
  part, host-authoritative quit, is `H06`.
- **Od** before admission there is no seat and no engine state.
- **Oe** a drop in ordinary play enters no barrier — `NetEvent::Disconnected`
  holds only the dropping peer (`.../game_session/multiplayer.rs`) — and the
  decided pause-then-resolve is unimplemented, so it waits for #25.
- Mid-mission join is unreachable: `claim_seat` refuses a fresh owner once
  `next_seat >= expected_players` and `ReadyBarrier::candidate` wants
  `expected_players - 1` peers, so no seat budget is left; it becomes expressible
  when #25 releases the start barrier without every expected seat.

A campaign is not one of these transitions: it is the dimension that decides how
many of them a session has. One host-owned campaign is a *session* over its
missions (`crates/robin_rs/src/game_session/mod.rs`), so a mission end hands the
whole party to the next mission instead of ending the session and leaving each
install to write its own save. H30 is its baseline: a host that advertises a
campaign — the listing says `campaign` and the fixed party size, and no roster —
and a second seat from its own install that joins it with no local save, both
then running the same mission at equal hashes. Its failure cells are the ordinary ones — a drop
before or after a mission-end commit is H13/H14, a reconnect during one is H07 —
so the row adds no column to the matrix. The mission-end handoff itself has no
process row yet: ending a live mission from outside needs a rollback-safe
mission-end command and the two-seat debriefing acknowledgement, which the
terminal matrix rows (H21-H27) own; until they land, the handoff is stated by
the session route (`crates/robin_rs/src/game_session/mod.rs:run_session_body`)
and its unit tests rather than by a process row.

### Corpus

`baseline` runs first in every scenario of its row; `fault` is the cell's own
case; `invariant` and `vocabulary` are not cells.

| id | layer | role | must assert | extends |
| --- | --- | --- | --- | --- |
| H01 | core | baseline | the pair baseline: one `BeginSim` admits both peers with equal `state_hash` at a frame off the 25-frame boundary, and the hash oracle compares on the exact 25-frame boundary rather than against newer state. asserts: `hash-equal(peers=0,1,every=25)`; `barrier(name=begin-sim,state=released)`; `count(subject=barrier,name=begin-sim,times=1)` | `story_regression::pair` |
| H02 | core | fault | seat 1 owns the PC it selects; seat 0's selection of that PC is refused while a connected partner owns it in `Exclusive`. asserts: `seat(seat=1,owns=true)`; `seat(seat=0,owns=false)` | `robin_engine/src/engine/coop.rs` |
| H03 | core | baseline | `LevelSucceeded` → prepare/ready/commit `CampaignExit`; committed engine equals the host snapshot. asserts: `transition(payload=CampaignExit(level_succeeded),acked=0,1,committed=true)`; `final-operation(operation=level_succeeded)` | `completed_mission_snapshot_commits_the_hosts_campaign_before_rebuilding` |
| H04 | core | baseline | same for `LevelFailed`. asserts: `transition(payload=CampaignExit(level_failed),acked=0,1,committed=true)`; `final-operation(operation=level_failed)` | same test's failure arm |
| H05 | core | baseline | `LevelFailed` commits `CampaignExit(LevelFailed)`, the commit barrier completes once, and the peer re-adopts the host snapshot; `Restart`/`LoadRestart` is a debriefing action, not a transition. asserts: `transition(payload=CampaignExit(level_failed),acked=0,1,committed=true)`; `count(subject=barrier,name=transition-commit,times=1)`; `barrier(name=snapshot-adopt,state=reached)` | `completed_mission_snapshot_commits_the_hosts_campaign_before_rebuilding` + `current_snapshot_ack_commits_once_and_detaches_authority` |
| H06 | core | baseline | a host quit ends the session with the authoritative operation `Quit`. asserts: `final-operation(operation=quit)` | `robin_rs/src/game.rs` |
| H07 | core | fault | after a drop the peer adopts the host snapshot and rejoins the baseline hash agreement under one `BeginSim`. asserts: `hash-equal(peers=0,1,every=25)`; `barrier(name=begin-sim,state=released)`; `count(subject=barrier,name=begin-sim,times=1)`; `barrier(name=snapshot-adopt,state=reached)` | `pair` + `Link::reconnect` |
| H08 | core | fault | a mutated peer is caught at the next compared frame and recovers to hash agreement from the host snapshot. asserts: `hash-diverged(peers=1)`; `barrier(name=snapshot-adopt,state=reached)`; `hash-equal(peers=1,every=25)` | `.../game_session/frame_prepare/pre_tick.rs` |
| H09 | core | vocabulary | duplicated `ModalProgress`/`ModalDecision` yields one decision per occurrence. asserts: `modal(count=1)`; `barrier(name=modal-ack,state=released)` | `modal_session_delayed_surface_and_delivery_matrix` |
| H10 | core | invariant | a transition's commit barrier completes exactly once. (The `SnapshotTransitions` tests this row names also cover the wrong-id, un-awaited-seat and duplicate-ack refusals.) asserts: `count(subject=barrier,name=transition-commit,times=1)` | `SnapshotTransitions` tests |
| H11 | transport | fault | a seat admitted with its content ready but never acknowledging `BeginSim` does not wedge the host: the ready barrier waits, extends once for the joiner, expires on its budget, evicts that seat (admission and writer dropped, owner kept for a re-admission) and releases on the frames that did arrive, quorum ignored. The process layer cannot schedule this drop: both pre-ack admission phases live between two `/state` samples, so a process row would assert a `reached` it can never observe. asserts: `barrier(name=begin-sim,state=released)` | `.../multiplayer/native/server.rs` + `.../multiplayer/native/peer_sessions.rs` |
| H12 | process | fault | connect to a closed endpoint → the client reports `Game loop failed: multiplayer: failed to connect to` and exits non-zero with no seat. Its `content-ready` absence is not asserted: the barrier is unsampleable, so the absence would hold whether or not the client had reached admission. asserts: `log(contains=Game loop failed: multiplayer: failed to connect to,count=1)`; `exit(peers=1,status=nonzero)` | `.../multiplayer/client_session.rs` |
| H13 | transport | fault | a mutated transition id or a lost ack never commits the transition, and both peers keep the same transition id. asserts: `transition(payload=CampaignExit(level_succeeded),acked=0,1,committed=false)`; `transition-id-equal(peers=0,1)` | `crates/robin_rs/src/host/transport.rs` |
| H14 | process | fault | a restarted native owner (`identity: "fresh"`) while its seat is parked and the roster full → the host refuses the claim (`… already has its configured 2 players`), exactly one admission is logged in the session, and the replacement exits non-zero. It never reads the rejection text: `reject_opening` writes a `Reject`, but the handler ends before the peer sees it and the client reports `read frame header: connection lost`, so decoding that frame belongs to a transport row. asserts: `log(contains=multiplayer session already has its configured 2 players,at_least=1)`; `log(contains=multiplayer client connected,count=1)`; `exit(peers=1,status=nonzero)` | `peer_sessions::claim_seat` |
| H15 | core | fault | a live client that settles the mission end does not complete locally: adoption of the host snapshot is still absent when the run is read. asserts: `barrier(name=snapshot-adopt,state=absent)` | `.../game_session/terminal_debriefing.rs` |
| H16 | core | fault | a mismatch seen during recovery starts no second reconnect. asserts: `barrier(name=snapshot-adopt,state=reached)`; `hash-equal(peers=1,every=25)`; `count(subject=reconnect,name=1,times=1)` | `.../game_session/frame_prepare/pre_tick.rs` |
| H17 | transport | fault | a transition pending when a seat reconnects completes with the same id on both peers once that seat acks again. asserts: `transition(payload=CampaignExit(level_succeeded),acked=0,1,committed=true)`; `transition-id-equal(peers=0,1)` | `.../multiplayer/native/server.rs` pending-Prepare resend |
| H18 | transport | fault | content admission needs no seat: a client reaches `content-ready` and is later admitted with a seat. asserts: `barrier(name=content-ready,state=reached)`; `seat(seat=1,owns=true)` | `host_content_preflight_uses_no_seat_then_exact_resume_reconnects` |
| H19 | process | fault | host exit with a seat admitted — the kill is anchored on the monotonic `begin_sim_frame`, the only fact that says the seat was admitted → that seat's reconnects are bounded: it gives up and exits non-zero within the client's reconnect budget (`RECONNECT_BUDGET`), and the process leaves well after the host died because the drop first waits out the transport's 30 s idle timeout; the content-transfer window itself stays a transport row. asserts: `log(contains=giving up on reconnection,count=1)`; `exit(peers=1,status=nonzero)` | `.../multiplayer/client_session.rs` |
| H20 | core | fault | a prepare whose commit frame is lost with the transport generation that received it never suppresses the replacement mission: the peer adopts the replacement snapshot, and a fresh `BeginSim` releases it to `running` (the #43 deadlock). asserts: `barrier(name=snapshot-adopt,state=reached)`; `barrier(name=begin-sim,state=released)` | `story_regression::a_lost_commit_cannot_suppress_the_replacement_missions_begin_sim` |
| H21 | core | invariant | the `modal-ack` barrier counts only seats in this session's simulation: a page drawn while a roster seat is still waiting for `BeginSim` completes on the host's decision, and a seat the start barrier released before the page closed still owes an answer. asserts: `barrier(name=modal-ack,state=released)` | `story_regression::a_page_never_awaits_a_seat_that_is_still_waiting_for_begin_sim` + `a_page_awaits_a_seat_released_into_the_mission` |
| H22 | process | fault | both seats acknowledge the aborted mission's page with Continue: the engine's `LevelInterrupted` reaches the debriefing, each seat leaves the page, and the session publishes and commits its campaign-exit snapshot rather than ending on one seat's answer. A forced-mission launch has no next mission, so the host's own exit is what ends it, and the client — whose transport generation the committed snapshot already ended — leaves on its unconfirmed-end budget (`SNAPSHOT_CONFIRM_BUDGET`) instead of reconnecting forever. asserts: `presentation(peers=0,1,state=gone)`; `log(contains=opening debriefing exit_code=LevelInterrupted,at_least=1)`; `log(contains=snapshot transition prepared (CampaignExit),at_least=1)`; `log(contains=snapshot transition committed,at_least=1)`; `exit(peers=0,1,status=zero)` | `.../game_session/terminal_debriefing.rs` |
| H23 | process | fault | the host answers the aborted mission's page with Restart and the client presses nothing: the host's vote is the decision, the session publishes a restart snapshot, the client adopts it and agreement resumes on the 25-frame cadence. asserts: `presentation(peers=0,state=gone)`; `log(contains=snapshot transition prepared (Save),at_least=1)`; `log(contains=rebuilding mission transport,at_least=1)`; `hash-equal(peers=1,every=25)`; `exit(peers=0,1,status=running)` | `.../game_session/terminal_debriefing.rs` |
| H24 | process | fault | the client answers Completed and the host answers Restart: the client's answer is a proposal, the host's Restart is the decision, and the client adopts the restart snapshot and keeps agreeing on it. asserts: `presentation(peers=0,1,state=gone)`; `log(contains=modal outcome proposed to host,at_least=1)`; `log(contains=snapshot transition prepared (Save),at_least=1)`; `log(contains=rebuilding mission transport,at_least=1)`; `hash-equal(peers=1,every=25)`; `exit(peers=0,1,status=running)` | `.../game_session/terminal_debriefing.rs` |
| H25 | process | fault | both seats answer Restart, the client first: its answer is a proposal and leaves the page open, the host's Restart is the decision, and the client's proposal never becomes a second one — the server publishes exactly one restart snapshot (one `payload_bytes=` line) and both seats stay in agreement on the replacement mission. The press order is the row's: a host that answers first closes the client's page before it can be pressed, which is a different claim (H32). asserts: `presentation(peers=0,1,state=gone)`; `log(contains=payload_bytes=,count=1)`; `hash-equal(peers=1,every=25)`; `exit(peers=0,1,status=running)` | `.../game_session/terminal_debriefing.rs` |
| H26 | process | fault | the host answers the page and the client never does: no frame schedules the client's acknowledgement while the host holds its decision, so the host is the one seat that ends the wait — it waits out the modal acknowledgement budget (`MODAL_ACK_BUDGET_MS`, .../multiplayer/modal_session.rs) and continues into the campaign exit. asserts: `presentation(peers=0,1,state=gone)`; `log(contains=snapshot transition prepared (CampaignExit),at_least=1)`; `log(contains=snapshot transition committed,at_least=1)`; `exit(peers=0,1,status=zero)` | `.../multiplayer/modal_session.rs` |
| H27 | process | fault | a won mission's page is acknowledged by both seats: the engine's `LevelSucceeded` reaches the debriefing, both seats leave the page, and the session publishes and commits the campaign-exit snapshot. The forced-mission launch then ends the application on both seats, as every mission end does there. asserts: `presentation(peers=0,1,state=gone)`; `log(contains=opening debriefing exit_code=LevelSucceeded,at_least=1)`; `log(contains=snapshot transition prepared (CampaignExit),at_least=1)`; `log(contains=snapshot transition committed,at_least=1)`; `exit(peers=0,1,status=zero)` | `.../game_session/terminal_debriefing.rs` |
| H28 | process | fault | the same for a lost mission: `LevelFailed` reaches the debriefing, both seats leave the page, and the campaign-exit snapshot is published and committed before the forced-mission launch ends. asserts: `presentation(peers=0,1,state=gone)`; `log(contains=opening debriefing exit_code=LevelFailed,at_least=1)`; `log(contains=snapshot transition prepared (CampaignExit),at_least=1)`; `log(contains=snapshot transition committed,at_least=1)`; `exit(peers=0,1,status=zero)` | `.../game_session/terminal_debriefing.rs` |
| H29 | process | baseline | the host presses quick-save then quick-load under two seats: a standalone multiplayer mission has no campaign to save, so quick-save writes the executor's local diagnostic and quick-load finds no quick slot; neither press ends the session, opens a page or moves the client out of agreement. asserts: `log(contains=multiplayer quick-save captured as a local diagnostic,count=1)`; `hash-equal(peers=1,every=25)`; `final-operation(operation=level_in_progress)`; `exit(peers=0,1,status=running)` | `.../game_session/live_gameplay.rs` |
| H30 | process | baseline | a campaign host opens a session a second seat joins from its own install with no local save, and both run the same campaign mission at equal hashes. asserts: `barrier(name=begin-sim,state=released)`; `hash-equal(peers=1,every=25)`; `mission(peers=0,1,count=1)`; `exit(peers=0,1,status=running)` | `.../main_menu/multiplayer_menu.rs` campaign launch + `crates/robin_rs/src/game_session/mod.rs` session loop |
| H31 | process | baseline | the client presses quick-load under two seats: only the host may load a session, so the press is refused in the UI and nothing moves — no page, no mission end, and the 25-frame agreement holds. asserts: `hash-equal(peers=1,every=25)`; `final-operation(operation=level_in_progress)`; `exit(peers=0,1,status=running)` | `.../game_session/live_gameplay.rs` |
| H32 | process | fault | the host answers Completed and the client answers Restart: the host's decision ends the page on every seat, so the client's late Restart reaches a seat with no terminal page and is refused there — the session leaves on the campaign-exit snapshot the host's answer chose, and no restart snapshot is ever prepared. asserts: `presentation(peers=0,1,state=gone)`; `log(contains=opening debriefing exit_code=LevelInterrupted,at_least=1)`; `log(contains=snapshot transition prepared (CampaignExit),at_least=1)`; `log(contains=snapshot transition committed,at_least=1)`; `log(absent=snapshot transition prepared (Save),count=0)`; `exit(peers=0,1,status=zero)` | `.../game_session/terminal_debriefing.rs` |
| H33 | process | fault | the host ends a running mission with the legacy `WIN` console cheat: the cheat is staged as the same one replicated command `/mission-result` sends, so no seat finalizes achievements inside a rollback. Both seats show one debriefing page, acknowledge it, and the session publishes and commits exactly one campaign-exit snapshot. asserts: `log(absent=achievement effect after finalization,count=0)`; `log(absent=fatal runtime error,count=0)`; `presentation(peers=0,1,state=gone)`; `log(contains=opening debriefing exit_code=LevelSucceeded,at_least=1)`; `log(contains=snapshot transition prepared (CampaignExit),at_least=1)`; `log(contains=snapshot transition committed,at_least=1)`; `exit(peers=0,1,status=zero)` | `.../http_server/dispatch.rs` + `.../console_overlay.rs` |
| H34 | process | fault | the same with the host's `LOOSE` console cheat: both seats reach the debriefing, both acknowledge it, the campaign-exit snapshot commits, and neither seat is left reconnecting behind a host that waits. asserts: `log(absent=achievement effect after finalization,count=0)`; `presentation(peers=0,1,state=gone)`; `log(contains=opening debriefing exit_code=LevelFailed,at_least=1)`; `log(contains=snapshot transition prepared (CampaignExit),at_least=1)`; `log(contains=snapshot transition committed,at_least=1)`; `exit(peers=0,1,status=zero)` | `.../game_session/terminal_debriefing.rs` |
| H35 | process | fault | a client is killed and relaunched on the same install mid-mission: its joining key is the durable, domain-separated install identity, so the host reclaims the same seat and the client returns to `running` with hash agreement instead of exhausting its reconnect budget. asserts: `log(absent=giving up on reconnection,count=0)`; `barrier(name=begin-sim,state=released)`; `hash-equal(peers=1,every=25)`; `mission(peers=0,1,count=1)`; `exit(peers=0,1,status=running)` | `.../multiplayer/identity.rs` + `.../multiplayer/native/peer_sessions.rs` |
| H36 | process | baseline | a campaign host ends a won mission with `/mission-result`, both seats acknowledge the debriefing with Continue, and the single host-owned session hands the whole party to the next campaign mission instead of leaving each install on its own save: both seats enter the second mission together at equal hashes and keep running. asserts: `mission(peers=0,1,count=2)`; `hash-equal(peers=1,every=25)`; `exit(peers=0,1,status=running)` | `H30` + `crates/robin_rs/src/game_session/mod.rs` session loop |

The mission-end rows are H22–H29, H32–H34; H35 is the same-identity reclaim, H36 the campaign handoff: H21 is a core row about the same
page, and H30 opens a campaign session a second seat joins. The sketch in
`docs/slop/multiplayer-terminal-matrix.md` numbers the six terminal rows
H21–H26, one lower. Its H22 (host answers Completed, client answers Restart)
asks whose answer wins and then says the session restarts; both cannot hold,
and H32 settles it — the host's answer decides, so the session leaves and the
client's Restart is refused at a seat whose page the host already closed.

`asserts:` is the machine-checked part of each cell: the exact expectations the
row's JSON must declare, and `scripts/validation/scenario_corpus.py --check`
fails when one is missing. The prose before it states the same claim in words;
the part the assertion vocabulary cannot tokenise — an engine-byte comparison, a
discarded prediction generation, the reason a window is reachable — is proved by
the row's named test, whose existence the gate checks. So the prose may carry
detail the tokens do not, and may never carry an assertion the row's prover does
not make.

**Decision:** this matrix is the coverage contract, and every row names its
prover in the scenario file's `proves` object — a Rust test
(`crate::module::test_name`) for a `core` or `transport` row, this suite for a
`process` row. The corpus gate fails when a named test does not exist in the
tree, when it is `#[ignore]`d and so runs in no suite by default, when
`proves.layer` is not the layer this table gives the row, and when a cell's
`asserts:` clause is not what the row declares. It also resolves every `Hnn`
cell to a scenario file, so a cell cannot claim coverage the corpus lacks; only
the issue that decides a `#nn` outcome may add that scenario, and each
out-of-scope reason is one line with a code pointer.

## 2. Scenario format and fault vocabulary (#7)

One file per scenario in a corpus directory that #16 creates, plus
`defaults.json` (mission, seed, `expected_players`, peer count). A scenario is
deltas and fixed verbs: a new verb bumps `schema` and changes the runner, never a
scenario. Schema 2 added the campaign dimension (`campaign`) and one assertion
(`mission`); schema 4 added the `console` fault verb for the legacy mission-end
cheats; every earlier row is unchanged in meaning.

The corpus file `H13.json` is the worked example: a transport row, its prover,
one fault and two assertions.

```json
{ "schema": 1, "id": "H13",
  "proves": { "layer": "transport", "tests": [
      "robin_rs::host::transport::transport_lifecycle_tests::committed_payload_is_consumed_once_and_keeps_simulation_held" ] },
  "peers": [ { "seat": 0, "role": "host" }, { "seat": 1, "role": "client" } ],
  "faults": [ { "at": { "barrier": "transition-commit", "after": 0 }, "peer": 1,
      "fault": "drop", "message": "SnapshotTransitionReady" } ],
  "expect": [ { "assert": "transition", "payload": { "CampaignExit": "level_succeeded" },
      "acked": [0, 1], "committed": false },
    { "assert": "transition-id-equal", "peers": [0, 1] } ] }
```

`proves` is the binding section 1 requires: `{"layer": "core"|"transport",
"tests": ["<crate>::<module path>::<test name>", …]}` names the Rust tests that
execute the row, or `{"layer": "process", "runner":
"scripts/validation/multiplayer_e2e.py"}` names this suite. `proves.layer` must
be the layer section 1 gives the row, and a `log` or `exit` assertion (or
`requires: ["graphical"]`) belongs only to a `process` row: those are the
assertions no Rust test makes, so a row that needs one is proved by the suite.

`campaign: true` on a scenario is the host's launch being a campaign
(`--mp-campaign`): one session over the campaign's missions, a fixed party, and
no local save on the joining seat. Only the host names it — a joiner learns the
campaign from the lobby it joined, which is what lets a second seat join a
campaign it has no save for.

`extends` runs the parent's `expect` list first: composition, not copying. `at`
is `{"frame": N}` in the issuing peer's numbering or `{"barrier": name,
"after": n}`; `peers[].start` is `"launch"`, `{"join_at_frame": N}` or
`{"join_at_barrier": name}`; `requires` lists `game-data`, `graphical` or
`netem` — `netem` names a driver this suite does not have yet, so a row that
needs one is refused by name and the suite fails until it exists.
`expect[].assert` is `hash-equal`, `hash-diverged`, `barrier`
(`reached|released|absent`), `transition`, `transition-id-equal`, `modal`, `log`
(`contains|absent`, and exactly one of `count` or `at_least`), `final-operation`
(a `GameCode::wire_name` from `crates/robin_engine/src/game_operation.rs`),
`presentation`, `count`, `mission` or `exit`. `presentation` states that the
seats named by `peers` showed a terminal page (`shown`, optionally of a named
`kind`) or showed one and left it (`gone`); a page that never appeared is not
`gone`, so the assertion cannot pass for a session that skipped the debriefing.
Transition ids are generated per run, so `transition-id-equal` compares peers
instead of pinning one. `count` on a log line is exact; `at_least` is its
floor, for a line printed once per retry — a refused handshake prints its reason
on every attempt, so that line's exact number is a retry schedule, not a
contract.

`hash-equal`'s `every` is read off consecutive `/state` samples'
`last_compared_hash_frame`, not from the engine: it needs at least two compared
frames, and a step wider than `every` passes only when the peer advanced far
enough that the samples could not have seen the boundary between; the
exact-boundary comparison itself is what the row's named Rust test pins. Only a
seat that receives hashes compares: the authoritative host publishes
`StateHash` and a client's publication of one is refused as host-only, so a
host's field is always null and a process row names the joining seats —
`hash-equal(peers=0,1,…)` is a claim no process run can settle.

`transition.payload` is the `SnapshotTransitionPayload` variant the host
committed — `"Save"` or `{"CampaignExit": "<GameCode wire name>"}` — because the exit code
is the part two terminal rows differ in; `"LoadRestart"` is a debriefing action,
not a transition. `count` is how a row states a cardinality claim without
reading log wording: `{"assert": "count", "subject": "barrier"|"transition"|
"operation"|"reconnect", "name": …, "times": n}`, where `reconnect` counts
`Reconnect` frames for a seat and `operation` counts a `GameCode` dispatch.
`seat` states what a seat holds after the run: `{"assert": "seat", "seat": 1,
"selected": ["<character id>"], "owns": true|false}`, which is how a preserved
selection or a refused late joiner is asserted. `exit` states how a seat's
process ended — `zero`, `nonzero`, or `running`, the last being a peer still
there, which is how a row says the host survived a fault: `/state` samples stop
when the process does, so they cannot on their own. `mission` states how many
missions each named seat has entered and that they are together: `{"assert":
"mission", "peers": [0, 1], "count": 1}`, where a mission is one distinct
`/state.map` the seat was in, so two installs on two saves fail it even when
each one played two missions.

| verb | core | transport | process |
| --- | --- | --- | --- |
| `kill` | drop channels, inject `Disconnected` | shut the client runtime | `SIGKILL` |
| `stop` / `continue` | hold the link queue | park the bridge | `SIGSTOP` / `SIGCONT` |
| `duplicate` | deliver the event twice | re-send the last `NetMsg` | `netem duplicate` |
| `drop` | discard the event | discard the frame | `netem loss 100%` window |
| `reorder` | swap two queued events | swap two frames | `netem reorder` + `delay` |
| `delay` | hold the event `ms` | hold the frame `ms` | `netem delay` |
| `diverge` | mutate the peer engine directly | unsupported | `POST /native` on that peer alone |
| `truncate` | cut the serialized event at `bytes` | cut the frame at `bytes` | unsupported |
| `restart` | rebuild the peer from its `PeerOwner` | restart the client runtime with a fresh state | `SIGKILL` then relaunch as a new process |
| `stall-surface` | hold the modal surface open past its deadline | same | same |
| `mission-result` | unsupported | unsupported | `POST /mission-result` on the host |
| `console` | unsupported | unsupported | `POST /console {"command": "..."}` on that peer |
| `command` | deliver the `PlayerCommand` to the peer | unsupported | `POST /command` on that peer |
| `native` | call the native on the peer | unsupported | `POST /native` on that peer |
| `key` | unsupported | unsupported | that keysym on the peer's own window |

A scenario may also end the mission and answer the page it opens, which is the
schema-4 half of the vocabulary (`campaign` and `mission` came with schema 2;
`console` came with schema 4). `mission-result` posts the decision on the host and
nothing else: the command it queues is replicated, so both seats end the same
mission at the same frame. A `resolve` list — not a fault — names a seat and a
`DialogResult`; the runner reads that seat's own `/state.presentation.kind` and
posts the typed dismissal on `/step-forward`, so it never guesses the
debriefing's text index and never presses a page the session already closed by
authority. The list is a sequence: a step whose seat has no page yet holds every
later one, because the first decision ends the page on every seat and a later
answer would only ever be refused. `settle_deadline_s` says how long a row may wait for a promise the
game reaches on its own schedule (the modal acknowledgement budget is minutes);
it extends only that row's settling window, never the suite's.

`key` is how a row reaches a surface that is only a window: the save keys are
`F1` (quick save) and `F5` (quick load), and each peer owns its own X server so
a key press belongs to exactly one seat. In a standalone multiplayer mission
there is no campaign to save, so the host's quick save is the executor's local
diagnostic and quick load finds no quick slot; a loadable quick save exists only
in a campaign co-op session, which starts from the lobby and has no developer
launch. H29 and H30 therefore prove the save surface and the two-seat liveness
around it, and the snapshot adoption a real quick load performs is proven by the
restart transfer H23 and H24 drive — the same
`begin_multiplayer_snapshot_transition` → validate → adopt code path.

The page a mission end leaves behind is not the last surface: the debriefing
completes into the mission-end leaderboard, which with no leaderboard endpoint
shows an error notice that needs an OK before the mission-end handoff finishes.
The driver keeps pressing Return for a stalled frame, but stops after two
post-debriefing attempts: enough to release that notice, not enough to answer
anything a later surface asks. A row that wants a page answered presses it
through `resolve`, never through Return.

These rows run a forced mission (`--mission`), not the campaign lobby, and the
two handoffs behave differently there. A mission that *ends* (Completed over an
aborted, won or lost mission) finishes the forced run: the host exits with the
mission's code, and the client, whose transport generation the committed
snapshot already ended, leaves on the debriefing's unconfirmed-end budget
(`SNAPSHOT_CONFIRM_BUDGET`) instead of reconnecting to an endpoint no process
serves — both processes leaving is how H22, H27 and H28 prove no seat hung. A
*restart* is a snapshot load, so the direct loop re-enters the same mission in
the same process with its transport intact: H23, H24 and H25 watch agreement
resume on the replacement generation instead. A campaign co-op session, which
would hand over to the next mission on the same transport, has no developer
launch (as above); the snapshot handoff itself is what these rows prove.

`drop`, `duplicate`, `reorder` and `delay` name a `NetMsg` variant in `message`;
`truncate` also names `bytes`; `restart` names whether the seat keeps its
identity (`"identity": "same"|"fresh"`), which is what separates a reconnect from
a stranger claiming a seat; the others name a peer. "This peer is one frame behind on purpose" is
`{"fault": "delay", "message": "Input", "ms": 40}`, not a new concept.

A recorded session exports into the same file from the host replay: `FrameRecord`
carries `timeline_after`, the seat-tagged `SimCommand` list, `host_controls` and
the per-second hash (`crates/robin_engine/src/replay.rs`). One `timeline` entry
per command at its `timeline_after`, one `hash-equal` per recorded hash, one
`transition` per `sv`/`lb` marker, empty `faults`; peer logs add `log`
expectations only for lines both peers emitted. A scenario stays smaller than
the code because it names barriers, faults and assertions rather than steps.

**Decision:** this schema is the only scenario input. The runner rejects an
unknown field, an unknown fault verb, an unimplemented `expect` verb and a fault
verb its layer cannot express, and a recorded export enters the corpus only after
it re-simulates to its own recorded hashes. `proves.layer` and section 1's
layer must agree, and the corpus gate reads both rather than guessing from the
verbs: a comparison of the two is what makes a row's "runs in" claim true.

## 3. How a scenario drives a session (#17)

Three layers exist and the harness grows on them. **core**: `Peer`s over real
`NetChannels` and `HostTransport::test_session`, no sockets or game data
(`.../game_session/multiplayer/story_regression.rs`). **transport**: real iroh
endpoints in one process, `start_server_with_key` plus `connect_client` on
ephemeral keys (`crates/robin_rs/src/multiplayer.rs`). **process**: real `robin`
binaries in a loopback namespace over the HTTP API
(`scripts/validation/multiplayer_e2e.py`).

Core owns what is a pure function of ordered messages: input scheduling and
rollback, hash comparison, seat ordering, modal decisions, snapshot transitions,
admission. Transport owns handshake, seat claim, content transfer and framing.
Process owns only what a process boundary shows: per-install state, real signal
death, graphical modal surface, stderr evidence, host exit.

Two facts decide the boundary. `POST /step-forward` with
`synchronized_multiplayer` does not lockstep peers: it advances the host, then
calls `reconnect_all_for_snapshot` and re-admits every peer from the new snapshot
(`crates/robin_rs/src/game_session/tick.rs`), so it reaches a fault window but
cannot prove scheduling or late-input behaviour. And admission was once
observable only through stderr text; `/state` now carries the `multiplayer`
object below (`crates/robin_rs/src/http_server/diagnostics.rs`), which is what
this layer reads where it can and what the `log` assertions cover where it
cannot.

**Decision:** a scenario names its layer in `proves`, and section 1 owns that
name. The suite runs only the `process` rows and refuses to load one whose faults
it cannot express rather than downgrading silently; a `core` or `transport` row
runs nowhere here and names the Rust test that executes it instead. `log` and
`exit` assertions and `requires: ["graphical"]` are the process layer's alone, so
a row carrying one must be a `process` row. `diverge` needs core or process;
content assertions need transport or process. `/state` also grows a
`multiplayer` object — `local_seat`, `expected_players`, `admission`,
`begin_sim_frame`, `pending_transition {id, payload, awaiting}`,
`last_compared_hash_frame`, `desync_count`, `final_operation` and the host's
`connect_string` — assembled where `Game` and the transport are both in scope, so
no assertion reads log wording.

## 4. Per-install state isolation (#18)

Per peer today: `save`, `config`, `cache`, `data`, `runtime`, `cwd` under the
evidence root plus a private HTTP port
(`scripts/validation/multiplayer_e2e.py`). That separates Linux peers only
because `dirs::data_dir()` honours `XDG_DATA_HOME`; Windows has no such
variable, and only the save root has an override.

| Path | Owner | Honours today |
| --- | --- | --- |
| saves, `profiles.json`, `keyconfigs.json`, `language.json` | `save_file::default_save_directory` | `ROBINHOOD_SAVE_DIR` |
| `multiplayer_identity.key` + `.lock` | `native_game_identity::identity_key_path` | `ROBINHOOD_SAVE_DIR` |
| `replays/attempts/*`, `replays/*.mission` | `mission_replays::native::default_directory`, `game_session::replay_init::default_replay_path` | nothing |
| `reports/*` | `bug_report::directory` | nothing |
| `datadir.txt` | `datadir_locator` | nothing |
| mods root | `mod_pack::default_mods_root` | `ROBINHOOD_MODS_DIR`, else the data dir's parent |

Two peers on one Windows account share replays, reports and one transport
identity: silently one owner.

**Decision:** every per-install path resolves through one function and honours one
new variable, `ROBINHOOD_INSTALL_HOME`, replacing `dirs::data_dir()/robin_hood` as
the parent of saves, identity, replays, attempts, reports and `datadir.txt`;
`ROBINHOOD_SAVE_DIR` keeps overriding the save root alone, and
`ROBINHOOD_MODS_DIR` is always set explicitly. Isolation is enforced, not
documented: a source test fails when any module outside the resolver calls
`dirs::data_dir()`, and the process runner lists each default location before and
after a run and fails the scenario if either changed.

## 5. Fault-injected runs: site and failure reporting (#19)

**Decision:** the process layer runs only on the provisioned Linux build host,
in one loopback-only user/network namespace around the whole run — never on a
GitHub-hosted runner, where `native-lifecycle` also does not run
(`docs/TESTING.md`). Core and transport rows are proved by the Rust tests their
`proves` names: a core row may name an `engine` test, and every transport row
names a `client-release` test, the only suite that compiles the transport
(`crates/robin_rs/Cargo.toml`, `default = []`).

Exit status: **0** every `expect` observed; **1** an assertion failed, or a
selected row was refused because the suite cannot express its faults — either is
a harness gap named in `summary.json`; **2** the deadline hit, i.e. a hang
including a peer that never returns; **3** a prerequisite is missing and nothing
about the game was proved. The drivers collapse a harness error and a game
failure into 1; the corpus separates them because a broken machine must never
read as a broken game.

Evidence per run at `<evidence>/<scenario-id>/<run>/`: the expanded scenario,
`summary.json`, `events.jsonl`, one log per peer, `hashes.json` — one entry per
`/state` sample's compared frame and desync count, plus every mission recording
the host install holds under its `replays/` (directory, files, `mission.json`
SHA-256; a host killed at frame zero wrote none, and a run that re-entered its
mission wrote two) — and the binary's SHA-256. The harness names no `--record`
path because a named directory holds one mission: the recorder's own
`replays/<stamp>.mission` naming is what lets a second attempt exist at all. A
non-empty directory is refused, as
`scripts/validation/lifecycle_gate.py` already does. Each run is bounded twice,
by the scenario's `budget_s` (default 600) and by the harness alarm; a barrier
wait is bounded by the transition-watchdog deadline (#4) plus a margin, so exit
2 means a promised state was missed, never that the machine was slow. After the
last fault a row is read one evidence window later, and a pending `exit` is
never settled by silence: a dropped session is not noticed until the transport's
idle timeout expires, then retries for the client's `RECONNECT_BUDGET` before it
publishes the fatal that ends the process, so the settle deadline covers both
plus the capped backoff and the same margin. A row settles on its positive
promises — a page, a barrier, a final operation and a hash cadence, which only
counts once two compared frames were watched — or, with none left to wait for,
on a quiet session. A row that promises only agreement and a live session would
otherwise run to its deadline and read as a hang.

Faults that cannot run there: Windows-specific ones (WinDivert duplication,
Known Folder paths, MSVC replay hashes), faults needing a real WAN (NAT rebinding,
relay failure), graphical modal faults without an X server, and every `netem`
verb (`duplicate`, `drop`, `reorder`, `delay`): the suite has no `netem` driver,
so a row that needs one is refused by name, listed under `refused` in
`summary.json` with its reason, and turns the suite red. The first three are
declared in `requires` and exit 3.

## 6. Hermetic bootstrap (#20)

A run needs a pinned `robin` binary, game data under `ROBINHOOD_DATA_DIR`
containing `Data/`, `assets/core-datadir` staged into each peer's working
directory and, for the process layer, `unshare`, `ip`, `Xvfb` and `python-xlib`.
Nothing installs anything: `lifecycle_gate.py` fails on a missing executable or
environment entry instead of skipping.

Peers never discover each other. `connect_string()` is a literal endpoint address
and `parse_connect_addr` accepts that JSON directly
(`crates/robin_rs/src/multiplayer/identity.rs`), so a dial needs no n0 DNS, no
Mainline DHT and no relay. Matching a peer by an address it was handed is not the
same as a hermetic bind, though: every native endpoint is built from
`presets::N0` with a DHT address lookup
(`.../multiplayer/identity.rs::bind_endpoint_with_relay`), and
`--mp-browser-join-links false` only skips the relay-readiness wait and the
invitation, not the preset. Matchmaking and `--join` stay outside the harness.

**Decision:** the harness bootstraps only from a pinned binary, a game-data root,
the staged core overlay and the host's advertised endpoint address, and no
scenario may invoke matchmaking or `--join`. Because the N0 preset and the DHT
lookup are unconditional today, the run may assert only that the join used the
advertised literal address; asserting socket-level isolation needs a hermetic
bind mode — relay disabled and no address lookup, selected by an explicit flag —
which this decision requires and which the bootstrap check then verifies by
failing if any relay URL or lookup is configured. Any missing prerequisite —
data, overlay, binary, namespace tool, X server — exits 3 naming the item before
a peer launches. There is no skip path: a scenario that cannot bootstrap fails,
and never passes.
