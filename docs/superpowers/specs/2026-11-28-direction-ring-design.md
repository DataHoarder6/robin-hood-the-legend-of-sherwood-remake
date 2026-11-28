# Design: full 15° direction ring

Status: approved by owner 2026-11-28 ("ok lets do it" after the three recommended next steps).

## Intent

Let the player change the view orientation in-game by 15° steps in both directions, at any point in a session, and see the world/assets from that direction — not a screen-space turn of one baked image.

Shipped baseline (`a56739d7c`): `ViewportState::direction_step()` (0..24), Shift+Comma/Period step both signs, host-only; `-dir1.map.png` alternate background selected for steps 1..23; editor bake supports `camera.yaw_deg`.

## Approaches considered

1. **Runtime 3D re-render** — rejected: no 3D sources in tracked repos for the runtime, and no renderer path for arbitrary camera azimuth.
2. **One alternate bake for all nonzero steps** — already shipped; rejected as incomplete because it shows the same second wall at every heading.
3. **24 pre-baked direction sets, host-selected per step** — chosen. Extends the shipped palette mechanism; no engine/replay/hash changes.

## Decisions (builder's calls)

- 24 baked terrain PNGs per playable map: `<map>.map.png` (step 0) plus `<map>-dir1.map.png` … `<map>-dir23.map.png`, same dimensions, one mod/datadir overlay.
- Steps 1..23 select their own PNG; no rounding to the nearest few directions.
- Depth PNG: reuse the step-0 depth until per-direction depth exists. This is a documented limitation, not a silent substitution.
- Mask sets: add direction-tagged mask tiles generated from the same coverage triangles; runtime keeps one engine `MaskIndex` truth but selects per-direction bitmap/bbox for blits.
- Bake and mask generation run through the yaw-capable editor bake already on `fork/main`; a batch script drives all 24 yaw values.
- All state stays host-only: no `GameAction`, engine state, replay byte, wire message or hash changes.

## Goals / non-goals

Goals: step-accurate baked background for every heading; masks follow the active direction; unit tests cover selection and restore; gates green on minipc.

Non-goals: in-mission sprite re-baking for every direction; new retail art; changes to mission scripts, AI vision or campaign state.

## Remaining work

1. Batch bake script + two-direction-to-24-direction packaging.
2. Runtime selection table extended from 24-entry boolean to per-step texture slot; depth/mask lookup per step.
3. Direction mask rasterizer + host-side table.
4. Verification: Node bake tests, Rust unit tests, full push gate, and one live screenshot pair.
