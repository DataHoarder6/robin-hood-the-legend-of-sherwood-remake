# Map Rotation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add host-only, replay-safe camera rotation to the shipped game and verify it on the `MultiTeamThreeWay` test level.

**Architecture:** One `CameraRotation` (angle + playfield centre) lives in `ViewportState`. Every world→screen mapping — terrain shader, sprites, decals, overlays, picking, split-screen — uses the same rotate-about-centre transform. The engine and its state hashes are untouched; sprites remap their view row host-side only.

**Tech Stack:** Rust, wgpu/WGSL, winit, `robin_rs` client, `scripts/check-quality.sh` suites.

**Spec:** `docs/superpowers/specs/2026-11-21-map-rotation-design.md`

## Global Constraints

- Angle ∈ ±45°, steps of 22.5°; default 0 keeps the current render path bit-identical.
- Playfield centre = `(screen.x / 2, (screen.y - PANNEL_HEIGHT) / 2)` everywhere.
- Rotation state is host-only: never in `SimulationFrameInput`, `BackgroundTransform`, `DebugFlags`, saves or state hashes.
- No `CARGO_TARGET_DIR`, no `--target-dir`, no `git stash`; work only in `.worktrees/map-rotation`.
- Commit by pathspec on `agent/map-rotation`; integration ref is `fork/main`.
- Tests: `nix develop --command cargo test --locked -p robin_rs --lib <name>` for iteration; final gates via `scripts/remote-check.sh client` and `engine`, plus `format` and `lint`.

## Review Focus

- Wrong rotation centre (full screen instead of playfield) → test angle-0 parity and non-zero round-trip against `(screen.x/2, (screen.y-PANNEL_HEIGHT)/2)`.
- Black corners at ±45° → `minimum_zoom` + `clip_view` test that the rotated playfield stays inside level bounds.
- Terrain/sprite drift → shared-helper test asserting CPU sprite bounds equal the shader's inverse lookup; GPU contract test with non-zero angle.
- Directional sprite remap on unmapped rows / projectiles → frame-remap test for mapped and unmapped actions.
- Replay divergence → engine suite + angle-0 state-hash test; no engine file changed by this plan.

---

### Task 1: Camera transform and host viewport state

**Files:**
- Create `crates/robin_rs/src/host/camera_transform.rs`
- Modify `crates/robin_rs/src/host.rs`, `crates/robin_rs/src/host/viewport.rs`

**Interfaces:**
- Produces: `CameraRotation { angle: f32, center: ScreenPoint }`; `map_to_screen(point, view, zoom, rotation) -> ScreenPoint`; `screen_to_map(point, view, zoom, rotation) -> MapPoint`; `rotated_playfield_size(screen: ScreenSize, zoom: f32, angle: f32) -> (f32, f32)`; `minimum_zoom(screen, level, angle) -> f32`; `ViewportState::{camera_angle, display_angle, camera_steps, rotate_by, reset_rotation, advance_rotation, playfield_center}`.

- [ ] **Step 1: Write failing tests in `camera_transform.rs`**

```rust
#[test]
fn rotation_round_trip_and_zero_parity() {
    // For angles 0, FRAC_PI_8, PI_4 and view (12.5, 40.0), zoom 1.5,
    // screen 800x680 -> centre (400,300):
    // screen_to_map(map_to_screen(p)) == p within 1e-3,
    // and at angle 0 map_to_screen(p) == (p - view) * zoom.
}

#[test]
fn positive_angle_rotates_screen_clockwise() {
    // A map point right of centre maps below centre at +PI/4.
}

#[test]
fn rotated_size_and_minimum_zoom_cover_playfield() {
    // At +PI/4 the rotated extent of an 800x680/zoom rect is
    // (1480/zoom, 1480/zoom); minimum_zoom for level 1000x1000 is >= 1.48.
}
```

- [ ] **Step 2: Run tests, expect failures**

Run: `nix develop --command cargo test --locked -p robin_rs --lib rotation_round_trip_and_zero_parity`
Expected: compile failure — module missing.

- [ ] **Step 3: Implement `camera_transform.rs`**

Use screen-space rotation `p' = (x*cos - y*sin, x*sin + y*cos)` in y-down pixels; inverse uses `-angle`. `rotated_playfield_size` returns `(w*|cos|+h*|sin|, w*|sin|+h*|cos|)`. `minimum_zoom` returns the smallest zoom in `0.5..=2.0` whose rotated extent fits the level, warning when the cap is insufficient.

- [ ] **Step 4: Implement viewport state in `viewport.rs`**

Add `camera_angle` (target), `display_angle` (smoothed), `rotate_by(steps: i8)`, `reset_rotation()`, `advance_rotation(now_ms: u32)`, `camera_steps() -> i8` (`round(display_angle / (PI/8))`). Make `screen_to_map_unchecked`/`map_to_screen_unclamped` delegate to `camera_transform`; `clip_view` uses `rotated_playfield_size`; `zoom_by` keeps its mouse anchor through the inverse transform. `adopt_director_camera` stores and zeroes the angle; `advance_director_camera` restores it when `owns_view` goes false.

- [ ] **Step 5: Add viewport tests**

```rust
#[test]
fn rotate_by_steps_and_director_restore() { /* ±22.5° steps, ±45° clamp, reset, director zero + restore */ }
#[test]
fn zoom_anchor_holds_at_rotated_angle() { /* screen point maps to same map point before/after zoom_by */ }
```

- [ ] **Step 6: Run all Task 1 tests, then commit**

Run: `nix develop --command cargo test --locked -p robin_rs --lib camera_transform`
Commit: `git add -- crates/robin_rs/src/host.rs crates/robin_rs/src/host/camera_transform.rs crates/robin_rs/src/host/viewport.rs && git commit -m "feat: add host camera rotation transform" -- <same files>`

### Task 2: Rotation keys

**Files:** Modify `crates/robin_rs/src/input_translator.rs`, `crates/robin_rs/src/game_session/frame_prepare/input.rs`

**Interfaces:**
- Consumes: `ViewportState::rotate_by`, `reset_rotation`, `advance_rotation`.
- Produces: `GameAction::{RotateLeft, RotateRight, ResetRotation}`.

- [ ] **Step 1: Write failing translator test**

```rust
#[test]
fn rotation_keys_translate_once_per_press() {
    // KeyCode::BracketLeft -> RotateLeft, BracketRight -> RotateRight,
    // Backslash -> ResetRotation, each only on release edge.
}
```

- [ ] **Step 2: Run it, expect failure**

Run: `nix develop --command cargo test --locked -p robin_rs --lib rotation_keys_translate_once_per_press`

- [ ] **Step 3: Add `GameKey::{RotateLeft, RotateRight, ResetRotation}` after `SherwoodTrading`**

Bind them in `set_reserved_bindings` to `BracketLeft`, `BracketRight`, `Backslash`; add matching `GameAction` variants emitted under `TranslationFlags::MISSION`.

- [ ] **Step 4: Consume actions in `apply_host_view_input`**

Call `advance_rotation(now_ms)` each frame and map the three actions to `host.frontend.viewport.rotate_by(-1/1)` and `reset_rotation()` inside the existing `view_suppressed` gate.

- [ ] **Step 5: Run test and commit**

Run: `nix develop --command cargo test --locked -p robin_rs --lib rotation_keys_translate_once_per_press`
Commit: `git commit -m "feat: bind camera rotation keys" -- crates/robin_rs/src/input_translator.rs crates/robin_rs/src/game_session/frame_prepare/input.rs`

### Task 3: DrawManager, overlays and sprite placement

**Files:** Modify `crates/robin_rs/src/draw_manager.rs`, `game_render.rs`, `game_render/entities.rs`, `blit_to_map.rs`, `renderer/masks.rs`, `renderer.rs`, `game_session/tick.rs`, `host.rs`; `FramePresentationInputs` gains `rotation: CameraRotation`.

**Interfaces:**
- Consumes: `CameraRotation`.
- Produces: rotation-aware `map_to_screen`, `clip_box`, `map_sprite_bounds`, `sprite_placement`, `render_cached_map_sprite`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn draw_manager_rotates_overlay_points() { /* angle 0 unchanged; +PI/4 maps east point south */ }
#[test]
fn sprite_bounds_are_conservative_at_angle() { /* bounds contain all four rotated corners */ }
#[test]
fn sprite_placement_floor_before_zoom_survives_rotation() { /* angle 0 equals existing expected tuples */ }
```

- [ ] **Step 2: Run them, expect failure**

Run: `nix develop --command cargo test --locked -p robin_rs --lib sprite_bounds_are_conservative_at_angle`

- [ ] **Step 3: Implement the shared transform plumbing**

`DrawManager::{set_rotation, map_to_screen, clip_box}`; `FramePresentationInputs::prepare` copies `host.viewport().camera_rotation()`. Replace direct `((world - view) * zoom)` screen calculations in `entities.rs` with the shared helper; pass `rotation` through all `sprite_placement`, `map_sprite_bounds` and `render_cached_map_sprite` call sites. BBox helpers return the AABB of the four rotated corners.

- [ ] **Step 4: Run tests and commit**

Run: `nix develop --command cargo test --locked -p robin_rs --lib rotation`
Commit by pathspec with message `feat: rotate sprites and overlays with the camera`.

### Task 4: Terrain shader and renderer uniform

**Files:** Modify `crates/robin_rs/shaders/map_quad.wgsl`, `renderer.rs`, `renderer/frame.rs`, `renderer/masks.rs`, `level_loading_host.rs`.

**Interfaces:**
- Consumes: `CameraRotation`, `display_angle`.
- Produces: map draws whose `tint.w` is the angle and whose `ScreenUniform._pad` is `playfield_size`.

- [ ] **Step 1: Write failing CPU-side test for uniform packing**

```rust
#[test]
fn map_draw_packs_angle_and_playfield_size() {
    // render_map_background(view, zoom, 800, 680, PI/4) queues
    // tint == [view.x, view.y, zoom, PI/4]; ScreenUniform._pad == [800.0, 680.0].
}
```

- [ ] **Step 2: Run it, expect failure**

Run: `nix develop --command cargo test --locked -p robin_rs --lib map_draw_packs_angle_and_playfield_size`

- [ ] **Step 3: Implement shader/uniform changes**

In `map_quad.wgsl`, rotate `v.position.xy - screen.playfield_size * 0.5` by `-camera.w` before the existing map-space `floor`; keep quantization in map space. Rename Rust `ScreenUniform::_pad` to `playfield_size` and set it at all three constructors to `(width, height - PANNEL_HEIGHT)` where a map quad is drawn. Add `angle` to `render_map_background` and `render_cached_map_sprite`, packing it into `tint[3]`. `draw_background` passes the viewport angle.

- [ ] **Step 4: Run test, then GPU contract suite**

Run: `nix develop --command cargo test --locked -p robin_rs --lib map_draw_packs_angle_and_playfield_size`
Run: `scripts/remote-check.sh gpu`
Commit by pathspec with message `feat: rotate terrain sampling with the camera`.

### Task 5: Directional sprite frames

**Files:** Modify `crates/robin_rs/src/game_render.rs`, `game_render/entities.rs`.

**Interfaces:**
- Consumes: `CameraRotation::steps`.
- Produces: `current_render_frame(sprite, steps) -> Option<(&SpriteScript, u16, u32)>` with unchanged return at steps 0.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn camera_step_shifts_view_row_by_one() {
    // base row from last_action + direction 3, steps 1 -> row base+4 used.
}
#[test]
fn unmapped_or_out_of_range_row_keeps_current_render_frame() {
    // last_action unmapped, or current_row - base >= 16: use current_row.
}
```

- [ ] **Step 2: Run them, expect failure**

Run: `nix develop --command cargo test --locked -p robin_rs --lib camera_step_shifts_view_row_by_one`

- [ ] **Step 3: Implement host-side row override**

`current_render_frame` resolves `base = sprite.row_for_action(sprite.last_action)`, `direction = current_row - base`, `row = base + (direction + steps).rem_euclid(16)`; falls back to `current_row` when base is unmapped or direction ≥ 16. Never mutate the engine sprite. Pass steps from `host.viewport().camera_steps()` at all three `entities.rs` call sites.

- [ ] **Step 4: Run tests and commit**

Run: `nix develop --command cargo test --locked -p robin_rs --lib camera_step`
Commit by pathspec with message `feat: keep directional sprites facing world directions`.

### Task 6: Split-screen fill and final gates

**Files:** Modify `crates/robin_rs/src/split_screen.rs`, `docs/NEW_FEATURES.md`

**Interfaces:**
- Consumes: `CameraRotation`.
- Produces: per-view `ViewportState` whose focal point lands at its site at non-zero angle.

- [ ] **Step 1: Write failing split-screen test**

```rust
#[test]
fn split_view_centre_stays_at_site_when_rotated() {
    // With angle PI/4, view.viewport(&base).map_to_screen_unclamped(center) == site.
}
```

- [ ] **Step 2: Run it, expect failure**

Run: `nix develop --command cargo test --locked -p robin_rs --lib split_view_centre_stays_at_site_when_rotated`

- [ ] **Step 3: Fix `SplitView::viewport`**

Invert the rotation around the playfield centre when deriving `view_position` from `center` and `site`.

- [ ] **Step 4: Record the feature in `docs/NEW_FEATURES.md`**

One section: keys, angle range, host-only/determinism note, test level name.

- [ ] **Step 5: Run full gates**

Run: `scripts/remote-check.sh client`
Run: `scripts/remote-check.sh engine`
Run: `scripts/remote-check.sh lint`
Run: `scripts/remote-check.sh format`
Expected: all pass. Report any suite that cannot run rather than claiming success.

- [ ] **Step 6: Manual playthrough**

Launch `MultiTeamThreeWay` with rotation, verify click-move, patrols, sight cones, shadows, both rotation directions, reset and no black corners at ±45°. Record the result in the handoff, including anything unverified.

- [ ] **Step 7: Commit and review**

Commit by pathspec with message `feat: finish host-only map rotation`. Branch is ready for review; do not land until the human partner asks.
