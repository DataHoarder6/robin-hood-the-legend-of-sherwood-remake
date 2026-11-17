# Bounded transition barriers in co-op games

Wayfinder ticket #14. Answers: how do shipped engines and co-op games structure
mission transitions, readiness barriers, join-in-progress and reconnect, and what
do they do when a peer misses a barrier — deadlines, safe states, player-visible
failure, telemetry.

Written 2026-11-17. Every external claim links its source; repo claims name the
file and function. Values are quoted from source at the linked revision.

## The defect, stated as a protocol problem

`Prepare`/`Ready`/`Commit` in this repo is two-phase commit over an unbounded
participant set (`docs/MULTIPLAYER.md`, "Modal authority and mission
transitions"). Three properties make a missed `Ready` a permanent stall:

- **No clock.** `SnapshotTransitions` holds only `pending`; `PendingSnapshotTransition`
  has `id`, `payload`, `awaiting` and no deadline
  (`crates/robin_rs/src/multiplayer/native/server_protocol.rs:55`,
  `:62`).
- **The wait set only grows.** `retain_for_reconnect`
  (`server_protocol.rs:104`) re-inserts the seat on disconnect, with the comment
  "Connectivity never shrinks the barrier, including after an ack"; a peer that
  leaves and never returns keeps its own commit blocked forever.
- **Duplicate and late acks are fatal, not idempotent.**
  `acknowledge` returns `RemoteProtocol` for "duplicated or was not expected"
  and "acknowledged no active snapshot transition"
  (`server_protocol.rs:79`). An ack in flight when `Commit` fires kills the
  server session instead of being dropped.

The client acks exactly once per `Prepare`, and a second `Prepare` while one is
pending is a protocol error
(`crates/robin_rs/src/game_session/multiplayer.rs:854`, ack at `:939`). There is
no re-send path for a `Ready` whose frame was lost in a transport rebuild, and
nothing in the state machine re-arms the ack. The modal lane has one —
`resend_pending_modal_proposals`
(`crates/robin_rs/src/game_session/multiplayer.rs:552`) — the transition lane
does not.

Compare the join barrier in the same file: `ReadyBarrier::candidate`
(`server_protocol.rs:24`) counts only *connected* seats and releases when
`connected >= expected_players - 1`. The join barrier is already
liveness-aware; the transition barrier is the only unanimity gate in the
protocol that cannot be satisfied by the surviving players.

This is textbook 2PC blocking. PostgreSQL documents the same hazard for
`PREPARE TRANSACTION`: the prepared transaction "continues to hold whatever
locks it held", and the documented remedy is an external resolver that "ensure[s]
they get closed out promptly", or else disabling the feature
([PostgreSQL 18, PREPARE TRANSACTION](https://www.postgresql.org/docs/current/sql-prepare-transaction.html)).

The transport cannot save you. iroh clamps its QUIC path idle timeout to 15 s
(`PATH_MAX_IDLE_TIMEOUT`, iroh's `endpoint/quic.rs`, "values higher than
`PATH_MAX_IDLE_TIMEOUT` (15 seconds) are clamped"), so a dead peer's socket is
torn down long before the barrier notices. The barrier — not the transport — is
the only unbounded wait left in the stack.

## Engine and library precedents

### Deadlines used by shipped networking stacks

| Stack | Deadline | Escalation | Source |
| --- | --- | ---: | --- |
| GGPO (rollback) | notify at 750 ms, disconnect at 5000 ms | event, then auto-disconnect | [`p2p.cpp` `DEFAULT_DISCONNECT_*`](https://github.com/pond3r/ggpo/blob/master/src/lib/ggpo/backends/p2p.cpp) |
| GGRS (Rust GGPO port) | notify at 500 ms, disconnect at 2000 ms | `NetworkInterrupted`, then `Disconnected` | [`sessions/builder.rs`](https://github.com/gschup/ggrs/blob/main/src/sessions/builder.rs) |
| Steam Networking Sockets | 10 000 ms initial, 10 000 ms connected | `ProblemDetectedLocally` end state | [`csteamnetworkingsockets.cpp` defaults](https://github.com/ValveSoftware/GameNetworkingSockets/blob/master/src/steamnetworkingsockets/clientlib/csteamnetworkingsockets.cpp) |
| ENet | 32 × RTO, clamped 5 000–30 000 ms | peer timeout | [`enet.h`](https://github.com/lsalzman/enet/blob/master/include/enet/enet.h) |
| Unity Netcode | `ClientConnectionBufferTimeout` 10 s (admission), `LoadSceneTimeOut` 120 s (scene event) | disconnect / release | [`NetworkConfig.cs`](https://github.com/Unity-Technologies/com.unity.netcode.gameobjects/blob/develop/com.unity.netcode.gameobjects/Runtime/Configuration/NetworkConfig.cs) |
| iroh (this repo's transport) | 15 s QUIC path idle | connection close | [`endpoint/quic.rs`](https://github.com/n0-computer/iroh/blob/main/iroh/src/endpoint/quic.rs) |

The spread is ~1 s to ~2 min. The ordering is consistent: **link-loss detection
is seconds, barrier completion is tens of seconds to two minutes**, because a
peer that must load a level is doing real work, not merely silent.

### GGPO / GGRS: two-stage notification, and a barrier that releases on death

GGPO's endpoint watchdog sends `GGPO_EVENTCODE_CONNECTION_INTERRUPTED` at
`disconnect_notify_start` (750 ms) carrying `disconnect_timeout - notify_start`
— a countdown the game can display — then declares the peer dead at
`disconnect_timeout` (5000 ms) and emits `DISCONNECTED_FROM_PEER`
([`ggponet.h`](https://github.com/pond3r/ggpo/blob/master/src/include/ggponet.h),
[`udp_proto.cpp`](https://github.com/pond3r/ggpo/blob/master/src/lib/ggpo/network/udp_proto.cpp)).
The start-up sync barrier then releases **for the surviving players**: a peer
that is `disconnected` no longer blocks `CheckInitialSync`
([`p2p.cpp`](https://github.com/pond3r/ggpo/blob/master/src/lib/ggpo/backends/p2p.cpp)):

```cpp
if (_endpoints[i].IsInitialized() && !_endpoints[i].IsSynchronized()
    && !_local_connect_status[i].disconnected) {
    return;   // still waiting
}
```

GGRS carries the same model in Rust as an event enum — `NetworkInterrupted {
addr, disconnect_timeout }`, `NetworkResumed`, `Disconnected`, plus
`Synchronizing { total, count }` for progress UI
([`GgrsEvent`](https://docs.rs/ggrs/latest/ggrs/enum.GgrsEvent.html)). For a Rust
game, that is the vocabulary to copy even if the session layer stays
host-authoritative.

### Unity Netcode: the closest match to this ticket

`SceneEventProgress` is a scene-transition barrier with exactly the missing
pieces
([`SceneEventProgress.cs`](https://github.com/Unity-Technologies/com.unity.netcode.gameobjects/blob/develop/com.unity.netcode.gameobjects/Runtime/SceneManagement/SceneEventProgress.cs)):

- the server records every client that was connected when the event started
  (`ClientsProcessingSceneEvent`);
- a client that disconnects is **removed from the wait set** and recorded in
  `ClientsThatDisconnected` (`OnClientDisconnectCallback`);
- a deadline is armed from `NetworkConfig.LoadSceneTimeOut` and a coroutine
  finishes the event on expiry: "This will handle completing the scene event when
  one or more client(s) disconnect(s) during a scene event and if it does not
  complete within the scene loading time out period it will time out the scene
  event."

The result is reported, not swallowed: `OnLoadEventCompleted(sceneName, mode,
clientsCompleted, clientsTimedOut)`
([`NetworkSceneManager.cs`](https://github.com/Unity-Technologies/com.unity.netcode.gameobjects/blob/develop/com.unity.netcode.gameobjects/Runtime/SceneManagement/NetworkSceneManager.cs)).
Join-in-progress is the same shape: `Synchronize` → client loads the server's
scenes → `SynchronizeComplete` → `OnSynchronizeComplete`.

Transition telemetry is built in: `SceneEventMetric` on
`NetworkMetricTypes.SceneEventSent` / `SceneEventReceived`
([`NetworkMetrics.cs`](https://github.com/Unity-Technologies/com.unity.netcode.gameobjects/blob/develop/com.unity.netcode.gameobjects/Runtime/Metrics/NetworkMetrics.cs)).

### Unreal: travel as an explicit state machine with typed failure

UE splits the problem in two. *Seamless* travel keeps the connection and loads a
transition map; *non-seamless* travel makes the client "disconnect from the
server and then re-connect to the same server, which will have the new map ready
to load" — and Epic recommends seamless "to avoid any issues that can occur
during the reconnection process"
([Travelling in Multiplayer](https://dev.epicgames.com/documentation/en-us/unreal-engine/travelling-in-multiplayer-in-unreal-engine)).
That is precisely the seat-reclamation path this repo already implements with a
replacement transport.

Failure is a closed enum, not a log line: `ENetworkFailure::Type` carries
`ConnectionLost`, `ConnectionTimeout`, `FailureReceived`, `TravelFailure`
([ENetworkFailure](https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/NetCore/ENetworkFailure__Type)),
and travel failures have their own `ETravelFailure::Type`. The driver enforces a
clock: `ConnectionTimeout` is "amount of time to wait before considering an
established connection timed out" and `InitialConnectTimeout` the same for a
connecting peer ([UNetDriver](https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Engine/UNetDriver));
stock `BaseEngine.ini` sets both to 60 s
([corroboration](https://forums.unrealengine.com/t/where-to-set-connectiontimeout-value/378351)).
Readiness has its own states — `WaitingToStart` → `InProgress` via
`StartMatch` ([Start Match](https://dev.epicgames.com/documentation/unreal-engine/BlueprintAPI/Game/StartMatch)),
i.e. the "waiting for players" screen is a match state, not an ad-hoc flag.

### Raft: unanimity is not the only safe rule

Two results from the Raft paper
([raft.pdf](https://raft.github.io/raft.pdf)) apply directly to a 3-seat
barrier:

- "A log entry is committed once the leader that created the entry has
  replicated it on a **majority** of the servers."
- "In the common case, a command can complete as soon as a majority of the
  cluster has responded to a single round of remote procedure calls; a **minority
  of slow servers need not impact overall system performance**."

and timeouts are what drive progress: randomized election timeouts of 150–300 ms
replace an unbounded wait for a silent peer. Raft also keeps safety independent
of clocks — "faulty clocks and extreme message delays can, at worst, cause
availability problems" — which is the right framing here: a deadline may evict a
live-but-slow peer (availability cost), it must never commit unsafe state
(correctness cost).

## What shipped co-op games do

Two GDC talks are the primary record for this problem in co-op games:

- **Bungie, "I Shot You First: Networking the Gameplay of Halo: Reach"** (GDC
  2011, David Aldridge) — deterministic synchronous networking for 1–4 player
  campaign co-op and Firefight, with host migration. The barrier is the slowest
  peer's input; migration is the answer when the peer stops answering.
  [GDC Vault](https://www.gdcvault.com/play/1014345/i-shot-you-first-networking) ·
  [official upload](https://www.youtube.com/watch?v=h47zZrqjgLc)
- **Bungie, "Shared World Shooter: Destiny's Networked Mission Architecture"**
  (GDC 2015, Justin Truman) — activity hosting and mission transitions with
  players joining and leaving mid-activity.
  [GDC Vault (free)](https://gdcvault.com/free/gdc-15) ·
  [upload](https://www.youtube.com/watch?v=Iryq1WA3bzw)

Read both for the shape of the problem — who owns the transition, how a joining
or leaving player re-enters — not for numbers; neither talk publishes timeout
values. The pattern the engines above make concrete, and these titles embody, is
that **someone must be allowed to decide, and the decision needs a clock.**

## Recommendations

Ordered by what removes the stall first.

1. **Give the barrier a deadline and a two-stage escalation.** Add
   `deadline: Instant` (and `notified_at: Option<Instant>`) to
   `PendingSnapshotTransition`, armed when `Prepare` is queued in
   `server_dispatch.rs` (`:259`). Emit `TransitionInterrupted { id, seat,
   remaining_ms }` at T1, then decide at T2. Copy GGPO/GGRS: notify earlier than
   you act.

2. **Choose values from the payload, not from a constant.** Link detection is
   seconds; transition completion is not. Start with
   `T2 = 5 s + payload_bytes / 512 KiB/s`, clamped to 30 s, and `T1 = 250 ms`
   (GGRS 500 ms, GGPO 750 ms are the sampled anchors). Unity's 120 s default is
   the upper bound to stay well under; a 2–3 player story campaign cannot spend
   two minutes on a black screen.

3. **Decide at T2 instead of waiting.** For a host-authored save or campaign
   exit, the commit is safe without the missing peer: the host already holds the
   bytes, and the peer keeps a parked seat that can re-enter through the normal
   replacement-transport admission. So at T2: **evict the seat from `awaiting`,
   commit for the connected players, park the evicted seat, and let rejoin
   re-admit it into the new mission.** This is Raft's majority rule, Unity's
   timeout release, and GGPO's disconnected-peer release in one move.

4. **Never let the wait set grow without a matching deadline.**
   `retain_for_reconnect` (`server_protocol.rs:104`) should re-arm the deadline
   for that seat, not merely re-insert it; a seat that is disconnected at T2 is
   evicted immediately rather than waited on (Unity removes disconnects from the
   wait set; GGPO's sync check skips `disconnected` peers).

5. **Make acks idempotent and re-sendable.** A duplicate `Ready` for the same
   `(transition id, seat)` is a no-op; an ack for a committed/unknown id is
   ignored with a `warn!`, never `RemoteProtocol`. Delete the "second prepare
   while pending" hard error
   (`crates/robin_rs/src/game_session/multiplayer.rs:877`) in favour of "same id
   → re-ack, different id → protocol error". Re-send `Prepare` to outstanding
   seats every ~500 ms with the same id, and re-send a pending `Ready` on
   reconnect — the modal lane already does exactly this for story acks
   (`docs/MULTIPLAYER.md`, "Reconnect sends retained modal progress … a client
   also resends any locally submitted acknowledgement for which it has not
   received a decision").

6. **Keep a tombstone so late frames are harmless.** After commit, retain the
   last few `(id, outcome)` pairs and drop acks/commits that match them. This
   closes the ack-in-flight-during-commit race that currently fails the session.

7. **Show the wait, then show the outcome.** Host and peers need
   "Waiting for *nickname*… (12 s)" from T1, and on eviction "Player X did not
   respond — continuing without them" for the survivors plus "you left the
   mission while it was changing; rejoin" for the evicted seat. Precedent:
   GGPO's `connection_interrupted { disconnect_timeout }` countdown, Unity's
   `clientsTimedOut` list. A silent freeze is the one outcome no shipped stack
   above chooses.

8. **Log and count the transition.** One structured event per phase —
   `transition_begin`, `transition_interrupted`, `transition_evicted`,
   `transition_commit` — with session id, transition id + sequence, seat,
   phase elapsed ms, payload bytes, `awaiting` size at each phase, disconnect
   reason, and outcome. Unity's `SceneEventMetric` is the shape; `tracing` at
   `warn` for every eviction so a stuck barrier is visible in a replay run.
   These also become the numbers that tune T2.

9. **Test the state machine on a virtual clock, not with sleeps.** Unit-test
   `SnapshotTransitions` directly: ack, duplicate ack, ack after commit,
   disconnect, reconnect, re-ack, T1, T2, eviction, second `Prepare` with a new
   id. Then one fault-injection test that drops the `Ready` frame and asserts the
   commit happens at T2 with the peer parked. GGRS ships a `SyncTestSession` for
   exactly this class of test: no network, deterministic re-simulation.

10. **Unify the two barriers.** `ReadyBarrier::candidate` already takes
    `(in_sim, attached, ready)` facts and ignores disconnected seats; give
    `SnapshotTransitions` the same fact-driven, deadline-driven shape. One rule
    in one place beats two policies that disagree about whether absence blocks.

## Anti-patterns seen in the wild (all of them in the current code path)

| Anti-pattern | Why it fails |
| --- | --- |
| Unanimity over a set that only grows | a single absent peer blocks every survivor (2PC blocking) |
| Retry with no stable identity | a re-sent `Prepare` looks like a protocol violation to a peer that already prepared |
| Duplicate ack = fatal error | normal at-least-once delivery becomes a session kill |
| Timeout that commits silently | players cannot tell an eviction from a bug |
| Deadline shorter than payload transfer | evicts healthy peers on slow links |
| `assert!` on a protocol race | turns a recoverable ordering into a panic (`crates/robin_rs/src/multiplayer/native/server_protocol.rs:71`, `crates/robin_rs/src/multiplayer/native/server_dispatch.rs:273`) |

## Open questions for wayfinder

- Retention policy for an evicted seat: how long, and does the survivor's mission
  continue solo or pause at the next checkpoint?
- Is "continue without the absent player" acceptable narratively in a 2–3 player
  story campaign, and who decides — host or player prompt?
- Does the deadline belong on `T2` only, or should a *host retrying a failed
  mission* carry a shorter retry budget (distinct from a first-attempt
  transition)?
- Should `T2` scale with measured peer throughput (bytes/observed rate) instead
  of a fixed 512 KiB/s assumption?
- Where does transition telemetry live: the replay journal, a separate counters
  file, or both?
- Who owns the waiting UI: the session modal lane (already reconnect-aware) or
  the mission loading screen?

## Sources

Engines and libraries

- [PostgreSQL 18 — PREPARE TRANSACTION](https://www.postgresql.org/docs/current/sql-prepare-transaction.html)
- [Raft paper (Ongaro & Ousterhout), §5](https://raft.github.io/raft.pdf)
- [GGPO — `ggponet.h`](https://github.com/pond3r/ggpo/blob/master/src/include/ggponet.h),
  [`sync.cpp`](https://github.com/pond3r/ggpo/blob/master/src/lib/ggpo/sync.cpp),
  [`backends/p2p.cpp`](https://github.com/pond3r/ggpo/blob/master/src/lib/ggpo/backends/p2p.cpp),
  [`network/udp_proto.cpp`](https://github.com/pond3r/ggpo/blob/master/src/lib/ggpo/network/udp_proto.cpp)
- [GGRS — `SessionBuilder`](https://docs.rs/ggrs/latest/ggrs/struct.SessionBuilder.html),
  [`GgrsEvent`](https://docs.rs/ggrs/latest/ggrs/enum.GgrsEvent.html),
  [`sessions/builder.rs`](https://github.com/gschup/ggrs/blob/main/src/sessions/builder.rs)
- [Unity Netcode `NetworkConfig`](https://docs.unity3d.com/Packages/com.unity.netcode.gameobjects@2.4/api/Unity.Netcode.NetworkConfig.html),
  [`SceneEventProgress.cs`](https://github.com/Unity-Technologies/com.unity.netcode.gameobjects/blob/develop/com.unity.netcode.gameobjects/Runtime/SceneManagement/SceneEventProgress.cs),
  [`NetworkSceneManager.cs`](https://github.com/Unity-Technologies/com.unity.netcode.gameobjects/blob/develop/com.unity.netcode.gameobjects/Runtime/SceneManagement/NetworkSceneManager.cs),
  [`NetworkMetrics.cs`](https://github.com/Unity-Technologies/com.unity.netcode.gameobjects/blob/develop/com.unity.netcode.gameobjects/Runtime/Metrics/NetworkMetrics.cs)
- [Unreal — Travelling in Multiplayer](https://dev.epicgames.com/documentation/en-us/unreal-engine/travelling-in-multiplayer-in-unreal-engine),
  [ENetworkFailure](https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/NetCore/ENetworkFailure__Type),
  [ETravelFailure](https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Engine/ETravelFailure__Type),
  [UNetDriver](https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Engine/UNetDriver),
  [Start Match](https://dev.epicgames.com/documentation/unreal-engine/BlueprintAPI/Game/StartMatch)
- [Steam Networking Sockets — `steamnetworkingtypes.h`](https://github.com/ValveSoftware/GameNetworkingSockets/blob/master/include/steam/steamnetworkingtypes.h),
  [default timeouts](https://github.com/ValveSoftware/GameNetworkingSockets/blob/master/src/steamnetworkingsockets/clientlib/csteamnetworkingsockets.cpp)
- [ENet — `enet.h`](https://github.com/lsalzman/enet/blob/master/include/enet/enet.h)
- [iroh — `endpoint/quic.rs`](https://github.com/n0-computer/iroh/blob/main/iroh/src/endpoint/quic.rs),
  [`QuicTransportConfig`](https://docs.rs/iroh/1.4.0/iroh/endpoint/struct.QuicTransportConfig.html)

Talks

- [I Shot You First: Networking the Gameplay of Halo: Reach (GDC 2011)](https://www.gdcvault.com/play/1014345/i-shot-you-first-networking) ·
  [official upload](https://www.youtube.com/watch?v=h47zZrqjgLc)
- [Shared World Shooter: Destiny's Networked Mission Architecture (GDC 2015)](https://www.youtube.com/watch?v=Iryq1WA3bzw)
- GDC Vault free GDC 2015 index: <https://gdcvault.com/free/gdc-15>

Repo

- `docs/MULTIPLAYER.md` — protocol 53, transition barrier, reconnect and modal ack rules
- `crates/robin_rs/src/multiplayer/native/server_protocol.rs` — `ReadyBarrier`, `SnapshotTransitions`
- `crates/robin_rs/src/multiplayer/native/server.rs` — `retain_transition_peer_for_reconnect`, commit path
- `crates/robin_rs/src/multiplayer/native/server_dispatch.rs` — `BeginSnapshotTransition` queueing
- `crates/robin_rs/src/game_session/multiplayer.rs` — client `Prepare` handling and single ack
