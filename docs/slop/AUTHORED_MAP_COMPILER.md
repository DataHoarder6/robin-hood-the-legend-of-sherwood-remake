# Authored map compiler: design and first slice

**Status:** design, not implementation. Last verified: 2026-11-21.

## Problem

`docs/NEW_FEATURES.md` (Done, "Focused level-editor workspace") records the
TODO: the reconstruction-only game baker cannot compile authored maps, imported
assets, or spline geometry. The editor can author terrain, splines, placed
assets, lights, masks, and doors, but none of it reaches a playable map.

Existing export-prepared navigation is documented in
[`level-editor/docs/navigation-graph-format.md`](../../level-editor/docs/navigation-graph-format.md)
and [`level-editor/docs/map-compilation.md`](../../level-editor/docs/map-compilation.md).

## Ruling: reuse the bake pattern, not the runtime solver

Web prior art (full report: `/tmp/opencode/research/authored-map-compiler.md`)
recommends Recast & Detour (ZLib), Clipper2 (BSL-1.0), and assimp (BSD-3) for
the geometry compiler, citing Unity's `NavMeshSurface` bake asset, Unreal's
streamed navmesh tiles, and Godot's serializable `NavigationMeshSourceGeometryData`.

**Decision: do not adopt those libraries.** This repo deliberately removed its
separate physical navigation solver and graph-free fallback
(`docs/NEW_FEATURES.md`, Done, "Export-prepared navigation"). Introducing a
runtime navmesh would reverse that decision. Reuse the *process* the sources
agree on, not their runtime representation:

- offline bake, serialised artefact, runtime load only;
- content-addressed artefact keyed by descriptor + source hashes + settings +
  tool version;
- versioned descriptor with append-only/optional fields;
- one golden-map CI assertion that the artefact hash is reproducible.

Cost if wrong: a hand-maintained contour compiler is more work than Recast
would be. Benefit: the shipping schema, pathfinder, and replay determinism
stay unchanged.

## First slice

1. A versioned authored-map descriptor: splines as centreline + width contours,
   assets as hash + transform, doors flagged. Extend the existing level JSON,
   not a new format.
2. A headless CLI that bakes only collision contours and the existing route
   graph from that descriptor. No lights, masks, multi-agent bakes, or spline
   deformation in this slice.
3. Hash the artefact over descriptor bytes, input asset hashes, settings, and
   compiler version; write the hash into the artefact manifest.
4. Add one golden-map test to the `editor` suite asserting the hash matches.

Deferred: visual/depth state, state-dependent masks, runtime obstacle carving,
and published map migrations.

## Pitfalls recorded by prior art

- Bake from a simplified navigation contour, never the deformed render spline.
- Fix one float-to-cell rounding rule and align bake chunks with a border
  margin; quantisation otherwise changes connectivity at chunk seams.
- Canonicalise overlapping or nested contours before baking.
- Model doors as runtime carve obstacles or gated off-mesh links, never as
  baked state.
