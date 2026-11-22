# Known capability gaps

One-page index of what the port cannot yet do. This is a pointer list, not a
second copy of the detail: follow the links for scope, evidence, and owner.
Current bug status is the
[issue list](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues);
per-defect evidence lives in [the defect ledger](DEFECTS.md).

[Issue #49](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/49)
— a bug-free, end-to-end co-op campaign meeting its nine criteria — is the
standing goal; the gaps below are what remains of it.

Last verified: 2026-11-21 against `fork/main`.

Closed 2026-11-21: [issue #45](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/45)
and [issue #47](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/47).

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
- Linux↔Linux full campaign co-op is the automated target
  ([issue #77](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/77));
  Windows cross-play is a human gate
  ([ADR 0010](adr/0010-linux-first-unattended-goal.md)).
  [Issue #49](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/49)
  remains the standing goal.
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
