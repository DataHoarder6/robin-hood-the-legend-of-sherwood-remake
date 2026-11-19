# Multiplayer waits without a deadline

Audit of every place one participant's progress depends on a message another
participant may never send. Read-only: no production code was changed.

Last verified: 2026-11-19, `fork/main` @ `ed1a378fe`. Written against
`5e1d2b17e`; ranks 2 and 3 were bounded before `af1f3f779` (`45fc7fb96`,
`9ff945a46`, `05bca0b90`) and the earlier re-check of this file said otherwise,
so their rows now sit in "Waits that do have deadlines" and the ranked entries
state the new behaviour. Each open ranked item is closed by a change that gives
the wait a budget and a named consequence, never by a note in this file.

Scope: `crates/robin_rs/src/game_session/terminal_debriefing.rs`,
`crates/robin_rs/src/multiplayer/native/{server.rs,server_protocol.rs,peer_sessions.rs}`,
`crates/robin_rs/src/multiplayer/client_session.rs`,
`crates/robin_rs/src/game_session/multiplayer.rs`,
`crates/robin_rs/src/game_session/runtime/{admission.rs,multiplayer_sync.rs,timing.rs}`,
`crates/robin_engine/src/multiplayer/{.rs,modal_session.rs}`,
`crates/robin_rs/src/multiplayer/{content_transfer.rs,matchmaking.rs,rendezvous.rs}`,
`crates/robin_rs/src/distributed_mod/admission/*`.

Two labels are used below and are not interchangeable:

- **peer wait** — the local side cannot proceed until another process sends
  something. These are the permanent-hang candidates.
- **local wait** — bounded by a local channel closing, a local UI action, or a
  local worker. A hung local worker still hangs the game, but no remote peer is
  involved.

Bare file names in the tables expand as follows; every other reference is
repo-relative.

| Short name | Path |
| --- | --- |
| `terminal_debriefing.rs` | `crates/robin_rs/src/game_session/terminal_debriefing.rs` |
| `multiplayer.rs` (game) | `crates/robin_rs/src/game_session/multiplayer.rs` |
| `input.rs` | `crates/robin_rs/src/game_session/frame_prepare/input.rs` |
| `admission.rs`, `multiplayer_sync.rs`, `timing.rs` | `crates/robin_rs/src/game_session/runtime/` |
| `client_session.rs`, `matchmaking.rs`, `rendezvous.rs`, `native.rs`, `wasm.rs`, `content_transfer.rs` | `crates/robin_rs/src/multiplayer/` |
| `server.rs`, `server_dispatch.rs`, `server_protocol.rs`, `peer_sessions.rs` | `crates/robin_rs/src/multiplayer/native/` |
| `client.rs` | `crates/robin_rs/src/multiplayer/native/client.rs` |
| `session_setup.rs` | `crates/robin_rs/src/game_session/multiplayer/session_setup.rs` |
| `modal_session.rs`, `multiplayer.rs` (engine) | `crates/robin_engine/src/multiplayer/` |
| `identity.rs` | `crates/robin_rs/src/multiplayer/identity.rs` |
| `modal_net.rs` | `crates/robin_rs/src/ingame_menu/modal_net.rs` |
| `multiplayer_menu.rs` | `crates/robin_rs/src/main_menu/multiplayer_menu.rs` |
| `cli.rs` | `crates/robin_rs/src/main_entry/cli.rs` |
| `http.rs` | `crates/robin_rs/src/leaderboard/http.rs` |
| `native.rs`, `workflow.rs` (admission) | `crates/robin_rs/src/distributed_mod/admission/` |
| `transport.rs` (host) | `crates/robin_rs/src/host/transport.rs` |

## Waits with no deadline

| Wait | Evidence | Trigger | Timeout | Action when it expires / the peer vanishes | Per-player state logged or shown | Consequence if it never resolves |
| --- | --- | --- | --- | --- | --- | --- |
| Host terminal debriefing holds for its own snapshot transition commit (`TerminalDebriefingPhase::AwaitingSnapshot`) | `terminal_debriefing.rs` (`await_remote_snapshot`, the `AwaitingSnapshot` arm of `tick`) | Host finished the debriefing leaderboard, published `BeginSnapshotTransition`, and at least one connected peer is still in `awaiting` | **30 s** (`SNAPSHOT_CONFIRM_BUDGET`) | Leaves the session: `TerminalDebriefingProgress::SnapshotUnconfirmed` takes the same exit as an emergency end, so the campaign loop reopens the menu instead of freezing the mission | A `session_failed` event names the wait and the budget (`waited_ms`, `budget_ms`); the run log carries the same wording | Closed 2026-11-19. The wait can no longer outlive its budget; the host barrier it waits on is bounded separately (`SnapshotTransitions::awaiting`, below). |
| Client terminal debriefing holds for the host's snapshot commit | `terminal_debriefing.rs` (`await_remote_snapshot`, the `AwaitingSnapshot` arm of `tick`) | Client finished its debriefing with outcome `Ok`/`Load` | **30 s** (`SNAPSHOT_CONFIRM_BUDGET`) | Leaves the session with `GameCode::Quit` and reopens the menu; it does not keep waiting for a host that may never answer | The same `session_failed` event and log line as the host row | Closed 2026-11-19 with the host row: both sides of the same wait now expire. |
| Client waits for `BeginSim` | `runtime/admission.rs:14-22` (`PeerWaitingForBegin`, `HostWaitingForBegin`, `HostWaitingForResyncBegin`); gate `runtime/multiplayer_sync.rs:60-74`; pause `multiplayer.rs:1274` | Peer adopted the snapshot and sent `ReadyToSim`, or host's mission is loaded and waiting on peers | **none** | Nothing in the game layer. Only the transport notices a dead host. | No log for the wait itself. `BeginSim` arrival logs "begin-sim barrier released" (`multiplayer.rs:625-629`). | Mission loaded but never simulated; input disabled by `set_gameplay_input_enabled(false)` (`host/transport.rs:178`). |
| Client waits for `InitialSnapshot` after (re)connect | `runtime/admission.rs:50-57` (`PeerWaitingForSnapshot`) | `Disconnected`, or a first join before the host's snapshot cache exists | **none** | Nothing. The host skips the send silently when its `initial_snapshot` cache is empty (`server.rs:995-1010`). | Console: "Disconnected; reconnecting…" and "Reconnected; synchronizing game…" (`multiplayer.rs:519-551`). No indication that the snapshot never arrived. | Peer frozen indefinitely on a snapshot it never receives; reconnect loops never converge. |
| Story / terminal modal acknowledgement barrier | `modal_session.rs:19-22` (`requires_modal_acknowledgements`), `:632-671` (votes), `:828-845` (unanimous over the roster), `:751-761` (`story_barrier_pending`); pause `multiplayer.rs:1295-1303` | Any shared story modal, and any `FinalDebriefing`/`MissionState EndState` page, while another seat is admitted | **none** | Nothing. The roster (`required_players`) is set once at `install_session` (`host/transport.rs:177`) and never shrinks when a seat leaves. | Host UI shows "Waiting for Player 2…" from `modal_waiting_names` (`modal_net.rs:159-164`) for scripted story modals only. The terminal debriefing uses `ModalDismissalGate` (`terminal_debriefing.rs:224-225`, `modal_net.rs:16-103`), which shows no waiting text at all. | Simulation paused for every participant. One player who never opens the page (or left the session) freezes everyone until they return. |
| Host waits for a peer's modal decision (debriefing popup and final page) | `terminal_debriefing.rs:628-648` (`AwaitingMissionAuthority`, `AwaitingFinalAuthority`), `modal_net.rs:58-103` | Host clicked continue on a debriefing page that requires acknowledgements | **none** | Nothing; the gate stays `AwaitingAuthority` and retries publication each UI tick. | None on screen; `tracing::error!` only when publication itself fails (`modal_net.rs:105-110`). | Host debriefing hangs with the page still open. Combined with the two rows above, a peer that disconnects at mission end deadlocks both sides. |
| Mission-end debriefing waits for its own command echo | `terminal_debriefing.rs:1105-1112` (`pending_terminal_debriefing` re-queued while `mission_attempt_sequence` has not advanced) | After the engine returns a terminal exit code, in any multiplayer mission | **none** | Nothing. The echo needs frames to advance; `pause_simulation` is true while `transport.reconnecting()` (`multiplayer.rs:1274`), so a disconnect at mission end stops the very echo being awaited. | "waiting for authoritative mission completion update" is logged once (`terminal_debriefing.rs:1147-1152`). | Frozen frame with no debriefing UI. Feeds the modal deadlock above. |
| Client reconnect loop (`SessionEnd::Drop`) | `client_session.rs:718-840`; backoff constants `:51-52` | Any transport loss | **no cap** — "reconnecting with exponential backoff" never gives up; `RECONNECT_BACKOFF = 500 ms`, `MAX_RECONNECT_BACKOFF = 10 s` | Each attempt is bounded (native), then sleeps and retries forever. No attempt limit, no "host gone" verdict. | Warn per drop: `client session ended: {reason}; reconnecting...`, plus the console line. | Client is unusable indefinitely if the host never comes back. Nothing returns the player to the main menu. |
| Browser reconnect attempt | `wasm.rs:192` (`reconnect_attempt: None`), used at `client_session.rs:762` | Browser transport loss | **none** — `None` means "the phase is unbounded apart from cancellation" (`client_session.rs:139-141`) | Nothing; the whole attempt (connect + Hello + Welcome read, `handshake_frame: None` too) can wait forever, so the loop above never even reaches its backoff. | Same drop warnings, none for the hung attempt. | Browser peer wedges inside one reconnect attempt with no UI feedback. |
| Client waits for the host's frame schedule | `game_session/multiplayer.rs:1243-1272`; `timing.rs:33-64` | Peer has no (or a stale) `StateHash` schedule; the schedule is the only clock the peer accepts | **none** | `clock_pause = true`, so simulation is held. `prediction_limit_reached` caps how far it may run ahead, not how long it may wait. | Throttled once per second: "multiplayer: waiting for host frame schedule" with `scheduled_frame`, `until_frame_ms` (`multiplayer.rs:1257-1267`). | Peer's world stops advancing while the app stays responsive. Only the transport's own loss detection ever breaks it. |
| Host waits for a peer's late-input resync | `game_session/multiplayer.rs:1140-1176` (`HostResynchronizing` + `send_ready_to_sim`), `:1113-1125` | A peer input arrived below the rewind horizon, or a client rollback failed | **none** | Host sets the resync admission state and waits for the same `ReadyBarrier` described above. | "multiplayer: waiting for authoritative rollback recovery" / "dropping every peer for full-snapshot resynchronization" (`server_dispatch.rs:239-251`). | Host frozen at the resync boundary; the barrier can only be satisfied by the peers it just dropped. |
| Peer stream read has no application deadline | `server.rs:1333` (peer reader `read_frame` with no deadline), `client_session.rs:862` (client reader likewise) | A stream stays open with no traffic (host wedged, NAT blackhole without QUIC loss detection) | **none at this layer** | Nothing. Detection is delegated entirely to QUIC/Iroh defaults; the endpoint is built from `presets::N0` with no `TransportConfig` override (`multiplayer/identity.rs:85-95`). | None. | Both sides believe the session is alive. See "Unclassified" — the effective QUIC idle limit was not verified on this host. |
| Native content admission, lobby path | `distributed_mod/admission/native.rs:35-36` (`OFFER_WAIT = None`, `TRANSFER_WAIT = None`); loop `workflow.rs:86-136` | Joining a host whose distributed mod the peer does not have, on native | **none** in this loop | `enforce_deadline` is a no-op when the budget is `None`; the loop polls every 10 ms forever if chunks stop. | None in this loop; the transport task logs/errors separately. | Only indirectly bounded by the transport task's 30 s chunk idle (`native.rs:62`). Not proven for every path — see "Unclassified". |
| Browser/local content decision and readiness (local wait) | `native/client.rs:238-239` (`content_decision: None`, `content_readiness: None`), consumed at `client_session.rs:472`, `:517` | Host offered content that needs local approval or staging | **none** on native | Nothing; the admission task waits for the game loop to send `ContentRequest` / `ContentReady`. Closes only if the outgoing channel is dropped. | None. | A swallowed consent dialog stalls mission start. Local, not peer-driven, but unbounded. |
| Matchmaking: joined player waits for the host's `Start` | `main_menu/multiplayer_menu.rs:478-491`; gossip `matchmaking.rs:720-840` | Browser/native lobby join where the host has not pressed Start | **none** (status text only) | Nothing; "Joined game. Waiting for host to start..." stays until a `Started` message arrives. | Status line only. | Lobby wait; user-cancellable. No mission-side hang. |

## Waits that do have deadlines

These are the boundaries that keep the table above from being wider. Values are
the constants the transports actually read, not comments.

| Wait | Value | Evidence |
| --- | --- | --- |
| Whole initial connect attempt | 15 s (checked between attempts) | `client_session.rs:48` `INITIAL_CONNECT_TIMEOUT`; enforced `:400-405` |
| Native handshake frame / initial attempt / reconnect attempt / post-content Welcome | 15 s each | `native.rs:59`; `native/client.rs:230-234` |
| Browser initial attempt | 5 s | `wasm.rs:37`; `wasm.rs:191` |
| Browser post-content Welcome, content chunk idle | 30 s | `wasm.rs:38`; `wasm.rs:193-195` |
| Native content chunk idle | 30 s | `native.rs:62` |
| Native content transfer budget | 30 min (the constant is named `CONTENT_DECISION_TIMEOUT`) | `native.rs:63`; `native/client.rs:237` |
| Browser content transfer / content decision | 15 min / 30 min | `wasm.rs:39-40` |
| Browser content readiness | 5 min | `wasm.rs:40`; `wasm.rs:198` |
| Server accept + game-stream open + first frame | 15 s each | `server.rs:818-836`; `native.rs:59` |
| Peer writer drain after reader authority loss | 15 s | `native.rs:61`; `server.rs:1307-1325` |
| Server relay reachability check | 15 s | `server.rs:614` |
| Browser relay online | 15 s | `wasm.rs:36` |
| Browser Welcome / content-offer waits in mission setup | 15 s each; 10 s for metadata | `session_setup.rs:66-79`, `:104-111`, `:124-141` |
| Native `AssignedLocalSeat` wait in mission setup | 2 s (logs and continues; it does not abort) | `session_setup.rs:194-212` |
| Direct-invite probe, consent confirmation, preflight | 15 s / 5 s / 5 s | `multiplayer_menu.rs:1865`, `:2006`, `:2113` |
| DHT slot read / announce | 15 s each | `rendezvous.rs:39` |
| Leaderboard HTTP request | 15 s | `leaderboard/http.rs:13` |
| Matchmaking Start debounce (gives an in-flight Join time to arrive) | 750 ms | `matchmaking.rs:818-824` |
| Matchmaking listing soft TTL | 60 s | `matchmaking.rs:36` |
| Synchronized start gate once `BeginSim` arrives | host clock + 500 ms | `server.rs:287-292`; opened at `multiplayer_sync.rs:61-69` |
| Host snapshot-transition acknowledgement barrier (`SnapshotTransitions::awaiting`) | 60 s + 1 s per 64 KiB of payload | `robin_engine/src/multiplayer.rs:260` (`TRANSITION_COMMIT_MS`), `:286` (`transition_budget`); stamped `server_dispatch.rs:283-287`; expired on the 500 ms tick `server.rs:866-874` |
| Host ready barrier (`ReadyBarrier::candidate`) | 120 s, refreshed once for a seat admitted mid-wait | `server_protocol.rs:23` (`READY_BUDGET_MS`), `:89-104` (wait start, one extension, expiry); expired on the same tick `server.rs:316-327` |

The synchronized start gate and the two host barriers above resolve on a clock.
Every remaining row in the first table waits for a message that may never
arrive.

The two host barriers expire into the same shape, so it is stated once: the
seats still waiting are evicted, one `tracing::warn!` names the transition (or
the evicted seats) and the time waited, and the host proceeds with the
participants that did answer (`server.rs:866-874` for a transition, `:316-327`
for readiness). An evicted seat keeps its authenticated owner and re-admits
from an authoritative snapshot like any late joiner; a message it sends after
eviction is dropped as
`PeerDispatchFailure::Inactive` (`server.rs:208-218`, `peer_sessions.rs:78-80`),
so a late ack cannot resurrect a barrier that already committed. The transition
deadline is stamped at prepare time, so a disconnect that calls
`retain_for_reconnect` (`server_protocol.rs:231-236`) re-adds the seat but
cannot push the deadline out: the deadline is absolute from prepare time, and
the tick that expires the barrier commits it under the same lock, after which
`retain_for_reconnect` finds no pending transition and does nothing.

## Ranked: most likely to produce a permanent hang

1. **Terminal debriefing `AwaitingSnapshot`, both sides — closed 2026-11-19
   (`SNAPSHOT_CONFIRM_BUDGET`).** It is released by
   `take_committed_snapshot_transition` (`frame_prepare/input.rs:419-431`),
   which requires *every* awaiting seat to have acknowledged; that release now
   races a 30 s budget on both sides (`terminal_debriefing.rs`,
   `SNAPSHOT_CONFIRM_BUDGET`). Expiry settles the phase as
   `SnapshotUnconfirmed`, records a `session_failed` event naming the wait and
   the budget, and leaves the session through the same exit an emergency end
   takes, so the campaign loop reopens the menu instead of freezing with the
   mission pinned to `LevelInProgress` (`terminal_debriefing.rs`,
   `frame_simulate.rs`). The host barrier it waits on is bounded separately at
   rank 2.
2. **Host snapshot-transition acknowledgement barrier — closed 2026-11-18
   (`45fc7fb96`, `05bca0b90`).** The deadline is stamped at prepare time
   (`server_dispatch.rs:283-287`) from the payload's own budget —
   `TRANSITION_COMMIT_MS` plus one second per 64 KiB
   (`robin_engine/src/multiplayer.rs:260`, `:286`), the same number the peer
   enforces on the transition it prepared, so there is one budget and not two.
   The 500 ms watchdog tick (`server.rs:699`, `:856-874`) calls
   `SnapshotTransitions::expire` (`server_protocol.rs:215-229`), which drains
   the seats still `awaiting` into `answered` and logs one warning naming the
   transition id, the evicted seats, the waited time and the payload size; the
   same tick commits the transition for the peers that answered
   (`server.rs:253-292`). `retain_for_reconnect` still re-adds a seat on
   disconnect, but the deadline is absolute, so a reconnect buys no extra time,
   and once the tick has committed there is no pending transition left to
   resurrect. A late ack is a no-op twice over: while the transition is pending
   it matches `answered` (`server_protocol.rs:199-203`), and after the commit
   the seat's writer is detached, so dispatch returns
   `PeerDispatchFailure::Inactive` and the frame is dropped
   (`server.rs:208-218`, dispatched at `:1453`). Proved by
   `expired_transition_evicts_absent_seats_and_completes_the_barrier`
   (`server/tests.rs:785`), which commits once with the acks that arrived, and by
   `snapshot_tracker_rejects_wrong_acks_and_treats_duplicates_as_no_ops`
   (`server/tests.rs:747`), which H10 and H17 name.
3. **Ready barrier against a static `expected_players` — closed 2026-11-18
   (`9ff945a46`).** `ReadyBarrier` records when it first became blocked
   (`server_protocol.rs:42`, `:89-91`) and returns `Expired` after
   `READY_BUDGET_MS` = 120 s (`:23`, `:100-104`), evaluated on the same
   watchdog tick even when no `ReadyToSim` ever arrives. On expiry the host
   evicts the admitted seats that are unattached or not readiness-complete
   (`peer_sessions.rs:180-201`), logs one warning naming those seats, the
   budget and the waited time (`server.rs:316-327`), and releases with the
   host frame plus every ready frame that did arrive, quorum ignored
   (`server_protocol.rs:115-126`). A seat admitted after the wait began
   refreshes the budget exactly once (`:91-95`), so a late joiner still gets a
   full load window and a second joiner cannot extend it again. The release is
   still single: `commit` asserts `begun.is_none()` (`:128-131`) and a begun
   barrier answers `Wait` (`:68-70`), and a late `ReadyToSim` from the evicted
   seat is dropped as `Inactive` because its writer is gone
   (`peer_sessions.rs:78-80`, `server.rs:1544`). `--mp-expected-players`
   keeps its meaning as what the host waits for
   (`main_entry/cli.rs:246-248`); `/state`'s
   `multiplayer.expected_players` stays that configured count, and the warning
   names the seats that never arrived or never loaded, so the two are read
   together and the barrier is never silent about having fewer seats than the
   count. Proved by
   `ready_barrier_expires_after_its_budget_and_extends_once_for_a_joiner`
   (`server/tests.rs:472`) and
   `ready_barrier_eviction_drops_admission_and_writer_but_keeps_the_owner`
   (`peer_sessions.rs:509`), the pair H11 names.
4. **Modal acknowledgement barrier, including terminal summaries** —
   `modal_session.rs:19-22` makes `FinalDebriefing` and `MissionState EndState`
   require acknowledgements, `:837-845` requires every roster seat, and
   `multiplayer.rs:1303` turns a pending story barrier into
   `pause_simulation`. The roster never shrinks. Only scripted story modals
   show "Waiting for …" (`modal_net.rs:159-164`); the terminal pages show
   nothing, so the host sees a frozen mission with an open debriefing page.
5. **Client reconnect with no cap, and simulation paused throughout** —
   `client_session.rs:718-840` retries forever (500 ms → 10 s backoff) with no
   attempt limit, while `multiplayer.rs:1274` keeps `pause_simulation` true. The
   debriefing command echo below needs frames, and this pause is what withholds
   them, so a host that disappears at mission end deadlocks the client through
   two independent waits. On wasm the situation is worse:
   `reconnect_attempt: None` (`wasm.rs:192`) means a single attempt can hang
   forever.

Runner-up: **debriefing command echo** — `terminal_debriefing.rs:1105-1112`.
Cheap to hit whenever the mission ends during a reconnect, and it feeds
candidates 1 and 4.

## Adjacent defects found while tracing the waits

Not hangs, but they are what turns some of the above from "wait" into "hard
failure", so they belong with this evidence:

- A peer that reconnects while a transition is pending receives
  `PrepareSnapshotTransition` again, and used to fail the protocol assertion
  `!host.transport.has_snapshot_transition()` because its transport still held
  the earlier payload. A prepare now belongs to the transport generation that
  received it (`68e655837`): ending that generation calls
  `await_authoritative_snapshot`, which drops the prepare and names it in the
  log (`transport.rs:323-350`), so the re-sent prepare arrives against an empty
  slot and the peer acknowledges the same session-bound id (H17).
- `establish` on native reads `handle.content_offer()` without waiting
  (`session_setup.rs:63`); the value is only published by the I/O task that
  just started, so a slow first handshake can be mistaken for "no content".

## Unclassified

- **QUIC idle detection for a silent but open stream.** `server.rs:1333` and
  `client_session.rs:862` read frames with no deadline. Recovery therefore
  depends on the Iroh/QUIC idle timeout, and neither endpoint overrides
  `TransportConfig` (`multiplayer/identity.rs:85-95`). The effective value comes
  from `iroh 1.4`'s `presets::N0`; the crate source is not present on this host,
  so the number was not verified and is deliberately not stated here.
- **Native content admission (`TRANSFER_WAIT = None`).** The lobby loop
  (`distributed_mod/admission/workflow.rs:86-136`) has no deadline of its own.
  It appears bounded indirectly by the transport task's 30 s chunk idle, but
  that requires the transport task to be the only producer of `ContentChunk`
  events for that channel. Not proven for every entry point, so listed rather
  than ranked.
- **Whether a departed player can ever be removed from `required_players`.**
  Only `set_modal_player_count` writes it (`modal_session.rs:624-630`), and it
  is called from `install_session` only (`host/transport.rs:177`). A mid-session
  path that re-derives the roster from live seats was not found; absence of a
  caller is not proof that none exists, so the conclusion rests on the call
  sites listed.
