# 0001 — Empty box selection is not a deselect

Status: accepted 2026-11-20.

## Context

During melee, a left-drag records an attack gesture instead of starting a
selection box; the box path is gated on `is_selected_unit_swordfighting`. When
the fight ends, one or two habitual drags remain. Over empty ground each becomes
a `BoxSelect`, whose engine handler clears the PC and tactical selections before
adding hits. The player loses their hero mid-battle. The original game has no
rule for this — the no-op box is a post-port feature.

## Decision

An empty drawn box is an input no-op, not a deselect. On left release,
`crates/robin_rs/src/game_session/mouse_input.rs` asks the engine for the box's
hit counts and dispatches neither `BoxSelect` nor `BoxSelectTacticalUnits` when
there is no selectable hero and no controllable soldier (soldiers only count with
`control_tactical_units` on). Any hit keeps the existing replace-both semantics,
with Shift unchanged. Click and right-drag remain the deliberate selection
changes.

The read-only query `Engine::box_selection_hits` shares `pc_in_box` and
`soldier_in_box` with the two command implementations, so the gate cannot drift
from what the commands select.

## Alternatives rejected

- **Guard inside the engine commands.** The engine cannot see the host's
  `control_tactical_units` preference, so a soldiers-only box with the
  preference off would still clear the hero. It would also change recorded
  outcomes and force `REPLAY_SCHEMA_VERSION` to 76.
- **Post-combat grace timer.** Hidden state that swallows intentional boxes and
  fails whenever the fight ends earlier or later than the timer assumes.
- **Per-kind no-op.** A soldiers-only box would leave the hero selected, so a
  following move order would drag the hero along.

## Consequences

- `BoxSelect` keeps its literal engine meaning; old replays replay as recorded
  and no schema bump is needed.
- Other callers (console, mods, parity traces) keep the original command
  behaviour.
- The shared predicates are the single source for both the gate and selection;
  changing box hit-testing means changing both through those helpers.

Spec: `docs/superpowers/specs/2026-11-19-empty-box-selection-design.md`.
