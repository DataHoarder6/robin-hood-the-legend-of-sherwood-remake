# Map Rotation Visual Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make host-only map rotation correct in split-screen, keep baked art crisp, and give the player visible feedback when it changes.

**Architecture:** Reuse the one shared camera transform in `host/camera_transform.rs`. Split-screen masking is derived from rotation state; the angle is capped by map size at the input edge so zoom never magnifies past 2.0; a short-lived HUD string uses the existing text renderer. No engine, replay or state-hash change.

**Tech Stack:** Rust, wgpu/WGSL, `robin_rs` client, `scripts/check-quality.sh` suites.

**Spec:** `docs/superpowers/specs/2026-11-22-map-rotation-polish-design.md`

## Global Constraints

- Deferred feature: no gates, no rebase, no landing until 0.3.0 has shipped and the owner asks.
- Never build with cargo/rustc on OCI. Suites only via `ROBIN_BUILD_HOST=minipc scripts/remote-check.sh SUITE`; use `runbg` + `runbg wait`.
- No `CARGO_TARGET_DIR`, no `--target-dir`, no `git stash`; commit by explicit pathspec on `agent/map-rotation`.
- Angle 0 stays bit-identical: CPU fast paths and the shader zero-angle branch must survive every task.
- No rotation state in engine, replay input, saves or state hashes.
- Format with the pinned rustfmt (`rustup which rustfmt --edition 2024`); a foreign formatter has already touched this tree once.
- Tests are written now but **cannot be run while 0.3.0 is landing**. Each "run" step below records the command to use when unblocked; never claim a pass before it runs.

## Review Focus

- Two-player split-screen at ±45° → no black bars painted over the rotated map. Test: Task 1 `split_edge_bars_are_empty_when_rotated`.
- A small map with a rotation keypress → the step is refused rather than zooming past 2.0 into mushy pixels. Test: Task 2 `small_level_rotation_refuses_steps_beyond_crisp_zoom`.
- Window resize after rotating → readout and coverage follow the new screen size. Test: Task 2 `rotation_cap_recomputed_after_resize`.
- Replay playback with a rotation keypress → no action is recorded and no stale readout appears. Test: Task 3 `readout_expires`.
- A map with no occlusion depth → no parallax code runs or panics. Test: Task 4 spike's explicit no-data case.

---

### Task 1: Split-screen edge mask follows rotation

**Files:**
- Modify `crates/robin_rs/src/split_screen.rs` (add helper + tests)
- Modify `crates/robin_rs/src/game_session/render.rs:1448-1471` (call helper)

**Interfaces:**
- Consumes: `ViewportState::{camera_angle, level_size, screen_size, view_position, zoom_factor}`; `MapBBox`.
- Produces: `pub(super) fn split_edge_bars(viewport: &ViewportState) -> [(i32, i32, i32, i32); 4]`, returning four `(x, y, w, h)` rects (zero-sized when unused).

- [ ] **Step 1: Write failing tests in `split_screen.rs`**
  - `split_edge_bars_match_legacy_rects_at_angle_zero`: level 3136×1984, screen 1024×768, `view_position = (10, 20)`, zoom 1.0; assert the four rects equal the values the current inline formulas produce (compute them in the test with the same formulas).
  - `split_edge_bars_are_empty_when_rotated`: same viewport after `rotate_by(1)` and `advance_rotation(u32::MAX)`; assert all four rects are `(0,0,0,0)`.
  - `split_edge_bars_are_empty_without_level_size`: `level_size = ZERO`; assert all zero.

- [ ] **Step 2: Run the tests (deferred until gate available)**
  Run: `ROBIN_BUILD_HOST=minipc scripts/remote-check.sh client`
  Expected before implementation: compile failure — `split_edge_bars` missing.

- [ ] **Step 3: Implement `split_edge_bars`**
  At angle 0 (and when level size is zero) return the four rects from the existing inline formulas. When `camera_angle != 0`, return four zero rects: rotation coverage in `clip_view` already guarantees every rotated corner is inside the level, so no out-of-map area exists. Document that invariant in the function comment.

- [ ] **Step 4: Replace the inline block in `render.rs` with a call to `split_edge_bars(&viewport)`**; keep the existing `w > 0 && h > 0` guard and `render_gpu_rect` call.

- [ ] **Step 5: Run the tests (deferred)** — same command; expected PASS. Do not claim a pass before running.

- [ ] **Step 6: Commit** by pathspec: `git commit -m "fix: keep split-screen edge bars out of rotated views" -- crates/robin_rs/src/split_screen.rs crates/robin_rs/src/game_session/render.rs`

---

### Task 2: Cap the rotation step by map size

**Files:**
- Modify `crates/robin_rs/src/host/camera_transform.rs` (add `rotation_step_allowed`)
- Modify `crates/robin_rs/src/host/viewport.rs` (`rotate_by`, `rotation_min_zoom`, tests)
- Modify `crates/robin_rs/src/host.rs` (re-export)

**Interfaces:**
- Consumes: `minimum_zoom(screen: ScreenSize, level: MapSize, angle: f32) -> f32`.
- Produces: `pub(crate) fn rotation_step_allowed(screen: ScreenSize, level: MapSize, angle: f32) -> bool` — true when `minimum_zoom(...) <= 2.0`.

- [ ] **Step 1: Replace the existing `small_level_rotation_zooms_past_the_usual_cap` test in `viewport.rs`**
  New name: `small_level_rotation_refuses_steps_beyond_crisp_zoom`. Level 200×200, screen 800×680. After `rotate_by(2)`, assert `camera_angle() == 0.0` and `zoom_factor <= 2.0`. After `advance_rotation(0); advance_rotation(u32::MAX); zoom_by(0.5, None)`, assert `zoom_factor <= 2.0`.

- [ ] **Step 2: Add `rotation_cap_recomputed_after_resize`** — level 1000×1000, rotate one step at screen 800×680 (allowed), then `set_screen_size(320.0, 240.0)`; assert a further `rotate_by(1)` is refused and `zoom_factor <= 2.0`.

- [ ] **Step 3: Add a unit test in `camera_transform.rs`** — `rotation_step_allowed` is false for angle `FRAC_PI_4` on a 200×200 level and true on a 1000×1000 level; true for angle 0 on both.

- [ ] **Step 4: Run the tests (deferred)** — `ROBIN_BUILD_HOST=minipc scripts/remote-check.sh client`; expected failures until implementation.

- [ ] **Step 5: Implement `rotation_step_allowed` and wire it into `rotate_by`**
  In `rotate_by`, compute the candidate angle first; if `rotation_step_allowed(screen_size, level_size, candidate)` is false, return without changing state. When level size is zero, refuse the step (no coverage to guarantee). Keep `reset_rotation` and `adopt_director_camera` untouched. Keep `apply_rotation_constraints` as the safety net for a later resize.

- [ ] **Step 6: Run the tests (deferred)** — same command; expected PASS.

- [ ] **Step 7: Commit** by pathspec: `git commit -m "fix: cap rotation by map size to keep baked art crisp" -- crates/robin_rs/src/host/camera_transform.rs crates/robin_rs/src/host/viewport.rs crates/robin_rs/src/host.rs`

---

### Task 3: Short-lived angle readout

**Files:**
- Modify `crates/robin_rs/src/host/viewport.rs` (readout deadline + pure expiry helper)
- Modify `crates/robin_rs/src/game_session/render.rs` (draw call near the other HUD text at ~line 2589)
- Modify `crates/robin_rs/src/game_session/event_hud.rs` (call `set_rotation_readout` after each rotation key)

**Interfaces:**
- Consumes: `ViewportState::camera_angle()`, `crate::window::process_uptime_ms()`.
- Produces: `rotation_readout_visible(now_ms: u32, until_ms: Option<u32>) -> bool`; `ViewportState::rotation_readout_until() -> Option<u32>`; `ViewportState::set_rotation_readout(now_ms: u32)`.

- [ ] **Step 1: Write failing tests in `viewport.rs`**
  - `readout_expires`: `None` → false; `Some(1000)` at now 999 → true; at 1000 → false; with wraparound `now_ms = 0` and `until = u32::MAX` → true.
  - `set_rotation_readout_sets_deadline`: after `set_rotation_readout(1000)`, `rotation_readout_until() == Some(2200)`; `reset_rotation()` must clear the deadline to `None`.

- [ ] **Step 2: Run tests (deferred)** — `ROBIN_BUILD_HOST=minipc scripts/remote-check.sh client`; expected compile failure.

- [ ] **Step 3: Implement**
  Add `readout_until_ms: Option<u32>` to `ViewportState`, initialised `None`, plus `set_rotation_readout(now_ms)` setting it to `now_ms.saturating_add(1200)` and clearing it in `reset_rotation`. Add the pure `rotation_readout_visible` helper with wrap-safe comparison. Call `set_rotation_readout` from `event_hud.rs` right after each `rotate_by`/`reset_rotation`, using the `now_ms` already sampled there. Do not store the deadline in engine state.

- [ ] **Step 4: Draw the string** in `render.rs` when `rotation_readout_visible(...)`, via `crate::hud_text::render_text_with_shadow_gpu(renderer, font, shadow, &format!("{}°", (angle.to_degrees()).round() as i32), x, y)` using the existing `HudFonts` and a fixed screen corner. No new font loading, no new HUD surface.

- [ ] **Step 5: Run tests (deferred)** — expected PASS.

- [ ] **Step 6: Commit** by pathspec: `git commit -m "feat: show a fading camera angle readout" -- crates/robin_rs/src/host/viewport.rs crates/robin_rs/src/game_session/render.rs crates/robin_rs/src/game_session/event_hud.rs`

---

### Task 4: Depth parallax spike (blocked until a build is available)

**Files:** scratch only under `/tmp/opencode/`; no product file until the spike is accepted.

- [ ] **Step 1: Decide feasibility by reading only** — `renderer/resources.rs:373` uploads a continuous occlusion depth texture; confirm whether the terrain quad shader can sample it alongside the sampled atlas.
- [ ] **Step 2: When the build host is available, build a throwaway spike** that offsets the sampled map pixel by a few units proportional to depth and `sin(angle)`. Inspect it in `MultiTeamThreeWay` at ±22.5° and ±45°.
- [ ] **Step 3: Decide** — keep only if it reads as depth and sprites stay attached to terrain. If kept, write a new spec/plan for the product change; if not, record the negative result in `docs/slop/` and stop. No tests or commits for a rejected spike.

---

### Task 5: Final static check and handoff

- [ ] **Step 1: Run pinned rustfmt check** on every changed `.rs` file: `rf=$(rustup which rustfmt); "$rf" --check --edition 2024 <files>`.
- [ ] **Step 2: Confirm no engine/replay file changed** — `git diff fork/main...HEAD --name-only` must contain no `crates/robin_engine/`.
- [ ] **Step 3: Update `docs/NEW_FEATURES.md`** with the readout, the angle cap and the split-screen fix; commit by pathspec.
- [ ] **Step 4: Report clearly what was and was not verified.** No landing until the owner says 0.3.0 has shipped.
