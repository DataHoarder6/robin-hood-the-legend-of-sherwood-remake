# New map: feasibility and plan

Last verified: 2026-11-19 against the code and scene files named below.

Question: can this port ship a new standalone skirmish map, and what blocks it?

Verdict: **the engine does not block it; content does.** No map size has to be
enlarged and no engine constant changed. What a new map costs is art, terrain
authoring and a mission script.

Map format, projection, the original production pipeline and the failed
AI-image attempt are documented in [NEW_MAP_IDEA.md](../NEW_MAP_IDEA.md); this
page does not restate them.

## Already solved

- **Size is data.** Grid dimensions derive from the background bitmap; cells are
  64 px. `crates/robin_engine/src/fast_find_grid.rs` (`GRID_CELL_SIZE`,
  `size_map`), `crates/robin_engine/src/engine/level_loading.rs`
  (`initialize_motion_from_level_data`).
- **Authoring exists.** The level editor renders the background from placed 3D
  assets at one pixel per map unit, bakes 1024-pixel tiles and exports a mod ZIP:
  `level-editor/docs/map-compilation.md`.
- **A bigger-than-York custom map already exists.** `level-editor/maps/wychford/`
  is 3600×2400 with 164 placements, a 1,194-vertex ground mesh, a river, a
  curtain wall and 21 guards, published as
  `level-editor/library/scenes/Wychford.rhlos-map.json`. York is 3136×2318
  (`level-editor/library/scenes/york.rhlos-map.json`).
- **Pathfinding and saves do not care about area.** Waypoint-graph A\* with a
  `u32`-link stream for large graphs (`crates/robin_engine/src/pathfinder.rs`);
  the grid is `serde(skip)` in the save (`crates/robin_engine/src/fast_find_grid.rs`).

## Limits that bind

| Limit | Value | Source |
| --- | --- | --- |
| Background texture | one texture, one quad, no tiling; device default 8192 px/side | `crates/robin_rs/src/renderer.rs`, `crates/robin_rs/src/window/gpu.rs` |
| Editor export frame | 16,384 px/side and 64 MP | `level-editor/app/src/map-compile.ts` |
| Fog raster | 4 M texels, downscaled above that | `crates/robin_rs/src/game_render/fog.rs` |
| Anti-collision | `O(actors²)` neighbour scan | `crates/robin_engine/src/engine/anti_collision.rs` |
| Mission authoring | editor authors spawns and soldiers only | `level-editor/maps/wychford/README.md` |

Headroom: York is 7.3 MP. York plus 50% per axis is 4704×3477 = 16.4 MP, inside
both export caps and under the texture limit. The texture limit binds first, at
8192 px on an axis.

## Plan

1. New scene `level-editor/maps/<name>/`, 1792×1280 (28×20 cells), committed like
   Wychford; the mod ZIP is a build artifact, not the source.
2. Layout reference: an orthographic drawing plus heightmap, the
   `terrain.jpg` + `terrain-layout.svg` pattern in `level-editor/maps/wychford/`.
   AI may draft the drawing. It never draws the shipped background.
3. Editor: ground mesh and material regions, road splines, river and bridge,
   curtain wall and gate, interiors, asset placements from the Leicester, Derby
   and Sherwood set, guards on the Mission tab.
4. Spellforge mission script for the two things the editor cannot author:
   patrols (`base.GetPatrol`, `crates/robin_spellforge/src/runtime/bootstrap.lua`)
   and the blazon completion condition.
5. Export, install into the configured mods directory, launch from Custom
   Missions (`level-editor/docs/map-compilation.md`).
6. Verify below.

## Verification gate

- Every route is walkable and both approaches reach the gate; save/load and
  replay a session on the new map.
- Project four known ground points through the fixed projection; the rendered
  background's features land within N pixels of them.
- Two known parallel ground edges stay parallel in the render (no vanishing
  point).
- Water surface height decreases monotonically along the flow direction.

The last three are the failures that a plausible-looking background hides; they
catch the class of defect the AI attempt produced.

## Licensing

- Ship the rendered PNG. Bundle no third-party model files: Quaternius' terms
  forbid redistributing assets as assets, whatever the pack page says.
- CC0, or CC-BY with a `CREDITS.md` naming artist, work, licence and URL.
- AI-drafted reference art: record model, version and prompt, and label it
  "generated, no rights asserted" — prompt-only output is not copyrightable in
  the US, so it cannot honestly be dedicated as CC0.

## Rejected

- **Enlarging an existing map.** It inherits the original's geometry, triggers
  and coordinate-keyed scripts, and still needs new background pixels.
- **AI-generated background.** No image model exposes an orthographic camera;
  scale drift across one large image and seam continuity remain unsolved.
- **Ripped or restrictively licensed art.** A repo that grants downstream
  rights cannot carry NC/ND/SA assets.
