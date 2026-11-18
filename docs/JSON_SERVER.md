# HTTP automation server

The `robin` binary exposes a loopback-only HTTP endpoint for debug tools,
test harnesses, replay drivers and screenshot pipelines. The implementation is
in [`http_server.rs`](../crates/robin_rs/src/http_server.rs).

This is a small JSON-over-HTTP API, not a JSON-RPC 2.0 server. Responses are
JSON except for `/screenshot`.

## Starting it

The server binds only to `127.0.0.1`; it has no authentication or TLS. The
default port is 17640.

```text
robin                         # listen on 127.0.0.1:17640
robin --http-server 9999      # choose another port
robin --http-server 0         # disable the server
robin --start-paused          # freeze simulation at frame zero
```

Build before launching a long-running game process:

```bash
cargo build --bin robin
RUST_LOG=debug ROBINHOOD_DATA_DIR=datadirs/demo_leicester_ecoste \
  target/debug/robin --start-paused
```

The listener runs in a dedicated `robin-http-server` thread. It places work on
a queue which the mission loop drains on the game thread. A request waiting on
that queue times out after 60 seconds; use a shorter client timeout when a
blocked or loading game should fail quickly.

## Inspection endpoints

| Method | Path | Result |
| --- | --- | --- |
| `GET` | `/` or `/info` | endpoint discovery document |
| `GET` | `/natives` | every script native with index and signature |
| `GET` | `/state` | last published frame, map, replay, multiplayer status and terminal block |
| `GET` | `/events` | multiplayer session event ring (`?since=<seq>&limit=<n>`) |
| `GET` | `/engine-dump` | complete serialized deterministic Engine state |
| `GET` | `/level-assets` | static level data plus runtime fast-grid flags |
| `GET` | `/host-debug` | non-authoritative host/UI debug state |
| `GET` | `/script` | loaded script classes, functions and instance counts |
| `GET` | `/script/decompile?class=Foo` | TypeScript-like script decompilation; omit `class` for all classes |

`/engine-dump` and `/host-debug` are deliberately separate. The former is
authoritative simulation state; the latter contains local viewport, hover and
trajectory-preview details which do not belong in Engine snapshots.

`/state` and `/events` answer from the process's own observability surface
without entering the mission queue. That queue is exactly what stops answering
when the game thread is deadlocked, and the mission is exactly what is gone once
it ends. On native the answer is built on the listener thread; in the browser it
is built inside the `rh_rpc` call, before the request could reach the tick queue.
Either way one shape answers, and neither endpoint has a queued decoder. Other
endpoints still queue their work and keep the 60-second budget.

## `/state`

`/state` returns the last snapshot the mission loop published at an RPC
boundary, so it answers after the mission that produced it has ended.
It also carries the admitted session when one exists; a single-player mission
reports `"multiplayer": null`, and so does a session whose host Welcome has not
been adopted yet — the roster arrives with that Welcome, so there is no player
count to report before it.

```json
{"frame": 133, "map": "Dem_Lei_MP", "replay": null,
 "multiplayer": {
   "local_seat": 0, "expected_players": 2,
   "admission": "running", "begin_sim_frame": 133,
   "pending_transition": null,
   "last_compared_hash_frame": 125, "desync_count": 0,
   "final_operation": "level_in_progress",
   "connect_string": "{\"id\":…,\"addrs\":[…]}"
 },
 "terminal": null,
 "observability": {
   "trace_log": "/home/you/.local/share/robin_hood/logs/robin-20261118T101500.log",
   "event_log": "/home/you/.local/share/robin_hood/logs/events.jsonl",
   "event_run": "20261118T101500-4711#0",
   "next_event_seq": 42
 }}
```

`admission` and `final_operation` are stable lowercase names
(`MultiplayerAdmission::wire_name`, `GameCode::wire_name`); neither is Rust
`Debug` output. `pending_transition` is `{"id", "payload", "awaiting"}` with
`payload` `"Save"` or `{"CampaignExit": "<GameCode wire name>"}` and `awaiting`
the seats whose acknowledgement the host's commit still waits for.
`connect_string` is the host's dial address and `null` on a client.

`terminal` is `null` while a mission is producing snapshots. A mission that
ended sets it, and the snapshot stays the last one published:

```json
{"terminal": {"operation": "Quit", "reason": "mission ended",
              "at": "2026-11-18T10:22:41.120Z"}}
```

`operation` is the `GameCode` the mission ended on; a mission that died without
reporting one keeps the `final_operation` of the snapshot that mission
published. A failed launch reports `null` with the launch error as `reason`, and
a mission that succeeded but whose services failed to drain reports
`application shutdown: …` — a shutdown, not a failed launch. One attempt records
one `terminal` and one `session_ended` event; a second writer for the same
attempt is ignored.

While an attempt is running, the frame, map and `multiplayer` block describe that
attempt's last boundary. A new attempt starts without the previous one's
snapshot, so a failed launch reports `null` for all three rather than repeating
the mission before it.

Before the first RPC boundary there is nothing to report: `/state` answers `503`
with `{"error":"engine not ready — no mission state yet"}`.

`observability.next_event_seq` is the cursor to hand to `/events` to follow the
session from this snapshot onward, and `event_run` names the run that cursor
belongs to. `trace_log` and `event_log` are the two files described below.

## Run log

Every native process writes its trace to
`<install home>/logs/robin-<YYYYMMDDTHHMMSS>.log` in addition to stderr, so a
game started from a desktop launcher that inherits no terminal still leaves
evidence. `<install home>` is `ROBINHOOD_INSTALL_HOME` when set, else
`<platform data dir>/robin_hood`; `install_home::log_directory` is the only
resolver. The process keeps the newest ten run logs and prunes the rest at
startup, and logs the path it opened once, at startup. `/state`'s
`observability.trace_log` repeats it.

## Session events

`GET /events?since=<seq>&limit=<n>` serves the multiplayer event ring, oldest
first. `limit` defaults to 100 and caps at 1000; `since` is the `next_seq` from
the previous page, so a poller never repeats an event.

```json
{"run": "20261118T101500-4711#0",
 "events": [
  {"seq": 41, "uptime_ms": 812345, "utc": "2026-11-18T10:21:05.422Z",
   "kind": "barrier_released", "seat": null, "frame": 133,
   "detail": {"start_epoch_ms": 1763461265422,
              "message": "multiplayer: begin-sim barrier released at frame 133"}}
 ],
 "next_seq": 42}
```

`kind` is one of `mission_started`, `host_started`, `admission`,
`seat_connected`, `seat_assigned`, `seat_evicted`, `transport_disconnected`,
`transport_reconnected`, `barrier_reached`, `barrier_released`,
`barrier_expired`, `transition_prepared`, `transition_committed`,
`transition_aborted`, `transition_dropped`, `modal_announced`, `modal_proposed`,
`modal_acked`, `modal_expired`, `desync`, `session_failed` and `session_ended`.
`seat` and `frame` are `null` when the event has no seat or frame; `detail`
carries the event's own fields and always names the wording in `message`.

`host_started` means this process is hosting; `seat_connected` means a peer
joined (by its `ConnectSeat` or by this process dialing out); `seat_assigned` is
this process receiving its seat from the host; `seat_evicted` is reserved for an
admitted `DisconnectSeat`. A lost peer link is `transport_disconnected` followed
by `transport_reconnected` when the transport recovers — the transport retries
on its own, so no seat was evicted.

`seq` restarts at zero in every process, so `run` identifies the process run that
served the page and `next_seq` is always that run's true cursor. A poller that
keeps a cursor across a game restart sees `run` change and an empty page whose
`next_seq` is the new run's cursor, not the stale one it sent: reset the cursor
and re-read, or the missed events stay invisible. `observability.event_run` in
`/state` carries the same identity.

The same call site writes the ring, the tracing line and one JSON line in
`<install home>/logs/events.jsonl`, so the log text and the JSON never drift.
The ring holds the newest 512 events in memory. On disk the live file belongs to
one process: startup archives the previous run's file as
`events-<stamp>-<pid>.jsonl`, keeps the newest two archives, and stops appending
past 4 MiB (the ring keeps answering). Both survive the mission that produced
them.

## Command endpoints

### Script natives

`POST /native` invokes one native. `this` optionally supplies the transient
script receiver.

```json
{"op":"SetMissionWon","args":[],"this":null}
```

`POST /batch` invokes multiple natives back-to-back in one queue drain:

```json
{"calls":[
  {"op":"GetFrame","args":[]},
  {"op":"SetMissionWon","args":[]}
]}
```

Missing required Engine objects fail with a contextual error; the server does
not manufacture default values for malformed calls.

### Console and player commands

`POST /console` accepts the same command string as the in-game debug console:

```json
{"command":"give all"}
```

`POST /command` accepts the externally tagged JSON representation of
[`PlayerCommand`](../crates/robin_engine/src/player_command.rs):

```json
{"SelectPc":{"pc_id":{"Pc":1},"append":false}}
```

The command enters the same deterministic command path as interactive input.

## Screenshots

`GET /screenshot` returns raw `image/png` bytes. Without `frame`, it captures
the next rendered frame; `frame=N` waits until the simulation has reached at
least that absolute frame.

| Query | Meaning |
| --- | --- |
| `frame` | earliest absolute simulation frame to capture |
| `full_map` | capture the complete level at 1:1 map scale |
| `w`, `h` | aspect-preserving output bounds |
| `hide_ui` | omit screen-space HUD drawing; for a viewport capture, also crop the panel area |

Boolean debug-overlay overrides are `view_cones`, `pc_sight`, `motion_graph`,
`surface`, `all_obstacles`, `elevation`, `noise`, `sound_source`, `actor_info`,
`script_zones`, `door`, `projection_areas`, `railroad`, `probability`,
`company_number`, `combat_energy`, `light_zones`, `animation_lines`,
`seek_points`, `fps`, `sprite_masks` and `entity_ids`.

Values accept `1`/`0`, `true`/`false`, `yes`/`no` and `on`/`off`; a bare flag
means true. Overrides affect only the throwaway capture frame and do not mutate
the live `DevState`.

```bash
curl -o shot.png \
  'http://127.0.0.1:17640/screenshot?hide_ui=1&view_cones=1'

curl -o map.png \
  'http://127.0.0.1:17640/screenshot?full_map=1&hide_ui=1&frame=100'

curl -o thumb.png \
  'http://127.0.0.1:17640/screenshot?w=640&h=480'
```

## Timeline control

These endpoints are intended for deterministic automation, especially with
`--start-paused`.

| Method | Path | Body | Behavior |
| --- | --- | --- | --- |
| `POST` | `/step-forward` | `{"n":10}` | run full frame-equivalent ticks |
| `POST` | `/step-back` | `{"n":10}` | restore and replay through rewind history |
| `POST` | `/go-to-frame` | `{"frame":100}` | move forward or backward to an absolute frame |
| `POST` | `/set-paused` | `{"paused":true}` | change the manual-pause flag |

Forward stepping applies recorded commands when replay playback is active and
runs normal rollback/history bookkeeping. Automation steps silently dismiss
pending dialogs, popup scrolls, debriefings, Sherwood reports and pause-all
states so they cannot deadlock the driver. Successful replies include
`modals_dismissed` where applicable.

Backward movement fails if the requested frame predates retained rewind
history. Moving the timeline resets rollback-checker history because the live
Engine is now on a reconstructed timeline.

A mission-end terminal page owns the mission until it is dismissed. A step with
a matching typed dismissal (or `auto_dismiss`) queues that dismissal for the page
and then advances the frame that applies it, so a paused automation session makes
progress instead of being rejected forever. On a multiplayer client the same
request is the acknowledgement the host's barrier waits for: it answers `200`
with `{"direction":"terminal-modal-dismissal","queued":true}` and never advances
the host's timeline, because frame stepping stays host-only.

A tick that ends the mission stages the campaign completion and queues the
mission-end page, and the session keeps running the page instead of closing the
application. The step's own transaction hands that completion to the same
post-initialize stage a live frame uses, so the campaign update is applied once,
in the frame that ended the mission, and the page settles on the next frame
without the driver un-pausing. Until the page exists a step without a dismissal
is refused with "mission ended" rather than advancing behind it.

```bash
curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"n":1}' http://127.0.0.1:17640/step-forward

curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"frame":250}' http://127.0.0.1:17640/go-to-frame
```

## Replay handoff

- `GET /get-replay` returns the current recorder's JSONL byte stream from its
  in-memory mirror as `{"content":"..."}`.
- `POST /load-replay` accepts `{"data":"...","paused":true}`. It stages the
  replay for the next mission start; the caller must then trigger that restart.

The same handoff works for native and wasm drivers. Replay schema compatibility
is intentionally limited to the current format.

## Errors

Failures return a 4xx or 5xx response with `{"error":"..."}`:

- `400` for malformed input or a rejected operation;
- `404` for an unknown route;
- `500` if the response channel disappears;
- `504` if the game thread does not process the request within 60 seconds.

For endpoint discovery, prefer `GET /` over copying a route list into external
tools; the discovery document is maintained next to the server dispatcher.
