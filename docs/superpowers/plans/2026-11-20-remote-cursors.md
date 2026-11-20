# Remote Teammate Cursors Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an opt-in overlay showing co-op teammates' cursor sprites live, smaller and colour-coded per seat, with no effect on deterministic simulation.

**Architecture:** A presentation-only `NetMsg::Cursor` (seat + world coordinates + sprite ID) rides the existing reliable/ordered iroh stream at 25 Hz; the host stamps the authenticated seat and relays it as a diagnostic. Each client buffers receive-timestamped samples per seat, interpolates at a 120 ms delay, and blits a tinted, 0.7× scaled existing cursor sprite using each client's own camera. Nothing crosses into `SimConfig`, `PlayerCommand`, or `BroadcastInput`.

Line numbers in file lists are from refactored base `e2a07835`; prefer the named symbols when lines drift.

**Tech Stack:** Rust workspace, bitcode over iroh QUIC (+ wasm WebSocket relay), wgpu renderer, serde profile config.

**Spec:** `docs/slop/remote-cursors.md` (confirmed design + repo evidence + web sources).

## Global Constraints

- Worktree: `/home/matt/git/robin-hood-remake/.worktrees/teammate-cursors`, branch `feat/teammate-cursors`, base `8eece78e1`. `fork/main` is integration; `land` refuses this repo (ADR 0004) — push the branch and use the repo's own loop.
- Never set `CARGO_TARGET_DIR` or `--target-dir`; no `git stash`; commit by pathspec (`git commit -- <files>`).
- Only test entry point is `bash scripts/check-quality.sh SUITE`; this host cannot compile, so verify with `bash scripts/remote-check.sh SUITE` (suites `engine`, `client`, `format`). Run cargo unfiltered.
- Append every new `NetMsg` variant **after** the last variant; bump the single `NET_PROTOCOL_VERSION` in `crates/robin_run_types/src/lib.rs` 72 → 73 and add its history line. `CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1` is now derived from it — do not edit `robin_run_protocol`.
- No new transport channel, no unreliable delivery, no extrapolation, no dead reckoning.
- Presentation-only: cursor data never enters `SimConfig`, `PlayerCommand`, `BroadcastInput`, replay, rollback, or state hashes.
- New player-visible feature must be recorded in `docs/NEW_FEATURES.md` (repo AGENTS.md §Repo structure).
- Clippy is out of scope for this branch.

## Review Focus

1. **Different resolutions / aspect ratios / camera pan** — a remote cursor must land on the same world point on every seat. Test: `map_to_screen(screen_to_map(p)) ≈ p` roundtrip at two viewport sizes in the `remote_cursor` unit tests.
2. **Jitter, packet delay, out-of-order arrival** — remote cursors must move smoothly and never overshoot on abrupt direction changes. Test: `interpolation_uses_buffered_samples_without_extrapolation` pins a linear lerp at `now - 120` and last-known position when no newer sample exists.
3. **Peer pauses / backgrounds / disconnects** — a stale cursor must disappear, not freeze forever. Test: `stale_seat_is_hidden` and `forget_removes_seat` in `remote_cursor.rs`; `forget` is wired into `NetDrain::handle_event`.
4. **Protocol mismatch after the bump** — old peers must not silently join. Test: existing golden-wire-byte test stays green (append-only variant, `engine` suite) and `ranked_protocol_tracks_exact_runtime_schemas` passes with the single constant at 73 (`client` suite, final gate).
5. **Determinism contamination** — receiving a cursor must not change simulation state or hashes. Test: `NetDrain` cursor arm only calls `RemoteCursorStore::ingest`; a regression test drains a cursor event and asserts no `Input`/rollback/engine mutation.
6. **Local cursor unaffected / UI owns the pointer** — the local cursor stays full-rate and remote cursors vanish under pause/menu/console. Test: `visibility_gating_hides_on_pause_ui_lock_and_console` pins the pure predicate.

---

### Task 1: Evidence doc, wire types, protocol bump

**Files:**
- Create: `docs/slop/remote-cursors.md` (already written; include in commit)
- Modify: `crates/robin_engine/src/multiplayer.rs` (`NetMsg`, `NetEvent`, `NetOutbound`, `decode_msg`, golden test at ~1227)
- Modify: `crates/robin_run_types/src/lib.rs:178` (version constant + history line)
- Test: `crates/robin_engine/src/multiplayer.rs` (test module), `crates/robin_rs/tests/ranked_protocol_schema.rs`

**Interfaces:**
- Produces: `pub struct CursorWireState { pub map_x: f32, pub map_y: f32, pub sprite_id: i32 }` (derives `Clone, Copy, Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode`); `NetMsg::Cursor { seat: PlayerId, state: CursorWireState }` appended last; `NetEvent::TeammateCursor { seat: PlayerId, state: CursorWireState }`; `NetOutbound::Cursor { seat: PlayerId, state: CursorWireState }`; `NET_PROTOCOL_VERSION = 73`.

- [ ] **Step 1: Write the failing test**

In `crates/robin_engine/src/multiplayer.rs` tests add:

```rust
#[test]
fn cursor_wire_state_roundtrips_through_bitcode() {
    let msg = NetMsg::Cursor {
        seat: PlayerId(3),
        state: CursorWireState { map_x: 512.25, map_y: -7.5, sprite_id: 168 },
    };
    assert_eq!(decode_msg(&encode_msg(&msg)).unwrap().to_string(), msg.to_string());
}

#[test]
fn non_finite_cursor_coordinates_are_rejected() {
    let msg = NetMsg::Cursor {
        seat: PlayerId(1),
        state: CursorWireState { map_x: f32::NAN, map_y: 0.0, sprite_id: 1 },
    };
    assert!(decode_msg(&encode_msg(&msg)).is_err());
}
```

Assert via `matches!(decoded, NetMsg::Cursor { seat: PlayerId(3), .. })` plus field equality if `NetMsg` has no `Display`; keep the finite-coordinate assertion either way.

- [ ] **Step 2: Run the test to verify it fails**

Run: `bash scripts/remote-check.sh engine`
Expected: compile failure — `CursorWireState` / `NetMsg::Cursor` do not exist.

- [ ] **Step 3: Add the types**

Append `CursorWireState` and the `NetMsg` variant after the last current variant (verify with a read — the new base has moved since this plan was written); add the `NetEvent` and `NetOutbound` variants at the end of their enums. In `decode_msg`, add a cursor arm rejecting non-finite `map_x`/`map_y` using the module's existing `Err(...)` style. Add a `/// Protocol 73 carries presentation-only teammate cursor state.` line and bump `NET_PROTOCOL_VERSION` 72 → 73 in `crates/robin_run_types/src/lib.rs`; `robin_run_protocol` derives its constant from that one.

- [ ] **Step 4: Run tests to verify they pass**

Run: `bash scripts/remote-check.sh engine`
Expected: PASS, including `shared_payload_variants_keep_golden_wire_bytes` (append-only). `ranked_protocol_tracks_exact_runtime_schemas` lives in the `client` suite and cannot compile until Task 5 adds the exhaustive match arms; it is checked in Task 7.

- [ ] **Step 5: Commit**

```bash
git commit -- crates/robin_engine/src/multiplayer.rs crates/robin_run_types/src/lib.rs docs/slop/remote-cursors.md
```

### Task 2: Tinted, scaled cursor draw

**Files:**
- Modify: `crates/robin_rs/src/cursor.rs:405-469,598-773`
- Test: `crates/robin_rs/src/cursor.rs` test module

**Interfaces:**
- Consumes: `Renderer::render_gpu_image_tinted` (`renderer/surfaces.rs:526-533`).
- Produces: `pub fn render_tinted_scaled(&self, renderer: &mut Renderer, mouse_x: f32, mouse_y: f32, scale: f32, tint: [f32; 4])`; private `fn scaled_cursor_dst_box(mouse_x: f32, mouse_y: f32, hotspot_x: f32, hotspot_y: f32, width: f32, height: f32, scale: f32) -> BBox`.

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn scaled_cursor_dst_box_keeps_hotspot_and_scales_dimensions() {
    let b = scaled_cursor_dst_box(100.0, 80.0, 8.0, 4.0, 16.0, 16.0, 0.5);
    // expect x0=96.0, y0=78.0, width=8.0, height=8.0 — assert via b.min_x()/b.max_x() etc.
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/remote-check.sh client`
Expected: compile failure — helper missing.

- [ ] **Step 3: Implement the draw**

`scaled_cursor_dst_box` offsets the hotspot by `scale` and scales frame width/height. `render_tinted_scaled` mirrors `render_with_effect` but draws the colour layer through `render_gpu_image_tinted(..., BlendMode::Blend, tint)`; no shadow layer for remote cursors. Leave `render`/`render_with_effect` byte-for-byte unchanged.

- [ ] **Step 4: Run test to verify it passes**

Run: `bash scripts/remote-check.sh client`
Expected: PASS, including existing `cursor_pixel_layers_preserve_every_rgb565_value_and_shadow_policy`.

- [ ] **Step 5: Commit**

```bash
git commit -- crates/robin_rs/src/cursor.rs
```

### Task 3: Preference and native UI toggle

**Files:**
- Modify: `crates/robin_engine_types/src/multiplayer_config.rs:9-60`
- Modify: `crates/robin_rs/src/host/frontend.rs:99-177`
- Modify: `crates/robin_rs/src/ingame_menu/multiplayer_privacy.rs:9-140`
- Modify: all eight `FrontendPreferences::new(...)` call sites (`host.rs:65`, `host_mouse.rs:2407`, `frame_simulate.rs:257`, `application/tests.rs:898,928,948,1006`, `game_input/tests.rs:714`)
- Test: `multiplayer_config.rs` and `multiplayer_privacy.rs` test modules

**Interfaces:**
- Consumes: nothing new.
- Produces: `MultiplayerConfig::show_teammate_cursors: bool` with `#[serde(default)]`, `Default = false`; `FrontendPreferences::new(..., multiplayer: &MultiplayerConfig)` and `FrontendPreferences::show_teammate_cursors() -> bool`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn teammate_cursors_default_off_for_fresh_and_partial_profiles() {
    assert!(!MultiplayerConfig::default().show_teammate_cursors);
    let partial: MultiplayerConfig = serde_json::from_str("{}").unwrap();
    assert!(!partial.show_teammate_cursors);
}
```

In `multiplayer_privacy.rs`: `toggling_cursor_preference_commits_only_on_accept` using the existing `state(config)` helper — flip via `activate(ID_CURSORS)`, assert `commit` returns false after cancel and true after accept.

- [ ] **Step 2: Run tests to verify they fail**

Run: `bash scripts/remote-check.sh client` and `bash scripts/remote-check.sh engine` (the config lives in `robin_engine_types`)
Expected: compile failure on the missing field/method.

- [ ] **Step 3: Implement the preference**

Add the field with `#[serde(default)]` to `MultiplayerConfig` and `false` to its `Default`. Add `show_teammate_cursors: bool` to `FrontendPreferences`, set it from the new `&MultiplayerConfig` parameter in `new`, and add the getter. Add `ID_CURSORS: u32 = 201` to the privacy modal, a widget at y=140 with label "Show Teammate Cursors", `draw_widget_radio`, and an `activate` arm toggling `self.working.show_teammate_cursors`. Keep the modal's accept/cancel semantics.

- [ ] **Step 4: Run tests to verify they pass**

Run: `bash scripts/remote-check.sh engine`, then `bash scripts/remote-check.sh client`
Expected: PASS. WASM has no privacy-modal entry point — note as a follow-up in `docs/NEW_FEATURES.md`, not a failure.

- [ ] **Step 5: Commit**

```bash
git commit -- crates/robin_engine_types/src/multiplayer_config.rs crates/robin_rs/src/host/frontend.rs crates/robin_rs/src/ingame_menu/multiplayer_privacy.rs crates/robin_rs/src/host.rs crates/robin_rs/src/host_mouse.rs crates/robin_rs/src/game_session/frame_simulate.rs crates/robin_rs/src/application/tests.rs crates/robin_rs/src/game_input/tests.rs
```

### Task 4: Remote-cursor store (pure presentation logic)

**Files:**
- Create: `crates/robin_rs/src/game_session/remote_cursor.rs`
- Modify: `crates/robin_rs/src/game_session.rs` (module declaration)
- Modify: `crates/robin_rs/src/host/frontend.rs:230-275`
- Test: `crates/robin_rs/src/game_session/remote_cursor.rs` (test module)

**Interfaces:**
- Consumes: `CursorWireState` (Task 1), `CursorRenderer` (`cursor.rs:98`), `ViewportState` (`host/viewport.rs`), `ResourceManager` (robin_assets), `Host`.
- Produces: `pub(crate) const SEAT_COLORS: [[u8; 3]; 5]`; constants `CURSOR_SEND_INTERVAL_MS: u64 = 40`, `CURSOR_INTERPOLATION_DELAY_MS: u64 = 120`, `CURSOR_STALE_MS: u64 = 300`, `REMOTE_CURSOR_SCALE: f32 = 0.7`; `#[derive(Default)] pub(crate) struct RemoteCursorStore` with `ingest(&mut self, now_ms: u64, seat: PlayerId, state: CursorWireState)`, `forget(&mut self, seat: PlayerId)`, `render_states(&self, now_ms: u64) -> Vec<(PlayerId, CursorWireState)>`, `sync_sprites(&mut self, resources: &mut ResourceManager, renderer: &Renderer, states: &[(PlayerId, CursorWireState)])`, `draw(&self, renderer: &mut Renderer, viewport: &ViewportState, states: &[(PlayerId, CursorWireState)])`; `#[derive(Default)] pub(crate) struct LocalCursorTx { pub last_sent_ms: u64 }`; `pub(crate) fn tint_for_seat(seat: PlayerId) -> [f32; 4]`; `pub(crate) fn publish_local_cursor(host: &mut Host, now_ms: u64, map_point: MapPoint, sprite_id: i32)`; `pub(crate) fn remote_cursors_visible(pref: bool, pause: bool, ui_task: bool, user_locked: bool, console: bool) -> bool`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn interpolation_uses_buffered_samples_without_extrapolation() {
    // ingest at t=1000 (x=0), t=1040 (x=40), t=1080 (x=80)
    // render_states(1200) must yield x == 40.0 (120 ms behind the newest sample)
    // render_states(2000) must yield x == 80.0, never > 80.0
}

#[test]
fn stale_seat_is_hidden() { /* newest sample at 1000, render at 1400 -> empty */ }

#[test]
fn forget_removes_seat() { /* after forget, render_states is empty */ }

#[test]
fn seat_colors_are_distinct_and_cover_all_seats() {
    assert_eq!(SEAT_COLORS.len(), 5);
    let unique: std::collections::BTreeSet<_> = SEAT_COLORS.iter().collect();
    assert_eq!(unique.len(), 5);
}

#[test]
fn visibility_gating_hides_on_pause_ui_lock_and_console() {
    assert!(remote_cursors_visible(true, false, false, false, false));
    for flags in [(true, false, false, false), (false, true, false, false),
                  (false, false, true, false), (false, false, false, true)] {
        assert!(!remote_cursors_visible(true, flags.0, flags.1, flags.2, flags.3));
    }
    assert!(!remote_cursors_visible(false, false, false, false, false));
}
```

Add `tint_for_seat(PlayerId(0)) == [r/255, g/255, b/255, 1.0]` derived from `SEAT_COLORS[0]`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `bash scripts/remote-check.sh client`
Expected: compile failure — module does not exist yet.

- [ ] **Step 3: Implement the store**

`ingest` pushes `(now_ms, state)` into a `VecDeque` per seat, caps at 8 samples, drops samples older than `CURSOR_STALE_MS`. `render_states` computes `target = now_ms.saturating_sub(CURSOR_INTERPOLATION_DELAY_MS)`, returns nothing for a seat whose newest sample is older than `CURSOR_STALE_MS`, linearly lerps `map_x`/`map_y` between the two samples bracketing `target` (single-sample case returns that sample), and takes `sprite_id` from the newer bracketing sample. `sync_sprites` calls `CursorRenderer::load_cursor` when a seat's ID changes. `draw` converts each state with `viewport.map_to_screen`, skips off-screen points, and calls `CursorRenderer::render_tinted_scaled` (Task 2) with `REMOTE_CURSOR_SCALE` and `tint_for_seat`. `publish_local_cursor` returns early unless `host.frontend.preferences().show_teammate_cursors()` and local seat exists; throttles on `LocalCursorTx.last_sent_ms` by `CURSOR_SEND_INTERVAL_MS`; sends `NetOutbound::Cursor { seat: host.transport.local_seat(), state }` via `host.transport.net()` and logs a warning on failure (log, never fabricate). Move the five `COLORS` from `render.rs:1984-1990` into `SEAT_COLORS` and have the local split-screen marker use them too. Add `pub remote_cursors: RemoteCursorStore` and `pub(crate) local_cursor_tx: LocalCursorTx` to `HostFrontend`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `bash scripts/remote-check.sh client`
Expected: PASS, including local split-screen cursor tests.

- [ ] **Step 5: Commit**

```bash
git commit -- crates/robin_rs/src/game_session/remote_cursor.rs crates/robin_rs/src/game_session.rs crates/robin_rs/src/host/frontend.rs crates/robin_rs/src/game_session/render.rs
```

### Task 5: Transport plumbing and host relay

**Files:**
- Modify: `crates/robin_rs/src/multiplayer/framing.rs:8-13,118-147`
- Modify: `crates/robin_rs/src/multiplayer/client_gameplay.rs:35-92`
- Modify: `crates/robin_rs/src/multiplayer/client_outgoing.rs:18-52`
- Modify: `crates/robin_rs/src/multiplayer/native/server.rs:1382-1499`
- Modify: `crates/robin_rs/src/multiplayer/native/server_dispatch.rs:13-32,96-204,333-357,545-563`
- Modify: `crates/robin_rs/src/game_session/multiplayer.rs:434-664`
- Test: each module's existing `#[cfg(test)]` block

**Interfaces:**
- Consumes: `NetMsg::Cursor`, `NetEvent::TeammateCursor`, `NetOutbound::Cursor`, `CursorWireState` (Task 1).
- Produces: `pub(super) fn broadcast_teammate_cursor(context: &ServerContext, seat: PlayerId, state: CursorWireState) -> Result<(), MultiplayerError>` in `server_dispatch.rs`; `Delivery::for_message(NetMsg::Cursor { .. }) == Delivery::Diagnostic`.

- [ ] **Step 1: Write failing tests**

In `client_gameplay.rs`: `cursor_wire_msg_decodes_to_presentation_event` asserting `decode(NetMsg::Cursor { seat: PlayerId(2), state })` returns `Ok(NetEvent::TeammateCursor { .. })` with identical fields. In `framing.rs`: `cursor_frame_is_classified_as_control`. In `server.rs` (or `server_dispatch.rs`): `cursor_is_diagnostic_delivery` and `client_cursor_relay_stamps_authenticated_seat` (assert the broadcast payload carries the authenticated `seat`, not a client-supplied one).

- [ ] **Step 2: Run tests to verify they fail**

Run: `bash scripts/remote-check.sh client`
Expected: compile failure on the missing arms / helper.

- [ ] **Step 3: Implement the plumbing**

Add `NetMsg::Cursor { .. }` to the `Control` arm of `net_frame_class`; map `NetOutbound::Cursor { seat, state }` to `NetMsg::Cursor { seat, state }` in `client_outgoing::prepare`; decode it to `NetEvent::TeammateCursor` in `client_gameplay::decode` before its `other =>` arm. Add `NetMsg::Cursor { .. }` to `validate_server_gameplay_wire_msg` and to `apply_authenticated_peer_message`'s dispatch, calling `broadcast_teammate_cursor(context, seat, state)` with the **authenticated** `seat`. Add the `NetOutbound::Cursor` arms to `run_server_outgoing_pump` and `validate_server_gameplay_outbound`, and `Delivery::Diagnostic` to `Delivery::for_message`. In `NetDrain::handle_event`, add a `NetEvent::TeammateCursor { seat, state }` arm that calls only `self.host.frontend.remote_cursors.ingest(crate::window::process_uptime_ms(), seat, state)`; in the existing `DisconnectSeat` arm, call `self.host.frontend.remote_cursors.forget(player_id)`. No engine, input, or rollback mutation in either arm.

- [ ] **Step 4: Run tests to verify they pass**

Run: `bash scripts/remote-check.sh client`
Expected: PASS, including the new tests and `story_regression` NetEvent matches.

- [ ] **Step 5: Commit**

```bash
git commit -- crates/robin_rs/src/multiplayer/framing.rs crates/robin_rs/src/multiplayer/client_gameplay.rs crates/robin_rs/src/multiplayer/client_outgoing.rs crates/robin_rs/src/multiplayer/native/server.rs crates/robin_rs/src/multiplayer/native/server_dispatch.rs crates/robin_rs/src/game_session/multiplayer.rs
```

### Task 6: Wire sampling, visibility, and rendering into the frame

**Files:**
- Modify: `crates/robin_rs/src/game_session/render.rs:1132-1259,1346-1360,1983-2038`
- Modify: `crates/robin_rs/src/game_session/interactive.rs:227-245,831-849,933-977`
- Test: `crates/robin_rs/src/game_session/remote_cursor.rs` (roundtrip test added here)

**Interfaces:**
- Consumes: all Task 1–5 interfaces; `viewport.map_to_screen` (`host/viewport.rs:311`).
- Produces: `pub(crate) fn cursor_screen_point(viewport: &ViewportState, state: &CursorWireState) -> Option<ScreenPoint>` in `remote_cursor.rs`; `hud.remote_cursors_visible: bool` on `MissionHud`; `ctx.remote_cursors_visible: bool` on `RenderContext`.

- [ ] **Step 1: Write failing test**

In `remote_cursor.rs`: `screen_roundtrip_preserves_world_point_at_two_viewport_sizes` — construct a `ViewportState` at 1024×768 and 1600×900, call `cursor_screen_point` on a fixed `CursorWireState`, assert `viewport.screen_to_map(point)` equals the state's `(map_x, map_y)` within 0.01, and assert `None` for an off-viewport point.

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/remote-check.sh client`
Expected: compile failure — `cursor_screen_point` does not exist.

- [ ] **Step 3: Wire the frame**

Add `cursor_screen_point` as a thin wrapper over `viewport.map_to_screen`, and change `RemoteCursorStore::draw` (written in Task 4 with an inline `map_to_screen`) to call it. In `update_mouse_and_cursor`, hoist `mouse_map` out of the `if let` so it is available after cursor selection; after `new_cursor` is final, call `publish_local_cursor(host, now_ms, mouse_map, new_cursor)` when `!engine.user_locked()`, and call `RemoteCursorStore::sync_sprites` with the destructured `cursor_res`/`renderer`. In `prepare_fixed_tick_hud`, set `hud.remote_cursors_visible = remote_cursors_visible(host.frontend.preferences().show_teammate_cursors(), ui.pause_menu.is_some(), ui.active_ui_task.is_some(), engine.user_locked(), ui.console_overlay.is_visible())`. Add the field to `MissionHud` (default `false` in its constructor) and to `RenderContext` (set from `hud` in `render_context`). In `render_frame_with_hud`, before the local cursor blit, when `ctx.remote_cursors_visible`, call `host.frontend.remote_cursors.draw(ctx.renderer, &host.frontend.viewport, &host.frontend.remote_cursors.render_states(now))`. Leave the local cursor draw untouched.

- [ ] **Step 4: Run test to verify it passes**

Run: `bash scripts/remote-check.sh client`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -- crates/robin_rs/src/game_session/render.rs crates/robin_rs/src/game_session/interactive.rs crates/robin_rs/src/game_session/remote_cursor.rs
```

### Task 7: Docs, full gate, manual acceptance

**Files:**
- Modify: `docs/NEW_FEATURES.md`, `docs/MULTIPLAYER.md`

**Interfaces:** none.

- [ ] **Step 1: Record the feature**

Add a `docs/NEW_FEATURES.md` entry: opt-in teammate cursors, where to enable it (Options → Multiplayer / Privacy), default-off, wasm UI follow-up. Add a short note to `docs/MULTIPLAYER.md` that `NetMsg::Cursor` is a host-relayed, presentation-only message on the existing stream and never enters simulation. Do **not** restate the protocol number there — the doc points at `NET_PROTOCOL_VERSION` by design.

- [ ] **Step 2: Run the full gates**

Run: `bash scripts/remote-check.sh format`, then `bash scripts/remote-check.sh engine`, then `bash scripts/remote-check.sh client`.
Expected: all PASS, unfiltered.

- [ ] **Step 3: Manual acceptance (the "worth it" gate)**

Run two native clients plus a headless server with ~150 ms injected latency; confirm remote cursors are smooth (no jitter, no overshoot), vanish on pause/disconnect, never appear if the preference is off, and that state-hash diagnostics report no mismatch. If this fails, fix before landing; do not flip the default on without this passing.

- [ ] **Step 4: Commit and push**

```bash
git commit -- docs/NEW_FEATURES.md docs/MULTIPLAYER.md
git push fork feat/teammate-cursors
```

## Self-review

- Every spec decision has a task: payload/coords (1,3), transport (2), smoothing/staleness (3), visuals (3,4), gating/toggle (5,6), protocol (1), acceptance (7).
- Every new name is used consistently: `CursorWireState`, `NetMsg::Cursor`, `NetEvent::TeammateCursor`, `NetOutbound::Cursor`, `RemoteCursorStore`, `SEAT_COLORS`, `render_tinted_scaled`, `remote_cursors_visible`.
- No task carries another task's code; signatures and test assertions carry the decisions.
- Review Focus items each have a pinning test in the owning task.
