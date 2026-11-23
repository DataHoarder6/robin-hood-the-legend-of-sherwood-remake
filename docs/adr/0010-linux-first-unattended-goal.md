# 0010 — Linux-first unattended goal

- **Status:** Accepted
- **Date:** 2026-11-22
- **Related:** [issue #49](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/49), [UNATTENDED.md](../UNATTENDED.md), [GAPS.md](../GAPS.md)

## Context

[Issue #49](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/49)
defines the standing goal with nine "What done means" criteria:

1. Both seats on the same campaign slot.
2. The whole mission sequence, transitioned by the game's debriefing.
3. Cross-play in all four host/guest OS directions.
4. Mid-campaign rejoin at the host's mission.
5. The terminal path, per mission, for both seats.
6. No desync — the canonical state hash agrees on both seats.
7. No unbounded wait.
8. Persistence across sessions.
9. Packaging: the Windows installer and the Linux package.

The unattended loop runs on Linux seats. Only a Linux↔Linux campaign is
automatable here: criteria 1, 2, 4, 5, 6, 7 and 8 can be driven and proved with
the build-host gate and two Linux seats. Criterion 3's Windows directions and
criterion 9's Windows installer need a real Windows machine, which the loop
cannot provision. Wine replay parity ([RELEASES.md](../RELEASES.md),
[TESTING.md](../TESTING.md)) proves only the toolchain/floating-point/locale
class, not the OS-layer class — Wine is not Windows. The owner will perform the
real-Windows check themselves and expects Windows to work if the build runs.

The recorded build also needs a human to play one full campaign end to end
([issue #56](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/56)):
the loop can prove the harness corpus and two-seat Linux replays, but it cannot
substitute for the owner's play of the shipped artifact. That step is a human
gate too — it is not a capability the loop blocks on.

## Decision

The automated loop targets criteria 1, 2, 4, 5, 6, 7 and 8 on Linux — a
Linux↔Linux full campaign played end to end on the recorded build. Criterion 3's
Windows directions and criterion 9's Windows installer are a `HUMAN_GATE`: a
human-owned check, not a capability the loop waits on. The loop records those
items as `HUMAN_GATE` (an annotation, not a fourth terminal state), as is the
human full-campaign play on the recorded build (#56), and continues with the
Linux campaign work. A missing Windows seat or human play is no longer a
`BLOCKED` capability that stops the campaign goal.

## Consequences

- The loop stops emitting the Windows `BLOCKED`; other missing capabilities
  (credentials, host configuration, CI/deploy authority) can still be `BLOCKED`.
- Issue #49 stays open until the owner's real-Windows check and full-campaign
  play are done; the loop does not close it on a Linux-only pass.
- No assertion is weakened: the Windows directions and the installer remain part
  of the standing goal, and Windows code paths stay gated by the same build and
  test suites as the Linux paths.
- An iteration still ends in one of the three terminal states — LANDED or FILED
  for the Linux work it completes — with `HUMAN_GATE` noting what was deferred.
