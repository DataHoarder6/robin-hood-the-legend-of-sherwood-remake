# Map rotation — design (2026-11-21)

## Intent

Make camera rotation a finished, playable feature: a real mission can be
rotated a little without changing gameplay. Proof of concept is the feature
itself, not a tilted bitmap.

## Goals

- Rotate the rendered world around the viewport centre by small angles while
  the simulation, pathfinding, AI and replays stay byte-for-byte identical.
- Keep terrain, sprite decals, character sprites, shadows, overlays and mouse
  picking in one coordinate transform.
- Characters keep facing the direction they actually face in the world after
  the camera turns.
- Default angle 0: untouched builds render and play as before.

## Non-goals

- No true 3D camera orbit. The terrain is one baked bitmap with a fixed
  projection; rotation cannot reveal new faces of buildings.
- No new map art or new level. The demo mission is the test level.
- No rotation of the HUD, menus, or split-screen divider.
- No rotation state in the engine, save games, replays or state hashes.

## Anti-goals

- Rotating only the terrain shader: sprites and clicks detach from the map.
- Rotating simulation coordinates: breaks pathfinding parity and replays.
- Any change that makes the default (angle 0) path differ from today.

## Constraints

- The renderer is hand-rolled wgpu; the terrain shader's `camera` uniform is
  `vec4` with its fourth slot unused — carry the angle there, no uniform
  layout change.
- Existing projection is fixed `map_x = x`, `map_y = y - z`
  (`docs/NEW_MAP_IDEA.md`, lines 134-153). Do not change it.
- Directional sprites and the engine camera stay as they are; rotation is a
  host presentation concern (`crates/robin_rs/src/host/viewport.rs:1-7`
  already separates host from engine camera).
- Build/test only through `scripts/check-quality.sh` suites, via
  `scripts/remote-check.sh` as required by `AGENTS.md`. Never set
  `CARGO_TARGET_DIR`.
- Original game data is required for a real playthrough; `ROBINHOOD_DATA_DIR`
  must be supplied by the owner for that step.

## Approaches considered

1. **Host-only presentation transform (chosen).** One shared map↔screen
   helper carries the angle; terrain shader, sprites, decals, overlays and
   picking use it. Sprite view frames are remapped by the same angle. Engine
   untouched, so replay parity is testable.
2. **New purpose-built level.** Needs a baked map bitmap, `.rhp` geometry,
   sectors and patrols. `docs/NEW_MAP_IDEA.md:179-181` records that polished
   new art is not low-effort; rejected.
3. **Simulation-space rotation.** Would change pathfinding and state hashes;
   rejected.

## Decided design

- Add a host-only `camera_angle: f32` (radians, default 0) to `ViewportState`.
  Range ±45°, in 22.5° steps — one step equals one directional sprite frame.
- Controls are host-only keys `[` / `]` (step ±22.5°) and `\` (reset).
  They are read through the existing physical-key edge path, not `GameKey`,
  so they never enter replay input. Smooth the rendered angle over ~150 ms;
  sprite view frames snap by the matching number of steps.
- Put rotation in one new module, `crates/robin_rs/src/host/camera_transform.rs`:
  `map_to_screen`, `screen_to_map`, and the rotated visible-area bound.
  `ViewportState`, `draw_manager`, `game_render`, `blit_to_map`, mouse picking
  and the renderer all call it; no second copy of the trigonometry.
- Terrain shader rotates screen pixels about the viewport centre before the
  existing map-space `floor` and texture lookup, using the unused `camera.w`.
- Culling and `clip_view` use a conservative rotated bound so no visible
  sprite disappears at non-zero angles.
- Character view-frame selection adds the camera step count to the existing
  world-direction frame index. Exact call site is for the plan to locate;
  behaviour is pinned by a test that asserts the same world direction picks
  the frame offset by the camera steps.
- Shadows baked into the map rotate with it; any separately drawn shadow
  sprite goes through the same placement helper.

## Acceptance

- `scripts/check-quality.sh client` and `engine` pass.
- Unit tests: map→screen→map round-trip at 0, ±22.5°, ±45°; zoom stays
  anchored under the cursor at non-zero angles; rotated culling keeps a
  corner sprite visible; view-frame remap matches the camera step count;
  replay state hashes are unchanged with angle 0.
- Manual playthrough of the Leicester demo mission with a real
  `ROBINHOOD_DATA_DIR`: rotate in both directions, reset, click-move a
  character, watch a patrol and a sight cone, check shadows stay attached.
  If game data is unavailable, report that verification as blocked — do not
  claim the feature is verified.

## Left to the builder

- Exact call site for directional sprite frame selection and for any
  separately rendered shadow sprites.
- Exact place in the frame-prepare input path for the two bracket keys.
- Which existing debug-text mechanism, if any, shows the angle readout;
  omit it if none exists.
- Worktree, plan and implementation branch: `agent/map-rotation`.
