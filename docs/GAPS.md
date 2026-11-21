# Known capability gaps

One-page index of what the port cannot yet do. This is a pointer list, not a
second copy of the detail: follow the links for scope, evidence, and owner.
Current bug status is printed by `gh issue list -R DataHoarder6/robin-hood-the-legend-of-sherwood-remake`
and recorded per defect in [the defect ledger](DEFECTS.md).

Last verified: 2026-11-21 against `fork/main`.

## Release blockers

- Mission completion exits the game instead of returning to the menu —
  [issue #47](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/47).
- A replay is not consumed when the mission opens a modal —
  [issue #45](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/45).

## Editor and map pipeline

- The reconstruction-only game baker cannot compile authored maps, imported
  assets, or spline geometry —
  [NEW_FEATURES.md](NEW_FEATURES.md) (Done, "Focused level-editor workspace").
- Extracted-map parity is unfinished across masks, visual/depth state, and
  full-map publication —
  [NEW_FEATURES.md](NEW_FEATURES.md) (Done, "Compiled map mask interchange").
- Authored scenery previews and published effect definitions are unfinished —
  [NEW_FEATURES.md](NEW_FEATURES.md) (Done, "Asset-owned scenery animation definitions").

## Multiplayer and reporting

- Matchmaking announcements are not signed with the game identity key —
  [NEW_FEATURES.md, Todo](NEW_FEATURES.md#todo).
- Whole-campaign co-op with Windows/Linux cross-play —
  [issue #49](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/49).
- Bug reports do not yet carry browser replay attachments or a native form —
  [NEW_FEATURES.md, crash and bug reporting](NEW_FEATURES.md#crash-and-bug-reporting).
- Bug reports do not name the UI element under the cursor —
  [issue #50](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/50).
- No test proves a bug report files a GitHub issue —
  [issue #51](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/51).

## Difficulty and performance

- Legendary difficulty hardening is undecided —
  [NEW_FEATURES.md](NEW_FEATURES.md) (Done, "Legendary and custom difficulty").
- The VQ context-model codec is not wired into the shipping schema —
  [NEW_FEATURES.md](NEW_FEATURES.md) (Done, "VQ sprite context-model codec").

## Code quality

- Runtime fields still use legacy sentinels where semantics are unproven —
  [NEW_FEATURES.md, Code Quality](NEW_FEATURES.md#code-quality).
