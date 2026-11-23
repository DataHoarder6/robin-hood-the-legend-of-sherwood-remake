# Design: Robin Hood remake v0.5.0 scope

Status: proposed by the controller, pending owner approval (brainstorming gate).
Owner's instruction 2026-11-23: "do all this" — fix browser deploy, rebase
map-rotation, define 0.5.0 scope and ETA. This document is the scope decision.

## Intent

Ship a 0.5.0 that is credible: a web build that actually loads and plays, plus
the rotating map — the only feature already declared for 0.5.0 in
`docs/BACKLOG.md`. Cut the large, human-gated campaigns rather than pretending
they fit a date.

## Approaches considered

1. **Browser-only 0.5.0** — tag the web build that works; rotating map to 0.5.1.
   Smallest, but drops the only declared 0.5.0 feature and breaks the BACKLOG
   contract. Rejected unless the rebase fails.
2. **Browser + rotating map (chosen)** — the declared 0.5.0 feature plus the
   fixes required for the web build to load and play. The map is tracked in
   BACKLOG, so it has a home; the browser fixes are the install base for it.
3. **Browser + map + co-op/Android** — rejected: #49/#55/#56 and #57/#58 are
   standing goals with human-seat gates, not releasable features on a date.

## Goals

- Web deployment works end to end: #84, #85, #87, #88, #89.
- Browser is playable: campaign progress survives reload (#76), canvas fits
  the viewport (#78), quick save works (#81), mouse clicks resolve dialogs
  (#67).
- Native first-run does not crash: #86.
- Rotating map lands on `fork/main` with docs and a release note.
- #51 is *recorded* (pass or "harness unavailable") per RELEASES step 6.

## Non-goals / anti-goals

- No co-op endgame (#49/#55/#56), no Android (#57/#58).
- No 0.5.1 niceties: #79, #80, #70–#73, #63, #62, #65, #66, #68, #69, #61.
- #75/#83 stay open unless the release corpus forces them in.
- No claim of a Windows cross-play proof; Wine parity only, per RELEASES step 3.

## Constraints

- `fork/main` is the integration ref; `land` refuses this repo (ADR 0004);
  the repo's own `scripts/land.sh` lands through a 30–60 min gate under `runbg`.
- RELEASES steps 1–9 are authoritative for a stable tag; step 5 is human-gated.
- One implementer at a time within the estate (dispatch baseline clause 4).

## Decisions

- 0.5.0 = browser deploy + browser playability musts + #86 + rotating map.
- Browser deploy fixes are a prerequisite, not the theme, and land first.
- Map rebase is second (engineering long pole, 28 ahead/85 behind, one
  `viewport.rs` conflict of the "keep both" kind).
- ETA is a dependency chain, not a date; the Windows seat (#55 phase 7,
  `player-b-pc` off with no WoL) is the unbounded human pole and is excluded
  from this release.

## Left to the builder

- Implement browser fixes in a worktree off `fork/main`; run the repo gate.
- Rebase `agent/map-rotation`, resolve the conflict, update the stale conflict
  doc, run `remote-check.sh --suites format lint check client-release wasm docs`.
- Open a 0.5.0 milestone and a rotating-map issue (tracking, not code).
- Record #51's status before any tag.
