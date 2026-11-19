# Empty box selection is a no-op

2026-11-19. Status: proposed.

## Intent

During melee, a left-drag draws an attack gesture, so players drag constantly.
When combat ends, one or two habitual drags remain. Each becomes a selection
box; over empty ground it clears the hero and allied-soldier selections. The
player reads this as "my character got unselected" and loses their selection
mid-fight.

Make an empty drawn box change nothing. Click and right-drag remain the
deliberate selection changes.

## Goals

- A drawn box that catches no selectable hero and no controllable soldier
  dispatches no selection command. Hero selection, soldier selection, selected
  action, and portrait state are untouched.
- Any hit keeps today's semantics: both selections are replaced unless Shift is
  held.
- Allied soldiers count only when `control_tactical_units` is on.
- No timer, cooldown, or gesture recognition.

## Non-goals

- Click semantics. A drag under the 40 px threshold is a click and still issues
  a group move (`click_ground`, `crates/robin_rs/src/game_input.rs`).
- The meaning of `PlayerCommand::BoxSelect` and `BoxSelectTacticalUnits` in the
  engine, parity traces, console, or mods.
- Touch-specific behaviour. Touch shares the mouse release path.
- Replay schema.

## Anti-goals

- A post-combat grace period. Hidden state, wrong at the edges, and it swallows
  intentional boxes.
- Per-kind no-op. A soldiers-only box would keep the hero selected, so a
  following move order would drag the hero along.
- An engine-level guard. The engine cannot see `control_tactical_units`, so a
  soldiers-only box with the preference off would still clear the hero. It also
  changes deterministic outcomes and forces a replay schema bump.

## Approaches

1. **Input-layer no-op (chosen).** The release path asks the engine whether the
   box catches anything and dispatches neither command when it does not. The
   command keeps its literal meaning; recorded replays keep replaying exactly
   as recorded.
2. **Engine-command no-op.** Rejected: preference invisible to the engine, and
   every existing replay would need rejection.
3. **Post-combat grace timer.** Rejected: hidden state, and it fails when the
   fight ends earlier or later than the timer assumes.
4. **Per-kind no-op.** Rejected: changes normal selection composition.

## Design

- Add a read-only engine query that counts selectable heroes and controllable
  soldiers inside a box, reusing the predicates behind
  `perform_box_selection` (`crates/robin_engine/src/engine/selection.rs`) and
  `box_select_tactical_units`
  (`crates/robin_engine/src/engine/tactical_control.rs`). One helper, two
  callers, so the query and the commands cannot drift.
- In the left-release path
  (`crates/robin_rs/src/game_session/mouse_input.rs`), before dispatching
  `BoxSelect` and `BoxSelectTacticalUnits`: if the hero count is zero and
  either the preference is off or the soldier count is zero, skip both
  commands. The existing gesture cleanup still runs; no `CancelMultiSelection`
  host event is emitted because no command was applied.
- Otherwise dispatch both commands as today.
- Threshold stays `MULTI_SELECTION_THRESHOLD`
  (`crates/robin_engine/src/engine/mod.rs`).

## Left to the builder

- Query name and return shape; where the shared predicate helper lives.
- Log line for a skipped empty box, if any.
- Test placement and suite, per `docs/TESTING.md`.

## Record

- `docs/NEW_FEATURES.md`: the player-visible rule.
- `docs/adr/0001-empty-box-selection-is-not-a-deselect.md`: the seam choice and
  the rejected engine guard.
