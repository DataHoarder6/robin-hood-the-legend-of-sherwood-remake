# D12: teleport under late input (open)

D12 is the desync seen when `TeleportSelectedToPoint` is applied through
rollback after its target frame. It is distinct from issue #77's proof:
`H38` deliberately waits for zero frame-cursor skew (`cursors_aligned(max_skew=0)`)
before injecting gameplay commands, so no late input is forced and the campaign
handoff can be proved without resolving D12.

## Measured failure

Two native seats desync at frame 50 (teleport) or frame 175 (debriefing) when
the host-authored teleport is injected while the joining seat's cursor is ahead
of the host's. The client requests a full-snapshot resync and can then die on
`invalid multiplayer admission ordering`. Pre-fix evidence on minipc:
`~/h38-explore-20261123-015956`, `~/h38-teleport-only-20261123-021825`.

## What is anchored now

`37841147c` stamps the command's authoritative target frame into
`PcData.teleport_target_frame` and broadcasts the same stamp to every seat.
`execute_teleport` derives the fade countdown from that anchor rather than the
element's execution frame, and reserves star-titbit ids from the target frame
and actor. No field is excluded from the state hash.

## Remaining gaps

A regression test that applies the teleport at frame N on one engine and at
N+1 on another, then compares the full `EngineInner::state_hash` at N+8, still
diverges on authoritative state that is not derived from the anchor:

- `SequenceManager` allocates sequence ids from a monotonic counter advanced by
  each frame; a command applied a frame later lands its teleport element at a
  different id.
- `TitbitManager::update` counts down `UnconsciousStar.phase` once per tick, so
  stars created a frame apart reach different phases at the same absolute frame.

The test is preserved as `#[ignore]` in
`crates/robin_engine/src/engine/rollback_safe/tests.rs` so the seam is not lost.
Closing D12 requires target-frame anchoring of both the sequence-id allocator
and the titbit phase countdown, which is beyond issue #77's scope.

Tracking: [#83](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/83).
