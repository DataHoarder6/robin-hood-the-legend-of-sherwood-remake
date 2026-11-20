# Remote teammate cursors — design evidence

Read-only investigation behind `docs/superpowers/plans/2026-11-20-remote-cursors.md`.
Facts from the worktree at `8eece78e1`; web sources listed at the end.

## Decision (confirmed by owner)

Opt-in overlay showing co-op teammates' cursors live, smaller and colour-coded per
seat. All twelve grill decisions accepted; see the plan for the task-level form.

## Repo facts that constrain the design

- **No remote-cursor data path exists.** `NetMsg`, `NetEvent`, `NetOutbound` in
  `crates/robin_engine/src/multiplayer.rs:492-755` carry no pointer position.
- **One reliable/ordered stream per peer.** `crates/robin_rs/src/multiplayer/framing.rs:149-175`;
  wasm uses the same framing over a WebSocket relay (`wasm.rs:1-10`). There is no
  datagram/unordered path, so cursor traffic rides the existing stream. `NetChannels`
  (`multiplayer.rs:758-773`) is a game-loop channel bundle, not a transport QoS selector.
- **`NetFrameClass` is a size-limit tag only** (`framing.rs:8-13,118-147`).
- **Input replication + authoritative snapshots** with predictive rollback
  (`docs/MULTIPLAYER.md:181-228`); `INPUT_DELAY_FRAMES = 2` at 25 Hz
  (`multiplayer.rs:48-53`), tick 40 ms (`engine/mission_runtime.rs:13`).
- **One host-only `CursorRenderer`** (`crates/robin_rs/src/cursor.rs:98-125`),
  swapping among ~60 action cursor IDs (`engine_types/src/resource_ids.rs:46-189`);
  blitted last at raw `ThreadedInput` position (`game_session/render.rs:2014-2038`).
  Tinted draws already exist (`renderer/surfaces.rs:526-533`); scaling does not.
- **No "ability mouse" object** — action state is encoded in the sprite ID, plus
  separate world-space aim overlays (`game_render/hud.rs:280-384`).
- **Coordinates**: local screen point in `ThreadedInput`; `viewport.screen_to_map`
  (`host/viewport.rs:291`) and `viewport.map_to_screen` (`:311`) both exist.
- **Existing per-seat colour palette** for local split-screen cursors:
  `render.rs:1984-1990`, five `[u8;3]` colours — reuse, don't invent a new palette.
- **Presentation-only preference home**: `MultiplayerConfig`
  (`engine_types/src/multiplayer_config.rs:1-35`) is explicitly outside `SimConfig`;
  its native UI is `ingame_menu/multiplayer_privacy.rs`. `GraphicConfig` is profile
  state with bitcode/StateHash derives and is *not* the right home.
- **Protocol bump contract** (refreshed base `e2a07835`): the single `NET_PROTOCOL_VERSION`
  lives at `crates/robin_run_types/src/lib.rs:178` and is 72; `robin_run_protocol`
  derives its constant from it, so bump only that one (to 73) and add its history
  line. The health-check test is `tests/ranked_protocol_schema.rs`. Append new
  `NetMsg` variants at the end so the golden wire bytes test
  (`multiplayer.rs` ~1227, discriminants unchanged) keeps passing.
- **Exhaustive matches needing arms**: `net_frame_class` (`framing.rs:118`),
  `client_outgoing::prepare` (`client_outgoing.rs:19-52`),
  `run_server_outgoing_pump` + `validate_server_gameplay_outbound`
  (`server_dispatch.rs:96-200,336-356`), `NetDrain::handle_event`
  (`game_session/multiplayer.rs:434-664`). Catch-all paths that still need behaviour:
  server inbound whitelist/dispatch (`server.rs:1391-1499`), `client_gameplay::decode`
  (`:35-75`), `Delivery::for_message` (`server_dispatch.rs:20-32`).

## Design consequences

1. Wire payload is `seat + map_x + map_y + sprite_id` — world coords, never screen
   pixels (resolution/DPI/camera-pan safe). Host overwrites `seat` with the
   authenticated seat before relaying.
2. Local cursor sampled at 25 Hz from the same `mouse_map`/`new_cursor` computed by
   `update_mouse_and_cursor` (`render.rs:1132-1259`) — no second hit-test.
3. Interpolation delay 120 ms (3 ticks at 25 Hz) between receive-timestamped
   samples; staleness 300 ms; no extrapolation (mouse direction changes are instant).
4. Presentation-only: never in `PlayerCommand`/`BroadcastInput`/`SimConfig`, so
   determinism, rollback and state hashes are untouched.
5. Single symmetric preference in `MultiplayerConfig` (default off): showing others
   and broadcasting yours. Native UI in the existing multiplayer/privacy modal.
6. Hide when the local pointer is owned by UI: pause menu, UI task, engine
   `user_locked`, console overlay.

## Web sources

- Canva cursor presence engineering (3 updates/s/user, 100k updates/s at scale):
  https://www.canva.dev/blog/engineering/realtime-mouse-pointers
- Liveblocks cursor animation (linear transitions, springs, ~100 ms throttle):
  https://liveblocks.io/blog/how-to-animate-multiplayer-cursors
- Entity interpolation / buffered snapshots (~100 ms constant delay):
  https://www.gabrielgambetta.com/entity-interpolation.html
- Unity Netcode interpolation/extrapolation docs:
  https://docs.unity3d.com/Packages/com.unity.netcode@7.2/manual/interpolation.html
- Overwatch GDC 2017 replication talk (interpolation between last two states):
  https://www.gdcvault.com/play/1024001/-Overwatch-Gameplay-Architecture-and
- Game Accessibility Guidelines — let players choose cursor/colour, reduce clutter:
  https://gameaccessibilityguidelines.com/full-list
