# Map rotation — visual polish design (2026-11-22)

## Intent

Make the existing host-only camera rotation look deliberate rather than like a
tilted screenshot, and fix the gaps that show up when it is used outside the
one named test level.

## Goals

- Rotation works correctly in split-screen on every map.
- The baked bitmap never has to be enlarged past the crisp zoom range just to
  cover a rotated corner.
- Rotating feels like a camera gesture, with visible feedback.
- No change to simulation, replay input, state hashes or angle-0 rendering.

## Non-goals

- No full 360° orbit and no re-baked or new map art. See Alternatives.
- No change to the default angle range beyond what map size already permits.
- No build, gate or landing while 0.3.0 is in flight.

## Constraints

- Every map is one baked isometric bitmap with fixed lighting and baked
  shadows; it has no true 3D orbit. The larger the angle, the more wrong the
  art looks — this is the hard ceiling on the whole feature.
- Level size comes from the background pixel dimensions in mission setup
  (`game_session/setup.rs`); a map with no background gets no rotation
  coverage and must be reported, not defaulted.
- One shared transform (`host/camera_transform.rs`) owns every world→screen
  mapping. New code must call it, not repeat trigonometry.
- Deferred feature: no gates, no rebase, no landing until told.

## Alternatives considered

1. **Polish the subtle rotation on shipped maps (chosen).** Five bounded
   changes, no new art, no asset pipeline. Works within the baked-art ceiling
   and can be gated later.
2. **Full 360° on the original maps.** Rejected: the art is upside down at
   180°, with inverted shadows and signs. Small angles are the only honest
   range for baked bitmaps.
3. **Re-bake maps through the level-editor 3D bake pipeline.** The only route
   to a real orbit, but retail maps cannot be re-baked without their original
   sources. Separate project, out of scope here.

## Decided work

1. **Split-screen edge bars.** `game_session/render.rs` masks out-of-map
   areas using unrotated view bounds. Derive the mask from the same rotated
   bounds `clip_view` uses, or skip the bars when rotation coverage already
   guarantees no exposed area. Builder decides which after reading the code.
2. **Cap angle by map size instead of widening zoom.** On small maps
   `minimum_zoom` currently raises zoom past 2.0 to cover the corners, which
   magnifies baked pixels. Derive the largest angle whose coverage fits at
   zoom ≤ 2.0 and refuse or clamp the step beyond it; keep angle 0 untouched.
3. **Depth parallax (spike first).** The renderer already receives baked
   occlusion depth (`upload_occlusion_depth`). Prototype a few pixels of
   foreground/ground shift while rotating. Keep it only if it reads as depth
   and does not detach sprites from terrain.
4. **Angle feedback.** A compact readout or compass that appears on rotation
   and fades afterwards. Use an existing debug-text mechanism if one exists;
   otherwise omit it rather than inventing a HUD surface.
5. **Right-drag rotation.** Mission mode already has a right-drag camera
   gesture. Bind horizontal right-drag (or a modifier on it) to rotation and
   keep `[`/`]` for precise steps. Check the gesture handler before choosing
   the binding.

## Left to the builder

- Exact bar-mask derivation and whether it can be deleted under rotation.
- Analytic or binary-search method for the map-size angle cap.
- Whether the depth data is sufficient for step 3; drop it if not.
- Which existing text surface, if any, carries the angle readout.
- Tests for each change, written failing first; names and files chosen during
  planning. No test may be claimed passing without a gate run.
