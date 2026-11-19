# Empty Box Selection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a drawn selection box that catches no selectable unit dispatch no command, so a leftover drag after combat no longer clears the selection.

**Architecture:** One read-only engine query counts selectable heroes and controllable soldiers in a box, sharing per-entity predicates with the two existing selection commands. The left-release path in the client skips both commands when the counts are empty for the active preferences. Engine command semantics and the replay schema are unchanged.

**Tech Stack:** Rust workspace, no new dependencies. Tests run through `scripts/remote-check.sh`.

**Spec:** `docs/superpowers/specs/2026-11-19-empty-box-selection-design.md`

## Global Constraints

- Threshold stays `MULTI_SELECTION_THRESHOLD = 1600.0` (`crates/robin_engine/src/engine/mod.rs`).
- No timer, cooldown, or gesture recognition.
- No `REPLAY_SCHEMA_VERSION` change.
- `PlayerCommand::BoxSelect` and `BoxSelectTacticalUnits` keep their current engine semantics.
- Soldiers count only when `control_tactical_units` is on (default off).
- The guard applies on left release only.

## Review Focus

1. Soldiers-only box with the preference off — must be a no-op, not a hero clear. Pinned by the decision-function test in Task 2.
2. Shift held on an empty box — must stay a no-op. Pinned by the shift release test in Task 2.
3. A dead or downed hero inside the box — must not count as a hit. Pinned by the dead-PC query test in Task 1.
4. Hero-only box with the preference off — must still dispatch `BoxSelect`. Pinned by the decision-function test in Task 2.
5. A hero another seat controls under Exclusive coop — must not count for this seat. Pinned by the seat-filter query test in Task 1.

---

### Task 1: Engine box hit query

**Files:**
- Modify: `crates/robin_engine/src/engine/selection.rs` (add `BoxSelectionHits`, `pc_in_box`, `box_selection_hits`; make `perform_box_selection` use `pc_in_box`)
- Modify: `crates/robin_engine/src/engine/tactical_control.rs` (add `soldier_in_box`; make `box_select_tactical_units` use it)
- Modify: `crates/robin_engine/src/engine/mod.rs` (re-export `BoxSelectionHits` beside `pub use selection::Stature;`)
- Test: `crates/robin_engine/src/engine/selection.rs`, `#[cfg(test)]` module

**Interfaces:**
- Consumes: `is_pc_selectable(&self, assets: &LevelAssets, id: EntityId) -> bool`, `coop_can_select(&self, seat: usize, id: EntityId) -> bool`, `is_tactically_controllable(&self, id: EntityId) -> bool`, `crate::sprite::BBox`.
- Produces:

```rust
pub struct BoxSelectionHits {
    pub heroes: usize,
    pub soldiers: usize,
}

impl EngineInner {
    pub fn box_selection_hits(
        &self,
        seat: usize,
        assets: &LevelAssets,
        pt1: MapPoint,
        pt2: MapPoint,
    ) -> BoxSelectionHits;
}
```

- [ ] **Step 1: Write the failing tests**

In `crates/robin_engine/src/engine/selection.rs`'s test module, add:

```rust
use crate::coordinates::MapPoint;

fn place_at(engine: &mut EngineInner, id: EntityId, x: f32, y: f32) {
    let element = engine.get_entity_mut(id).unwrap().element_data_mut();
    element.set_position_map(MapPoint::new(x, y));
    element.sprite.position_iface.settle_current_position();
}

#[test]
fn box_selection_hits_counts_a_selectable_hero_inside_and_none_outside() {
    let assets = LevelAssets::default();
    let mut engine = EngineInner::new();
    let hero = add_selectable_test_pc(&mut engine);
    place_at(&mut engine, hero, 50.0, 50.0);
    assert_eq!(
        engine
            .box_selection_hits(0, &assets, MapPoint::new(40.0, 40.0), MapPoint::new(60.0, 60.0))
            .heroes,
        1
    );
    assert_eq!(
        engine
            .box_selection_hits(0, &assets, MapPoint::new(200.0, 200.0), MapPoint::new(220.0, 220.0))
            .heroes,
        0
    );
}

#[test]
fn box_selection_hits_ignores_a_dead_hero() {
    let assets = LevelAssets::default();
    let mut engine = EngineInner::new();
    let hero = add_selectable_test_pc(&mut engine);
    place_at(&mut engine, hero, 50.0, 50.0);
    engine.get_entity_mut(hero).unwrap().pc_data_mut().unwrap().life_points = 0;
    assert_eq!(
        engine
            .box_selection_hits(0, &assets, MapPoint::new(40.0, 40.0), MapPoint::new(60.0, 60.0))
            .heroes,
        0
    );
}

#[test]
fn box_selection_hits_counts_a_controllable_soldier() {
    use crate::element::AiBrain;
    use crate::engine::test_support::actors::make_test_soldier;

    let assets = LevelAssets::default();
    let mut engine = EngineInner::new();
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(data) = &mut soldier else { unreachable!() };
    data.element.active = true;
    data.element.set_position_map(MapPoint::new(50.0, 50.0));
    data.soldier.command_interface = crate::human_control::CommandInterface::TacticalOrders;
    data.npc.ai_brain = AiBrain::Enemy(Box::default());
    data.npc.life_points = 100;
    engine.add_test_entity(soldier);
    assert_eq!(
        engine
            .box_selection_hits(0, &assets, MapPoint::new(40.0, 40.0), MapPoint::new(60.0, 60.0))
            .soldiers,
        1
    );
}

#[test]
fn box_selection_hits_excludes_a_hero_another_seat_controls() {
    let assets = LevelAssets::default();
    let mut engine = EngineInner::new();
    engine.control.sim_config.coop.control = crate::coop::CharacterControl::Exclusive;
    let hero = add_selectable_test_pc(&mut engine);
    place_at(&mut engine, hero, 50.0, 50.0);
    engine.players.seats[1].connected = true;
    engine.players.seats[1].selection = vec![hero];
    assert_eq!(
        engine
            .box_selection_hits(0, &assets, MapPoint::new(40.0, 40.0), MapPoint::new(60.0, 60.0))
            .heroes,
        0
    );
    assert_eq!(
        engine
            .box_selection_hits(1, &assets, MapPoint::new(40.0, 40.0), MapPoint::new(60.0, 60.0))
            .heroes,
        1
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `scripts/remote-check.sh engine`
Expected: FAIL — `no method named box_selection_hits` and unresolved `BoxSelectionHits`.

- [ ] **Step 3: Implement the query and the shared predicates**

In `selection.rs`:

```rust
pub struct BoxSelectionHits { pub heroes: usize, pub soldiers: usize }

impl EngineInner {
    pub fn box_selection_hits(&self, seat: usize, assets: &LevelAssets, pt1: MapPoint, pt2: MapPoint) -> BoxSelectionHits {
        let box_rect = selection_box(pt1, pt2);
        BoxSelectionHits {
            heroes: self.world.pc_ids.iter().filter(|&&id| self.pc_in_box(assets, seat, id, &box_rect)).count(),
            soldiers: self.world.entities.npc_ids().filter(|&&id| self.soldier_in_box(id, &box_rect)).count(),
        }
    }
}
```

Add `fn selection_box(pt1, pt2) -> BBox` holding the min/max normalization now duplicated in both commands. Add `pc_in_box` with the exact predicates and geometry currently inside `perform_box_selection` (selectable + coop + sprite AABB intersect or centre point). Replace that body's per-PC test with `pc_in_box`, keeping the loop order and the `!shift` clear. In `tactical_control.rs`, add `soldier_in_box` with the current filter (controllable + centre point) and use it in `box_select_tactical_units`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `scripts/remote-check.sh engine`
Expected: PASS, including the four new tests.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(engine): expose box selection hit counts" -- crates/robin_engine/src/engine/selection.rs crates/robin_engine/src/engine/tactical_control.rs crates/robin_engine/src/engine/mod.rs
```

---

### Task 2: Client guard on empty box release

**Files:**
- Modify: `crates/robin_rs/src/game_session/mouse_input.rs` (left-release box branch, around the `multi_selection_active() && draw_multi_selection()` arm)
- Test: `crates/robin_rs/src/game_session/mouse_input.rs`, new `#[cfg(test)] mod box_selection_tests`

**Interfaces:**
- Consumes: `Engine::box_selection_hits(seat, assets, pt1, pt2) -> BoxSelectionHits` from Task 1.
- Produces: `fn box_selects_anything(hits: &BoxSelectionHits, control_tactical_units: bool) -> bool` (private to `mouse_input.rs`).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn empty_box_release_dispatches_nothing() {
    // fixture(); add_pc_with_status at (50.0, 50.0)
    // start_multi_selection(MapPoint::new(500.0, 500.0)); latch_selection_outline();
    // on_left_mouse_up(...)
    assert!(commands.is_empty(), "empty box must dispatch no command");
}

#[test]
fn shift_empty_box_release_dispatches_nothing() {
    // same, with modifiers.shift = true
    assert!(commands.is_empty());
}

#[test]
fn box_over_a_hero_dispatches_box_select() {
    // hero at (50.0, 50.0); box (40,40)-(60,60)
    assert!(commands.commands.iter().any(|input| matches!(input.command, PlayerCommand::BoxSelect { .. })));
}

#[test]
fn box_selects_anything_requires_the_preference_for_soldiers() {
    let soldiers_only = BoxSelectionHits { heroes: 0, soldiers: 3 };
    assert!(!box_selects_anything(&soldiers_only, false));
    assert!(box_selects_anything(&soldiers_only, true));
    assert!(box_selects_anything(&BoxSelectionHits { heroes: 1, soldiers: 0 }, false));
}
```

Use the existing `MouseCtx` construction pattern from `shift_planning_tests`; `Host::scratch` already has `control_tactical_units` off.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `scripts/remote-check.sh client`
Expected: FAIL — empty-box tests dispatch `BoxSelect`; `box_selects_anything` is unresolved.

- [ ] **Step 3: Implement the guard**

```rust
fn box_selects_anything(hits: &BoxSelectionHits, control_tactical_units: bool) -> bool {
    hits.heroes > 0 || (control_tactical_units && hits.soldiers > 0)
}
```

In the release arm, before building the commands:

```rust
let pt1 = host.frontend.input.multi_selection_pt1();
let pt2 = host.frontend.input.multi_selection_pt2();
let hits = engine.box_selection_hits(local_seat.0 as usize, assets, pt1, pt2);
if box_selects_anything(&hits, host.frontend.preferences().control_tactical_units()) {
    // existing BoxSelect + BoxSelectTacticalUnits dispatch
} else {
    tracing::debug!("empty box selection ignored");
}
```

The existing `cancel_selection_gestures()` cleanup after the branch stays where it is.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `scripts/remote-check.sh client`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(client): ignore empty box selections" -- crates/robin_rs/src/game_session/mouse_input.rs
```

---

### Task 3: Record the rule

**Files:**
- Modify: `docs/NEW_FEATURES.md` (new bullet at the top, matching the existing dense style)
- Create: `docs/adr/0001-empty-box-selection-is-not-a-deselect.md`

- [ ] **Step 1: Write the `NEW_FEATURES.md` entry**

State: a left-drag selection box that catches no selectable hero and no controllable allied soldier dispatches no selection command; hero and soldier selections, selected action, and portraits are untouched; a box with any hit keeps replacing both selections unless Shift is held; click and right-drag remain the deliberate selection changes. Name `control_tactical_units` as the gate for soldiers.

- [ ] **Step 2: Write the ADR**

`docs/adr/0001-empty-box-selection-is-not-a-deselect.md`: context (post-combat drag habit), decision (input-layer no-op), rejected alternatives (engine guard, grace timer, per-kind no-op), consequences (engine command unchanged, no replay schema bump, new read-only query). Link the spec.

- [ ] **Step 3: Verify the links resolve**

Run: `rg -n "empty box|empty-box" docs/NEW_FEATURES.md docs/adr/0001-empty-box-selection-is-not-a-deselect.md docs/superpowers/specs/2026-11-19-empty-box-selection-design.md`
Expected: each named path exists; no restated fact beyond the single source.

- [ ] **Step 4: Commit**

```bash
git commit -m "docs: record empty box selection rule" -- docs/NEW_FEATURES.md docs/adr/0001-empty-box-selection-is-not-a-deselect.md
```
