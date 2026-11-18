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

### Corpus

`baseline` runs first in every scenario of its row; `fault` is the cell's own
case; `invariant` and `vocabulary` are not cells.

| id | layer | role | must assert | extends |
| --- | --- | --- | --- | --- |
| H01 | core | baseline | the pair baseline: one `BeginSim` admits both peers with equal `state_hash` at a frame off the 25-frame boundary, and the hash oracle compares on the exact 25-frame boundary rather than against newer state. asserts: `hash-equal(peers=0,1,every=25)`; `barrier(name=begin-sim,state=released)`; `count(subject=barrier,name=begin-sim,times=1)` | `story_regression::pair` |
| H02 | core | fault | seat 1 owns the PC it selects; seat 0's selection of that PC is refused while a connected partner owns it in `Exclusive`. asserts: `seat(seat=1,owns=true)`; `seat(seat=0,owns=false)` | `robin_engine/src/engine/coop.rs` |
| H03 | core | baseline | `LevelSucceeded` → prepare/ready/commit `CampaignExit`; committed engine equals the host snapshot. asserts: `transition(payload=CampaignExit(LevelSucceeded),acked=0,1,committed=true)`; `final-operation(operation=LevelSucceeded)` | `completed_mission_snapshot_commits_the_hosts_campaign_before_rebuilding` |
| H04 | core | baseline | same for `LevelFailed`. asserts: `transition(payload=CampaignExit(LevelFailed),acked=0,1,committed=true)`; `final-operation(operation=LevelFailed)` | same test's failure arm |
| H05 | core | baseline | `LevelFailed` commits `CampaignExit(LevelFailed)`, the commit barrier completes once, and the peer re-adopts the host snapshot; `Restart`/`LoadRestart` is a debriefing action, not a transition. asserts: `transition(payload=CampaignExit(LevelFailed),acked=0,1,committed=true)`; `count(subject=barrier,name=transition-commit,times=1)`; `barrier(name=snapshot-adopt,state=reached)` | `completed_mission_snapshot_commits_the_hosts_campaign_before_rebuilding` + `current_snapshot_ack_commits_once_and_detaches_authority` |
| H06 | core | baseline | a host quit ends the session with the authoritative operation `Quit`. asserts: `final-operation(operation=Quit)` | `robin_rs/src/game.rs` |
| H07 | core | fault | after a drop the peer adopts the host snapshot and rejoins the baseline hash agreement under one `BeginSim`. asserts: `hash-equal(peers=0,1,every=25)`; `barrier(name=begin-sim,state=released)`; `count(subject=barrier,name=begin-sim,times=1)`; `barrier(name=snapshot-adopt,state=reached)` | `pair` + `Link::reconnect` |
| H08 | core | fault | a mutated peer is caught at the next compared frame and recovers to hash agreement from the host snapshot. asserts: `hash-diverged(peers=1)`; `barrier(name=snapshot-adopt,state=reached)`; `hash-equal(peers=1,every=25)` | `.../game_session/frame_prepare/pre_tick.rs` |
| H09 | core | vocabulary | duplicated `ModalProgress`/`ModalDecision` yields one decision per occurrence. asserts: `modal(count=1)`; `barrier(name=modal-ack,state=released)` | `modal_session_delayed_surface_and_delivery_matrix` |
| H10 | core | invariant | a transition's commit barrier completes exactly once. (The `SnapshotTransitions` tests this row names also cover the wrong-id, un-awaited-seat and duplicate-ack refusals.) asserts: `count(subject=barrier,name=transition-commit,times=1)` | `SnapshotTransitions` tests |
| H11 | transport | fault | a seat admitted with its content ready but never acknowledging `BeginSim` does not wedge the host: the ready barrier waits, extends once for the joiner, expires on its budget, evicts that seat (admission and writer dropped, owner kept for a re-admission) and releases on the frames that did arrive, quorum ignored. The process layer cannot schedule this drop: both pre-ack admission phases live between two `/state` samples, so a process row would assert a `reached` it can never observe. asserts: `barrier(name=begin-sim,state=released)` | `.../multiplayer/native/server.rs` + `.../multiplayer/native/peer_sessions.rs` |
| H12 | process | fault | connect to a closed endpoint → the client reports `Game loop failed: multiplayer: failed to connect to` and exits non-zero with no seat. Its `content-ready` absence is not asserted: the barrier is unsampleable, so the absence would hold whether or not the client had reached admission. asserts: `log(contains=Game loop failed: multiplayer: failed to connect to,count=1)`; `exit(peers=1,status=nonzero)` | `.../multiplayer/client_session.rs` |
| H13 | transport | fault | a mutated transition id or a lost ack never commits the transition, and both peers keep the same transition id. asserts: `transition(payload=CampaignExit(LevelSucceeded),acked=0,1,committed=false)`; `transition-id-equal(peers=0,1)` | `crates/robin_rs/src/host/transport.rs` |
| H14 | process | fault | a restarted native owner (`identity: "fresh"`) while its seat is parked and the roster full → the host refuses the claim (`… already has its configured 2 players`), exactly one admission is logged in the session, and the replacement exits non-zero. It never reads the rejection text: `reject_opening` writes a `Reject`, but the handler ends before the peer sees it and the client reports `read frame header: connection lost`, so decoding that frame belongs to a transport row. asserts: `log(contains=multiplayer session already has its configured 2 players,at_least=1)`; `log(contains=multiplayer client connected,count=1)`; `exit(peers=1,status=nonzero)` | `peer_sessions::claim_seat` |
| H15 | core | fault | a live client that settles the mission end does not complete locally: adoption of the host snapshot is still absent when the run is read. asserts: `barrier(name=snapshot-adopt,state=absent)` | `.../game_session/terminal_debriefing.rs` |
| H16 | core | fault | a mismatch seen during recovery starts no second reconnect. asserts: `barrier(name=snapshot-adopt,state=reached)`; `hash-equal(peers=1,every=25)`; `count(subject=reconnect,name=1,times=1)` | `.../game_session/frame_prepare/pre_tick.rs` |
| H17 | transport | fault | a transition pending when a seat reconnects completes with the same id on both peers once that seat acks again. asserts: `transition(payload=CampaignExit(LevelSucceeded),acked=0,1,committed=true)`; `transition-id-equal(peers=0,1)` | `.../multiplayer/native/server.rs` pending-Prepare resend |
| H18 | transport | fault | content admission needs no seat: a client reaches `content-ready` and is later admitted with a seat. asserts: `barrier(name=content-ready,state=reached)`; `seat(seat=1,owns=true)` | `host_content_preflight_uses_no_seat_then_exact_resume_reconnects` |
| H19 | process | fault | host exit with a seat admitted — the kill is anchored on the monotonic `begin_sim_frame`, the only fact that says the seat was admitted → that seat's reconnects are bounded: it gives up and exits non-zero within the client's reconnect budget (`RECONNECT_BUDGET`), and the process leaves well after the host died because the drop first waits out the transport's 30 s idle timeout; the content-transfer window itself stays a transport row. asserts: `log(contains=giving up on reconnection,count=1)`; `exit(peers=1,status=nonzero)` | `.../multiplayer/client_session.rs` |

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
scenario.

The corpus file `H13.json` is the worked example: a transport row, its prover,
one fault and two assertions.

```json
{ "schema": 1, "id": "H13",
  "proves": { "layer": "transport", "tests": [
      "robin_rs::host::transport::transport_lifecycle_tests::committed_payload_is_consumed_once_and_keeps_simulation_held" ] },
  "peers": [ { "seat": 0, "role": "host" }, { "seat": 1, "role": "client" } ],
  "faults": [ { "at": { "barrier": "transition-commit", "after": 0 }, "peer": 1,
      "fault": "drop", "message": "SnapshotTransitionReady" } ],
  "expect": [ { "assert": "transition", "payload": { "CampaignExit": "LevelSucceeded" },
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

`extends` runs the parent's `expect` list first: composition, not copying. `at`
is `{"frame": N}` in the issuing peer's numbering or `{"barrier": name,
"after": n}`; `peers[].start` is `"launch"`, `{"join_at_frame": N}` or
`{"join_at_barrier": name}`; `requires` lists `game-data`, `graphical` or
`netem` — `netem` names a driver this suite does not have yet, so a row that
needs one is refused by name and the suite fails until it exists. `expect[].assert` is `hash-equal`, `hash-diverged`,
`barrier` (`reached|released|absent`), `transition`, `transition-id-equal`,
`modal`, `log` (`contains|absent`, and exactly one of `count` or `at_least`),
`final-operation` (a `GameCode` name
from `crates/robin_engine/src/game_operation.rs`), `count` or `exit`. Transition
ids are generated per run, so `transition-id-equal` compares peers instead of
pinning one. `count` on a log line is exact; `at_least` is its floor, for a line
printed once per retry — a refused handshake prints its reason on every attempt,
so that line's exact number is a retry schedule, not a contract.

`transition.payload` is the `SnapshotTransitionPayload` variant the host
committed — `"Save"` or `{"CampaignExit": "<GameCode>"}` — because the exit code
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
when the process does, so they cannot on their own.

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
`/state` sample's compared frame and desync count, plus the host replay's path
and SHA-256 (or the fact that the host died before writing one) — and the
binary's SHA-256. A non-empty directory is refused, as
`scripts/validation/lifecycle_gate.py` already does. Each run is bounded twice,
by the scenario's `budget_s` (default 600) and by the harness alarm; a barrier
wait is bounded by the transition-watchdog deadline (#4) plus a margin, so exit
2 means a promised state was missed, never that the machine was slow. After the
last fault a row is read one evidence window later, and a pending `exit` is
never settled by silence: a dropped session is not noticed until the transport's
idle timeout expires, then retries for the client's `RECONNECT_BUDGET` before it
publishes the fatal that ends the process, so the settle deadline covers both
plus the capped backoff and the same margin.

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
