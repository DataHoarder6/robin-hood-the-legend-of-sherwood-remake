# Android Touch Controls Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete issue #58 by connecting the existing typed touch vocabulary to the real recogniser, window emitter, host-only presentation state, quick-group commands, and shared finger-sized hit targets.

**Architecture:** Keep the platform-independent `TouchClassifier` as the gesture authority. Emit ordinary button `1`/`3` mouse events for actions and a new host-only `GameEvent::TouchPressFeedback` for visual pressed state. Store touch modifier mode in `HostFrontend`, cycle it from a small touch-only HUD control, and translate latched portrait taps into the existing `AssignQuickGroup`/`RecallQuickGroup` commands. Reuse `touch_target_rect()` in both world/HUD hit gates.

**Tech Stack:** Rust, winit, existing `robin_rs` input/HUD modules, Python-free unit tests, `scripts/remote-check.sh` on minipc.

**Spec:** GitHub issue #58 and its owner-approved Agent Brief (2026-11-25).

## Global Constraints

- No engine, replay, schema, save-format, network-wire, or state-hash changes.
- Long press threshold is 450 ms; two-finger tap window is 250 ms; slop cancels.
- Secondary actions use existing right-button handlers (`MouseDown`/`MouseUp`, button `3`).
- Press feedback is presentation-only and never produces a `PlayerCommand`.
- Keyboard behavior is unchanged.
- One shared 48×48 logical-pixel minimum hit-target helper; boundary-inclusive.
- No on-device claim: arm64 device/emulator evidence remains BLOCKED.
- Build/test only through `scripts/remote-check.sh` or the documented `--sync` targeted-test recipe on minipc.
- Commit by pathspec; no stash, no shared `main` edits, no `CARGO_TARGET_DIR`.

## Review Focus

- A second finger or slop breach must cancel a pending primary tap without emitting a click.
- Long press and two-finger tap must reach button `3`, not button `1`.
- Press feedback must clear on release, cancel, focus loss, and transform takeover.
- Latch mode must affect touch portrait taps only; keyboard and unlatched taps stay unchanged.
- Padded targets must not create overlaps that steal neighboring world clicks.

---

### Task 1: Recogniser and emitter secondary actions plus press feedback

**Files:**
- Modify: `crates/robin_rs/src/touch_input.rs`
- Modify: `crates/robin_rs/src/window.rs`
- Modify: `crates/robin_rs/src/gfx_types.rs`
- Modify: `crates/robin_rs/src/input.rs`
- Modify: `crates/robin_rs/src/game_session/render.rs`
- Test: existing `touch_input.rs`, `input.rs`, and renderer-facing tests

**Interfaces:**
- Consumes: `TouchClassifier::{started,moved,ended,cancel_all}` and `TouchOutput`.
- Produces: button-aware pointer outputs, host-only `TouchPressFeedback`, and `ThreadedInput::touch_pressed()`.

- [ ] **Step 1: Add failing recogniser tests**
  - Long hold without slop emits secondary button down/up on release.
  - Two contacts released inside 250 ms without movement emit one secondary click at their centroid.
  - Slop or a late second contact emits no secondary click.
  - Every contact emits pressed feedback down/up; cancel and transform takeover clear it.
  - Existing primary tap, drag, double tap, pan/zoom, and ignored-third-pointer tests stay green.

- [ ] **Step 2: Run the focused tests on minipc and capture RED**
  - Use the documented `scripts/remote-check.sh --sync` plus targeted `cargo test` recipe.
  - Expected: new tests fail because `TouchOutput` has no button/press-feedback variants.

- [ ] **Step 3: Extend `TouchOutput` and `TouchClassifier`**
  - Add `button: u8` to pointer down/up outputs and track the active button in drag state.
  - Add `PressFeedback { contact, x, y, pressed }`.
  - Use 450 ms long-press and 250 ms two-finger-tap thresholds.
  - Emit feedback on contact start/end/cancel; suppress it when a transform takes ownership.

- [ ] **Step 4: Map outputs in `window.rs` and add the host-only event**
  - Add `GameEvent::TouchPressFeedback { contact, x, y, pressed }`.
  - Map pointer outputs to `MouseDown`/`MouseUp` with the carried button.
  - Map feedback to the new event; never to a command.

- [ ] **Step 5: Store and consume press feedback**
  - `ThreadedInput` tracks active feedback contacts and exposes `touch_pressed()`.
  - Clear feedback on release, `PointerCancel`, focus loss, and transform takeover.
  - HUD draw sites use `left_mouse_down() || touch_pressed()` for visual `mouse_pressed`.

- [ ] **Step 6: Run focused tests and then `scripts/remote-check.sh --suites format client`**
  - Expected: all focused tests pass; client suite and formatting pass.

- [ ] **Step 7: Commit**
  - `git commit -m "touch: connect secondary gestures and press feedback" -- <task files>`

### Task 2: Touch modifier latch and quick-group portrait commands

**Files:**
- Modify: `crates/robin_rs/src/input_translator.rs`
- Modify: `crates/robin_rs/src/host/frontend.rs`
- Modify: `crates/robin_rs/src/touch_plan_hud.rs`
- Modify: `crates/robin_rs/src/game_session/event_hud.rs`
- Modify: `crates/robin_rs/src/game_session/mouse_input.rs`
- Modify: `crates/robin_rs/src/game_session/render.rs`
- Modify: `crates/robin_rs/src/game_session/report.rs`
- Test: existing frontend/event-HUD/mouse-input tests

**Interfaces:**
- Consumes: `TouchModifierLatch`, `HostFrontend`, portrait hit results, existing `PlayerCommand::{AssignQuickGroup,RecallQuickGroup}`.
- Produces: touch-only latch mode and latched portrait-tap dispatch.

- [ ] **Step 1: Add failing latch and dispatch tests**
  - Mode cycles off → assign → recall → off.
  - Latched portrait tap in assign mode emits `AssignQuickGroup { index = slot }`.
  - Latched portrait tap in recall mode emits `RecallQuickGroup { index = slot }`.
  - Unlatched portrait tap keeps current selection behavior.
  - Secondary gestures ignore the latch; keyboard modifiers are unchanged.
  - HUD hit/render/report tests cover the new control and its padded target.

- [ ] **Step 2: Run focused tests on minipc and capture RED**
  - Expected: mode/control and dispatch tests fail because no runtime latch exists.

- [ ] **Step 3: Implement runtime latch state**
  - Replace the partial Shift-only latch with an explicit touch mode (`Off`, `Assign`, `Recall`).
  - Store it in `HostFrontend`; reset it on modal/focus/session-policy cancellation.
  - Add a touch-only HUD control beside the planning control; short tap cycles mode.

- [ ] **Step 4: Dispatch quick-group commands from portrait taps**
  - In `on_normal_portrait_click`, inspect touch mode before ordinary selection.
  - Assign/recall the portrait slot through existing commands; consume the click.
  - Keep all keyboard and unlatched paths byte-for-byte behaviorally unchanged.

- [ ] **Step 5: Update HUD rendering and cursor reporting**
  - Render the current mode label and pressed state.
  - Include the control in cursor-element reporting and touch hit-testing.

- [ ] **Step 6: Run focused tests and `scripts/remote-check.sh --suites format client client-release`**
  - Expected: all focused tests and both client suites pass.

- [ ] **Step 7: Commit**
  - `git commit -m "touch: latch quick-group assignment and recall" -- <task files>`

### Task 3: Shared finger-sized hit targets and documentation

**Files:**
- Modify: `crates/robin_rs/src/game_session/frame_prepare/input.rs`
- Modify: `crates/robin_rs/src/touch_plan_hud.rs`
- Modify: `crates/robin_rs/src/host/viewport.rs`
- Modify: `android/README.md`
- Modify: `docs/NEW_FEATURES.md`
- Test: existing viewport/touch-plan tests plus world-hit tests

**Interfaces:**
- Consumes: `touch_target_rect()` and existing HUD/world hit-test functions.
- Produces: one shared padded-target policy and current user documentation.

- [ ] **Step 1: Add failing hit-target tests**
  - Zoom, corner, stature, plan/modifier, and Sherwood controls accept points inside the 48×48 padded rect.
  - Points outside the padded rect do not hit.
  - Padding does not make neighboring controls overlap or steal world clicks.
  - `touch_point_is_world` uses the same helper.

- [ ] **Step 2: Run focused tests on minipc and capture RED**
  - Expected: production hit gates fail because they still use raw rects.

- [ ] **Step 3: Replace duplicated raw-rect gates**
  - Use `touch_target_rect()` in `touch_point_is_world`, HUD routing, and touch-plan/modifier hit tests.
  - Preserve existing geometric rendering rects; only hit areas grow.

- [ ] **Step 4: Update Android and feature documentation**
  - Android README names long press, two-finger tap, latch modes, quick-group behavior, 48×48 targets, and press feedback.
  - `docs/NEW_FEATURES.md` records the player-visible touch controls and the blocked on-device evidence.

- [ ] **Step 5: Run focused tests and `scripts/remote-check.sh --suites format client client-release docs`**
  - Expected: all selected suites pass.

- [ ] **Step 6: Commit**
  - `git commit -m "touch: share finger-sized hit targets and document controls" -- <task files>`

---

## Final Verification

- [ ] Run `scripts/remote-check.sh --suites format lint client client-release docs` on the exact final tree.
- [ ] Confirm no engine/replay/schema/save-format files changed.
- [ ] Review the complete diff against `fork/main`.
- [ ] Land through `runbg run NAME -- bash scripts/land.sh`, verify remote read-back, remove the worktree and branch.
