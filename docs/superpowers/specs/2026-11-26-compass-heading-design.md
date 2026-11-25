# Design: compass-heading rotation (second rotation plane)

Status: approved by owner 2026-11-26 (all grill rounds agreed).

## Intent

Keep the shipped ±45° host view rotation exactly as-is. Add a second control that
turns the world compass heading in 15° steps through a full 360°, so the map,
buildings and resources can be viewed from any direction.

## Decisions

- Characters and HUD stay upright; world placement/terrain/sprites rotate.
- 15° steps, wrapping at 360°; one step per `<`/`>` press edge.
- Heading resets on mission load and cutscene; it is not stored in saves or
  replays and never changes engine state, state hashes or peers' views.
- Per-seat in split-screen; local view only.
- Heading composes with the existing view rotation into ONE effective angle
  consumed by renderer, picking, overlays and sprite rows.
- Heading is never refused by the ±45° crispness cap. Baked art may read wrong
  at 180°; that is a documented limitation, not a blocker.
- Small fading heading readout; minimap viewport rotates with it.
- `<`/`>` own Shift+Comma/Period during gameplay; dev step shortcuts move to a
  modifier combination.

## Approaches

1. **Two independent transforms downstream** — rejected: terrain, sprites,
   picking and overlays would drift apart; the GPU contract pins one angle.
2. **Engine compass redefinition** — rejected: changes AI vision, combat and
   replay hashes; violates the host-only boundary.
3. **Composed host angle (chosen)** — both controls update host state; one
   composed angle feeds every existing consumer.

## Constraints / non-goals

No engine state, saves, replays, network, level editor, menu rotation or
startup modal. No re-baking of retail maps. Existing ±45° rotation retains its
cap and tests.

## Verification

Unit tests: step wrap, composition with view rotation, crispness-cap bypass,
reset. Gate: `client-release` suite. Manual: laptop at angle 0 and rotated.
