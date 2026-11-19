# Ravensholt — The Blazon at the Ford

Standalone skirmish map, not part of the campaign. Blazon objective: clear the
garrison, take the blazon from the hall.

Map units: 1792 × 1280 (28 × 20 cells at 64 px). Same size as `Croisement02`
plus two cell rows for the upper floor.

## Premise

A fortified manor holds the ford on the Ravensholt road. The garrison has
seized a blazon and hangs it in the hall. Robin's band enters from the south
bank at dusk.

## Shape of the place

- **River**, west to east across the middle third, flowing east. One stone
  bridge, one ford.
- **South bank**: village street, cottages, a mill on the millrace, gardens,
  wooded margin west.
- **North bank**: curtain wall around the manor, round towers at the corners,
  gatehouse on the south wall facing the bridge, keep in the north-west, hall
  in the north-east with the blazon, chapel between them.
- **Two ways in**: the bridge to the gatehouse, or the ford to the east postern.
  The postern is the quieter route and the longer one.
- **Upper floor**: gatehouse first floor and the wall walk, reached by the
  gatehouse stair and by the chapel stair. Guards patrol the wall walk.

## Features the map must exercise

| Feature | Where |
| --- | --- |
| Elevation | wall walk, gatehouse floor, chapel stair, riverbank |
| Interior | hall (blazon), chapel, gatehouse lower room |
| Water | river, bridge, ford, millrace |
| Two routes | bridge → gate; ford → postern |
| Patrols | wall walk, courtyard, village street |
| Blazon | hall, north-east |

## Files

- `terrain-layout.svg` — the schematic the terrain is authored from: water,
  banks, roads, wall line, building footprints, upper-floor outline.
- `layout.json` — placements, added when the scene exists.
- The scene itself lives in `level-editor/library/scenes/Ravensholt.rhlos-map.json`.

The schematic is a drawing, not the background. The editor renders the
background from placed assets; this file only fixes the layout.

The scene is the editor's own "New map" output, unmodified: size 1792×1280,
grid spacing 128, initial elevation 0, material `grass_short`, no placements.
It is what `createNewMap` (`level-editor/app/src/new-map.ts`) writes, so it can
be regenerated rather than hand-edited.

Authoring the terrain and placements needs an asset library with
`3d-assets/index.json` and `game-data/index.json`. Neither is in this
repository; point the editor at one with `EDITOR_LIBRARY`.
