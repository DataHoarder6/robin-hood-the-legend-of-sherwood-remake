# Documentation index

## Maintained development guides

- [Project quick start and platform overview](../README.md)
- [Testing and feature matrix](TESTING.md)
- [Playing: stable and dev builds](PLAY.md) — the channel each machine runs, delivery, and the version check before a shared session
- [Coordinates](COORDINATES.md), [rendering](RENDERING.md), and [gameplay profiling](GAMEPLAY_PROFILING.md)
- [Game data distributions](DATADIRS.md)
- [Multiplayer](MULTIPLAYER.md), [campaign history](CAMPAIGN_HISTORY.md), and [Spellforge](SPELLFORGE.md)
- [Multiplayer harness decisions](multiplayer/HARNESS.md) — coverage matrix, scenario format, layer rules, isolation and bootstrap for the e2e gate
- [Presentation decisions](PRESENTATION.md) — the zoom ladder and what a viewport larger than the level shows
- [New features and remaining product work](NEW_FEATURES.md)
- [Argument bundling candidates](SIGNATURE_BUNDLING_CANDIDATES.md)
- [Nix package and cross-host install](NIX.md)
- [Working in parallel: symptoms, rules, commands](PARALLEL_AGENTS.md) — shared-checkout rules, stale-main recovery, build copies, disk levers, and `just doctor`
- [Android build and packaging](../android/README.md)

## Service and deployment guides

- [Highscores, replay verification, ranked authority authoring, VPS and browser deployment](../crates/robin_highscores/README.md) is the consolidated service and deployment guide; the operations scripts live in `crates/robin_highscores/ops/`.
- [Crash reporting](CRASHES.md) — what a panic leaves on disk, the upload endpoint contract, and the receiver that files one GitHub issue per crash
- [Bug reports](BUG_REPORTS.md) — the voluntary in-game report: its three entrances, the bundle it carries, and its endpoint contract

These documents describe operations; reading or testing a deployment command
does not by itself authorize publishing a release or modifying a live service.

## Historical assessments and design evidence

- [Plans](plans/) contains design and migration history; check each document's status before treating its future-tense steps as outstanding work.
- [Feature reviews](feature-reviews/) and [interviews](interviews/) contain supporting assessments and source material.

Keep original-game ordering, format, and resource-behavior evidence beside the
implementation it explains. Historical evidence is retained for that purpose;
current build commands and suite selection belong in the maintained guides.
