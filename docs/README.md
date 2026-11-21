# Documentation index

## Maintained development guides

- [Project quick start and platform overview](../README.md)
- [Testing and feature matrix](TESTING.md)
- [Playing: stable and dev builds](PLAY.md) — the channel each machine runs, delivery, and the version check before a shared session
- [Coordinates](COORDINATES.md), [rendering](slop/RENDERING.md), and [gameplay profiling](slop/GAMEPLAY_PROFILING.md)
- [Game data distributions](DATADIRS.md)
- [Multiplayer](MULTIPLAYER.md), [campaign history](slop/CAMPAIGN_HISTORY.md), and [Spellforge](slop/SPELLFORGE.md)
- [Multiplayer harness decisions](multiplayer/HARNESS.md) — coverage matrix, scenario format, layer rules, isolation and bootstrap for the e2e gate
- [Presentation decisions](PRESENTATION.md) — the zoom ladder and what a viewport larger than the level shows
- [Defect ledger](DEFECTS.md) — one row per diagnosed defect: class, reproduction, the check that proves it, status
- [New features and remaining product work](NEW_FEATURES.md)
- [Known capability gaps](GAPS.md) — the one-page index of what is unfinished and where the detail lives
- [Unattended orchestration](UNATTENDED.md) — terminal states, fixed iteration order, gate/land path, retry/quarantine, and the Windows-as-BLOCKED boundary
- [Nix package and cross-host install](NIX.md)
- [Working in parallel: symptoms, rules, commands](PARALLEL_AGENTS.md) — shared-checkout rules, stale-main recovery, build copies, disk levers, and `just doctor`
- [0.3.0 release evidence](releases/0.3.0.md) — candidate, gate counts, the H23 evidence path, and the still-human two-seat check
- [Authored map compiler design](slop/AUTHORED_MAP_COMPILER.md) — offline bake pattern, the no-runtime-solver ruling, and the first slice
- [Android build and packaging](../android/README.md)

## Service and deployment guides

- [Highscores, replay verification, ranked authority authoring, VPS and browser deployment](../crates/robin_highscores/README.md) is the consolidated service and deployment guide; the operations scripts live in `crates/robin_highscores/ops/`.
- [Crash reporting](CRASHES.md) — what a panic leaves on disk, the upload endpoint contract, and the receiver that files one GitHub issue per crash
- [Bug reports](BUG_REPORTS.md) — the voluntary in-game report: its three entrances, the bundle it carries, and its endpoint contract

These documents describe operations; reading or testing a deployment command
does not by itself authorize publishing a release or modifying a live service.

## Historical assessments and design evidence

- [Interviews](interviews/) contain supporting source material.

Keep original-game ordering, format, and resource-behavior evidence beside the
implementation it explains. Historical evidence is retained for that purpose;
current build commands and suite selection belong in the maintained guides.
