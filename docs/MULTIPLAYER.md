# Multiplayer architecture

Updated 2026-11-18. Multiplayer uses a server-ordered input stream with a
small scheduling delay, predictive simulation, rollback for late inputs,
periodic state-hash verification, and authoritative snapshots for joins. A peer
states its wire protocol version in its join and a mismatch refuses it; the
version is `NET_PROTOCOL_VERSION` in `crates/robin_run_types/src/lib.rs`, never
a number restated here, and compatibility with older protocols is unsupported.

## Seat model

- `PlayerId(u8)` is the deterministic seat identifier. `PlayerId::HOST` is
  seat 0; later peers receive seats in server join order.
- `PlayerInput` carries the issuing seat with every deterministic command.
- `Host::transport.local_seat` is process-local and says which seat this
  machine drives. It is never serialized.
- `SeatState` stores deterministic per-player selection, quick groups,
  connection/nickname state, follow target, locker state and alt-lock.
- Disconnect preserves a seat's selection and quick groups. Reconnecting with
  the same authenticated native endpoint or durable browser owner reclaims the
  parked seat; nicknames are presentation only.

Local input must use `PlayerInput::new(local_seat, command)`. The host-only
constructor is for single-player and explicitly host-owned paths.

## Transport

All game-session traffic runs over iroh. Native peers use QUIC with relay
fallback and hole punching; browsers use iroh's relay-over-WebSocket path and
only the HTTPS relay disclosed in the signed invitation. Peers are addressed
by iroh endpoint id — the
public half of a persistent per-install key stored next to the save data
(`multiplayer_identity.key`). Because the id is derived from the stored key,
a host's connect address is known before its endpoint is even bound, which is
how matchmaking can advertise a game ahead of mission launch. There is no
bind address, port forwarding, or NAT configuration anywhere. Endpoint ids
resolve through two layered address-lookup systems: the n0 DNS/pkarr default
and publish/resolve on the BitTorrent Mainline DHT
(`iroh-mainline-address-lookup`), so lookups keep working even without any
hosted discovery infrastructure. Each session uses one bidirectional stream
per peer carrying class-tagged, length-prefixed frames. The browser applies
the same directional allocation limits as native before decoding a body.

### Browser invitations, identity, and content

- Browser-link publication is a persisted Multiplayer/Privacy setting. It is
  on by default and can be overridden per host launch. Disabling it leaves
  native iroh play available without requiring relay readiness.
- A canonical `rhmp3` invitation is signed by the host endpoint key, valid for
  first redemption for exactly 30 minutes, and binds the wire protocol, the full
  engine commit, mission/session, expected seats, one disclosed canonical
  HTTPS relay, Demo/Full edition, and the native host's exact content-closure
  SHA-256. The URL stores the public ticket in its fragment; the stable shell
  captures and erases it before its first request.
- One authenticated multiplayer session spans its campaign's mission
  transports. At load/restart/campaign boundaries the host retains the exact
  session id, authenticated owner-to-seat roster, expected player count, and
  disclosed relay route for the one replacement transport. The next host
  endpoint consumes that state once; leaving the session discards it.
- The browser's durable seat key is an IndexedDB-held, non-extractable Ed25519
  private key on the isolated `identity.robinhood.phiresky.xyz` origin. That
  origin exposes only typed status, redemption, and seat-proof operations.
  The proof binds session, host endpoint, and the page's ephemeral iroh
  endpoint; there is no generic signing operation.
- Demo bytes must match their exact length, SHA-256, and native source-closure
  identity. Full owners select a local package whose canonical schema-2
  manifest binds the source Data/locale closure and every transformed
  datadir, split mission, and audio byte. Missing, extra, stale, or mismatched
  content stops before game boot. Retail assets are never uploaded.
- Operators compute the catalog/ticket closure with
  `cargo run --example content_identity -- <installation>/Data`. The Full web
  conversion recipe embeds the same value automatically.
- Relays can observe participant IP addresses, connection timing, and byte
  counts. Gameplay remains end-to-end encrypted; the invitation/log/UI states
  this rather than implying relay anonymity.

## Matchmaking

Matchmaking is fully serverless. Every player who opens the multiplayer menu
joins one well-known iroh-gossip topic; peers for the topic are found through
the Mainline DHT (`distributed-topic-tracker`), so there is no broker, no
master server, and nothing to configure. Hosts periodically announce their
game as soft state (listings expire when the announcements stop); joiners
broadcast their join intent and the host counts them; Start is broadcast with
the synchronized `start_at_epoch_ms` (and repeated, with the started-state
announce as a fallback path, since gossip is fire-and-forget). The game id
doubles as the host's game endpoint id, which joiners then pass to the normal
`--connect` path. Announcements are currently unauthenticated soft state —
signing them with the game identity key is a known TODO.

## Ownership and platform split

`robin_engine::multiplayer` contains platform-neutral protocol types,
`NetChannels`, the shared frame cursor and the snapshot handoff. The native
iroh transport and identity live in
`robin_rs::multiplayer::{native, identity}`; the relay-only browser client
lives in `robin_rs::multiplayer::wasm`. Canonical ticket and content-closure
code is shared across the platform boundary.
Mission-loop admission, input scheduling, rollback and hash comparison live in
`robin_rs::game_session::multiplayer`. Session modal identities, acknowledgements,
recovery and decisions live in `robin_engine::multiplayer::modal_session`. The graphical
and true-headless drivers use the same network drain and timeline admission
state; the headless path does not construct renderer, native-input, UI, or
audio substitutes.

Each wait a participant can block on has one home: the snapshot-transition and
ready barriers in `robin_rs::multiplayer::native::server_protocol` (driven by
`native::server`), the reconnect loop in `robin_rs::multiplayer::client_session`,
the modal acknowledgement gate in `robin_engine::multiplayer::modal_session`, and
the admission phases in `robin_rs::game_session::runtime::admission`.

The Engine owns deterministic seats and the shared script/director camera. The
interactive host owns its one local `ViewportState`; rendering projects the
world through that host viewport while HUD/input query the Engine selection for
`host.transport.local_seat`. Do not recreate the obsolete per-seat Engine
camera design: split-screen or replay-from-another-seat would need an explicit
host viewport policy.

## Leaderboard submissions

Multiplayer has no ranked session protocol: there are no named-seat join
attestations, ranked admission challenges, or leaderboard co-sign messages on
the wire, and seats enter the simulation as soon as `Welcome` is queued.
Ranked submission is a single-uploader recorded replay handled after the
mission: the host uploads the replay signed by its player key and the server
re-simulates it; the other seats stay anonymous. Current ranked policies reject
co-op configurations (`coop.players > 1`), including general “Any configuration”
boards. Co-op mission-end screens explain this restriction and allow browsing
the mission's general board without offering an upload. Non-host clients never
prepare submission evidence from their local prediction/reconnect journals.

## Protocol 53

Messages are bitcode-encoded binary frames, class-tagged and length-prefixed
on a single bidirectional iroh stream per peer. The handshake rejects a
different protocol version.

| Direction | Message | Purpose |
| --- | --- | --- |
| client → server | `Hello { protocol_version, nickname, browser_auth }` | open or resume a session; browser auth carries the signed ticket and durable seat proof |
| server → client | `ContentOffer { offer }` | disclose the exact bounded full-mod/package identity before assigning a gameplay seat |
| client → server | `ContentRequest { full_mod_sha256, resume_offset }` | consent to and resume the exact immutable content transfer |
| client → server | `ContentReject { full_mod_sha256, reason }` | explicitly refuse content without falling back to mismatched local assets |
| server → client | `ContentChunk { full_mod_sha256, offset, total_bytes, bytes }` | transfer one bounded sequential segment of the canonical package |
| client → server | `ContentReady { full_mod_sha256 }` | confirm that exact content was verified, admitted, and mounted before `Welcome` |
| client → server | `ContentPrepared { full_mod_sha256 }` | cache verified content during lobby preflight without claiming a gameplay seat |
| server → client | `Welcome { your_seat, session_id, mission_id, mission_seed, sim_config, speech_timing_locale, host_nickname }` | authoritative session identity, mission construction, speech timing, and seat assignment |
| server → client | `Reject { reason }` | typed fail-loud admission rejection |
| client → server | `Input { origin_frame, command }` | propose a local command |
| server → peers | `BroadcastInput { server_frame, origin_frame, target_frame, input }` | globally ordered, scheduled input |
| server → peers | `StateHash { frame, hash, clock_frame, ms_until_next_frame }` | desync detection and pacing sample |
| server → joining peer | `InitialSnapshot { frame, engine_bytes }` | authoritative mid-mission state |
| client → server | `ReadyToSim { frame }` | peer loaded and adopted the snapshot |
| server → peers | `BeginSim { frame, start_epoch_ms }` | release the start barrier |
| client → server | `ModalProposal { instance, kind, result, requested_frame }` | present a non-authoritative client request to the host |
| server → clients | `ModalProgress { instance, kind, accepted, player_names }` | announce a story and retain its acknowledgement state for reconnect |
| server → clients | `ModalDecision { instance, kind, result, decision_frame }` | commit the host's sole authoritative result for one exact modal occurrence |
| server → client | `ReconnectRequired { reason }` | discard the prediction future and perform a complete handshake/snapshot admission |
| server → clients | `PrepareSnapshotTransition { id, payload }` | distribute exact host-authored save or campaign-exit bytes for validation and retention |
| client → server | `SnapshotTransitionReady { id }` | acknowledge the exact prepared transition bytes |
| server → clients | `CommitSnapshotTransition { id }` | release the transition only after every current peer is ready |

Client `Input` frames can contain only seat-authored gameplay commands. Shared
settings, campaign mutations, modal decisions, and seat connect/disconnect
events require host authority and are rejected before server broadcast; the
engine enforces the same predicate again at deterministic command admission.

`Welcome` is authoritative. A peer must not substitute a local mission, seed,
or `SimConfig` after decode failure. The snapshot payload uses the current
Engine schema and is rejected rather than migrated when incompatible.
`speech_timing_locale` is `None` for new sessions: the required core
`Data/AudioDurations.json` owns speech and sound-source timing. Peers can
play different installed languages without installing an English voice pack.
The legacy `Some(locale)` handshake belongs to earlier engine builds.

The session id is stable across an authorized outer-mission rebuild. A native
client reuses its process-held iroh transport key, while a browser re-proves its
durable signer key. The replacement server seeds those authenticated owners as
disconnected seats and accepts only their exact reclamation; nicknames carry
no authority. The host pins the previously disclosed relay route for the
replacement endpoint, so a redeemed browser invitation remains reachable
without silently minting a new invitation lifetime.

Native client transport keys are not persisted: same-process reconnect and
outer-mission rebuild retain the seat identity, but restarting the client
process creates a new transport owner. The separate durable game identity is
used for ranking and does not authorize reclamation of that parked native seat.

## Input scheduling and rollback

The server stamps every input for:

```text
max(server_frame, origin_frame) + INPUT_DELAY_FRAMES
```

`INPUT_DELAY_FRAMES` is currently 2 (about 80 ms at 25 Hz). Each mission keeps
future inputs grouped by target frame. Commands within a frame execute in
ascending player-seat order, preserving each player's own command order. The
complete frame is ordered after merging network drains, and late-input edits
restore that same order in the retained journal before reconstruction.
When a late input arrives:

1. splice it into the shared command log at its target frame;
2. restore the dense recent Engine snapshot when available;
3. replay deterministically to the current frame;
4. use the longer-horizon rewind history when outside the dense window;
5. require a full host-snapshot reconnect when the input predates the retained
   command history, rather than applying it at a different frame.

A client state-hash mismatch requests a diagnostic report and initiates the same
snapshot reconnect. Gameplay input and simulation are held immediately, pending
prediction inputs and hashes are discarded, and recovery completes only after
snapshot adoption and the host readiness barrier. Further mismatches during that
recovery do not initiate additional reconnects. A missing historical hash sample
is not treated as a mismatch.

During reconstruction, live speech resolutions are regenerated from the corrected
pending speech queue and the mission's immutable timing catalog. This applies to
both paused transactions and ticks, including newly pending speech where the old
prediction was silent. Corrected facts and checkpoints are published only after
successful reconstruction; ordinary rewind and replay retain strict recorded-fact
validation. If reconstruction still fails, a client requests a host snapshot.

This is predictive rollback, not strict lockstep. Clients are paced close to
the host clock but do not wait for every possible input before each tick.

Client pacing waits are capped at one 40 ms tick so clock corrections cannot
block network ingress or modal acknowledgements. Prediction stops one hash
publication interval beyond the newest host clock sample until a fresh sample
arrives. Shared story waits rebase the local clock without advancing its host
frame watermark, so reading a scroll does not create fast-forward catch-up debt.

The host broadcasts a pre-tick state hash every 25 frames. Peers compare at the
same frame boundary used by replay recording. Hash mismatch is an error and
must not be repaired by silently adopting a new default Engine.

## Join, start and reconnect

- The host publishes a current Engine snapshot for peers that join after
  initialization.
- A joining peer installs the exact snapshot and trims older pending input/hash
  state before announcing `ReadyToSim`.
- Both interactive and true-headless missions wait until the configured
  expected players are ready, then the server broadcasts `BeginSim`. The wait is
  bounded: if the host has loaded and a peer has not become ready after 120 s,
  the host evicts that peer and starts with the participants that did load. A
  peer admitted after the wait began refreshes the budget once, so a late joiner
  gets a full load window; a second joiner does not extend it again.
- Admission is an explicit timeline state machine: a host waits for
  `BeginSim`; a peer waits for successful snapshot adoption, then `BeginSim`;
  both wait for `start_epoch_ms` before simulation advances. A peer never
  announces `ReadyToSim` after a decode/adoption failure, and `BeginSim` before
  its snapshot is a fatal ordering error. Each admission phase has its own
  180 s budget: when it expires the session ends with a fatal error instead of a
  loaded but frozen mission, and a pinned JSON bundle naming the phase, seat,
  session and newest replay is written under `robin_hood/diagnostics/`.
- True-headless hosts perform the same local-seat bootstrap and frame-zero
  snapshot publication as interactive hosts, then keep the reconnect snapshot
  and host clock samples current while running.
- Native clients reconnect with exponential backoff and reclaim their parked
  owner generation. Browser clients reconnect with the same ephemeral iroh
  endpoint and durable session/host/transport proof. Reconnection is bounded:
  after a drop the client retries for 60 s, then publishes a fatal timeout that
  returns the game to the menu instead of leaving it on a frozen frame.
  Disconnect immediately
  returns admission to the snapshot-wait
  state; simulation and headless HTTP timeline steps remain held until the
  replacement snapshot and `BeginSim` are accepted. A replacement snapshot
  may be older than the abandoned prediction future; adoption clears queued
  inputs, hashes, rewind anchors, and dense history before seeding the new
  exact timeline anchor. Ordinary stale snapshots remain ignored.

## Modal authority and mission transitions

Shared stories have session-owned identities and acknowledgement state. Effect
admission reserves their identities at the source timeline boundary, before a
widget exists. The common modal batch lifecycle selects the next presentation;
widgets bind to that existing occurrence and submit local outcomes. Every
admitted player must acknowledge a story before the host publishes its decision.

The session drain services acknowledgements and retains authoritative decisions
while simulation is paused, independently of widget polling. A shared story's
close does not wait for a future simulation frame: the client may be paused one
frame behind the host. The session itself holds the story pause barrier before
the surface is drawn. Progress, duplicate acknowledgements and repeated decisions
are idempotent; conflicting authoritative decisions are errors.

Engine rollback leaves already-admitted presentation decisions intact. Both
recent-history and sparse-snapshot reconstruction collect story effects without
replaying audiovisual output. Reconciliation recovers newly discovered occurrences
and suppresses already-admitted ones, counting repeated identical text ids
separately. Filtering a page or aborting its batch retires only its pending
reservations, preserving later batches.

Reconnect sends retained modal progress and completion tombstones alongside the
engine snapshot. A client also resends any locally submitted acknowledgement for
which it has not received a decision. Neither recovery depends on a new engine
tick or another click. The wire message shapes, engine save schema and replay
encoding are unchanged. The normal replay journal still records the authoritative
kind/result at the mission-frame boundary; playback and seeking retain their
existing modal batch policy.

The acknowledgement barrier is bounded. Once the host has drawn a shared page and
it is still waiting, a seat that has not acknowledged after 300 s is marked
acknowledged: the host continues with the participants that are present, and the
late or reconnecting seat receives the durable decision instead of a second
chance to block the session. A peer's missing acknowledgement costs it the page,
never the session. While the wait is live the story surface shows the seats it is
waiting for and the seconds left before the host continues without them.

Host-only save, load, restart, QuickLoad, and campaign-exit operations use an
exact `Prepare`/`Ready`/`Commit` barrier. Clients first decode, validate, and
retain the exact host bytes. The host commits only after every currently
connected peer acknowledges the same session-bound transition id; reconnecting
peers must acknowledge again. Participants then leave the old mission and
perform a complete handshake/readiness admission for the replacement state.
Local multiplayer saves remain explicitly diagnostic and are never accepted as
authoritative load input.

The barrier is bounded. The host stamps a deadline at prepare time from a
monotonic clock: 60 s plus one second per 64 KiB of payload, which pays for the
bytes a peer must receive, validate and retain. When the deadline passes, every
seat still waiting is evicted from the barrier, the host commits for the
remaining connected peers, and the evicted seat keeps its authenticated owner:
it re-admits later from an authoritative snapshot. A late acknowledgement from
an evicted seat is a no-op, and a duplicate `Ready` is idempotent. A transport
close starts the deadline; it never evicts on its own. Without a deadline a
frozen client, or a connection that was never reclaimed, freezes every mission
transition for the rest of the session.

A peer's copy of a prepared transition belongs to the transport generation that
received it. When that generation ends — a disconnect, desync recovery, or the
host rebuilding its mission transport for a replacement mission — the peer drops
the prepare and names it in the log; the host's replacement stream re-sends a
still-pending prepare with the same session-bound id, so the cost is one
re-acknowledgement and never a lost session. A prepare that stays uncommitted
past its own payload budget fails that peer's session by name (`snapshot
transition … stayed prepared for … ms`) instead of being discovered minutes
later by the admission watchdog. Holding a start barrier for a prepare that can
no longer be committed is what left a restarted mission frozen with one seat
running and the other waiting for `BeginSim` (#43).

## CLI

- `--server` hosts a game transport on the install's persistent iroh
  identity; the endpoint id to share is logged at startup.
- `--connect ENDPOINT_ID` joins a host by its endpoint id.
- `--mp-expected-players N` configures the start barrier for direct launches.
- `--mp-nickname NAME` supplies the display/overlay label, not reconnect authority.
- `--mp-browser-join-links true|false` overrides the saved browser-link
  publication preference for this hosted game.
- `--join RHMP3_TICKET` consumes a canonical host-signed invitation. The web
  shell installs it internally after authenticating and scrubbing it.

Only the host records the canonical server-ordered multiplayer replay. A
connecting peer cannot select `--record`, and the browser peer's replay RPC
fails with `no active replay recording` instead of publishing a competing
history. Native diagnostic recordings preserve snapshot adoption and late-input
reconstruction as explicit exact-state boundaries. Replay schema 66 distinguishes
these network corrections from saved-game loads, verifies their state hashes,
and excludes them from ranked submission. Playback adopts the recorded engine
state without applying saved-game load fixups.

The in-game multiplayer menu needs no flags or environment: it joins the
serverless matchmaking swarm automatically (see **Matchmaking** above).

## Deterministic smoke test

Build first, then run host and client separately (the host logs
`multiplayer: hosting on iroh endpoint <ID>` — pass that id to the client):

```bash
cargo build --bin robin

target/debug/robin --server --mp-expected-players 2 \
  --mp-nickname host --start-paused --fast-forward --http-server 7780

target/debug/robin --connect <HOST_ENDPOINT_ID> --mp-nickname alice \
  --start-paused --fast-forward --http-server 7781
```

After both peers report `BeginSim`, advance both through the HTTP server in
matching increments and verify:

- no `multiplayer DESYNC` or fatal transport errors;
- matching state hashes at frames 0, 25, 50, ...;
- commands from both seats appear in the same server order;
- a deliberately late input rolls back and reconstructs the same hash;
- reconnect reuses the same seat and preserves its selection/quick groups.

For meaningful gameplay coverage, replay the same recorded command stream on
both peers; stepping empty frames proves clock/snapshot agreement but not the
full command surface.

The live browser driver is `wasm-www/scripts/live-relay-e2e.mjs`. It requires
a native host with its loopback HTTP API, a fresh `rhmp3` ticket, and Chrome
already listening on a remote-debugging port. It asserts relay-WebSocket
welcome, authoritative input delivery, a forced transport loss, a progressed
replacement snapshot, post-reconnect input, host replay availability, and
browser-peer replay refusal. Production QA additionally requires the isolated
signer origin and exact ticket-selected `/wasm/<commit>` and content catalog
to be deployed.

The same recorded hash chain is the oracle for cross-build agreement: the
`parity-cross` gate replays its tracked recording corpus
(`scripts/validation/parity_corpus/`) on a native Linux build and on the Windows
release build under Wine and requires the chains to match. Its inputs, exit
codes and limit are in `docs/TESTING.md`, "Linux × Windows replay-hash parity".

## Remaining work

Fault and reorder coverage is a contract now, not a wish: the matrix in
`multiplayer/HARNESS.md` §1 names every cell, the corpus in
`scripts/validation/scenarios/` states each one, and the file that proves each
row is named in the row and enforced by the corpus gate — a Rust test for the
core and transport rows, `multiplayer-e2e` for the process rows. The process
suite runs real processes with real signals on the build host, on demand and as
the second step of the stable-release preflight (`docs/RELEASES.md`), not on
every push: it needs the licensed data, a built binary and an unprivileged
namespace. What follows is what the matrix deliberately does not cover yet.

- define a host viewport policy for split-screen or replay viewing from a
  non-local seat;
- define dedicated-server seat ownership; a true-headless `--server` still
  owns and bootstraps seat 0 like an interactive host;
- extend the matrix around Spellforge snapshot adoption beyond admission, which
  already recomputes the package hash, journal usage/digest and combined
  snapshot budget before state hashing or VM attachment, and whose bounded
  persistent journal participates in snapshot adoption and desync detection;
- Windows-side faults (WinDivert duplication, Known Folder paths, MSVC replay
  hashes) wait on the parity gate in #21, which needs a Windows executor.

## What each seat records

Every seat records, into its own install directory (see the harness doc's
isolation rule):

- a local mission replay of every mission it plays (`replays/`), produced by the
  same recorder single-player uses — a peer is not a spectator, it owns a
  timeline;
- `logs/robin-<YYYYMMDDTHHMMSS>.log`, this run's full trace — every line stderr
  would have carried, written whether or not the process has a terminal. Ten
  runs are kept and older ones are pruned at startup;
- `logs/events.jsonl`, the session narrative as one JSON object per line:
  mission start, host start, admission phases, seat connect/assign/evict,
  transport disconnect/reconnect, start-barrier reach/release, the transition
  prepared/committed/aborted/dropped states, modal announce/propose/ack/expiry,
  desync and session failure. An eviction is an admitted `DisconnectSeat`; a
  transport retry is its own `transport_disconnected`/`transport_reconnected`
  pair, so a reconnect is never read as a participant leaving. Each line
  carries `seq`, `uptime_ms`,
  `utc`, `kind`, `seat`, `frame` and `detail`, and the same call site writes the
  log line, so the two cannot disagree;
- `GET /events?since=<seq>` while the process lives, which serves that same ring
  (see [JSON_SERVER.md](JSON_SERVER.md#session-events));
- `GET /state`, which keeps answering after the mission ends: the last published
  snapshot plus a `terminal` block naming the operation and why it stopped;

and, when a session fails, a pinned bundle under `diagnostics/` named for its
session — `session-<session id>.json`, or the write time when the failure
predates a session identity — that names the reason, the phase, the seat, the
session, the newest replay's path and modification time, and this run's log.

One install home therefore reconstructs a session without the game: the replay
carries the inputs and hash chain, the run log carries everything traced, and
`events.jsonl` carries the multiplayer facts with timestamps and sequence
numbers. A failing seat's directory is the whole report.

The **host's** replay is the shared record: it carries every seat's input and the
authoritative hash chain, and a bug report is reproduced from it. A peer's replay
is a local witness: comparing the two per-25-frame hash chains is the desync
oracle. No seat ever writes another seat's files, and no peer records on behalf of
the host.
