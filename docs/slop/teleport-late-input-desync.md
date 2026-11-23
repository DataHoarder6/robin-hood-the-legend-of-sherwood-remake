# D12: teleport under late input (fixed 2026-11-23)

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

## Anchored now

`37841147c` stamps the command's authoritative target frame into
`PcData.teleport_target_frame` and broadcasts the same stamp to every seat.
`execute_teleport` derives the fade countdown and reserve star-titbit ids from
that anchor. The 2026-11-23 fix closed the two remaining gaps:

- **Sequence ids.** `manage_input_process_teleport` launches the teleport
  element through `launch_target_frame_anchored_element`, which calls
  `SequenceManager::insert_target_frame_anchored_sequence` with the target
  frame as the sequence id. On the lagging seat the idle fallback that already
  occupies that slot is moved to the next allocator slot (keeping its own
  element ids re-stamped), so the two seats end with the same sequence set,
  the same `next_sequence_id` / `next_element_id` and the same `IndexMap`
  insertion order. Without moving it the seat lost one sequence and one
  allocator tick.
- **Star phases.** The no-supplier `UnconsciousStar` countdown is advanced once
  per tick, so `execute_teleport` now seeds it from
  `phase_elapsed = (frame_counter + 1) - target_frame` rather than
  `frame_counter - target_frame`. `frame_counter` is the frame the command was
  admitted into; the hourglass increments it after commands run, so the
  element's authoritative execution frame is `frame_counter + 1`. A forward
  tick seeds elapsed 0 and receives one more tick; a one-frame-late replay
  seeds elapsed 1 and receives one fewer. Both reach the same phase at the
  same absolute frame. The fade counter keeps its original `elapsed` seed; it
  is driven by the renderer's pre-frame tick (`tick_pc_teleport_fades`), not by
  the engine hourglass, and is not exercised by this engine-level check.

No field is excluded from the state hash and the check's assertion was not
narrowed.

## Check

`teleport_fade_state_matches_at_target_frame_despite_one_frame_apply_lag`
(`crates/robin_engine/src/engine/rollback_safe/tests.rs`, `engine` suite) applies
the same teleport at its target frame 42 on one engine and a frame late on the
other, then asserts full `EngineInner::state_hash` equality at absolute frame
50. Its `#[ignore]` was removed as the red step. Red before the fix:
`15345386212305945320` vs `14066127454569604982`
(minipc:~/d12-red-test.log); the JSON diff showed six star phases one tick
apart and the teleport and idle-fallback sequences swapped between ids 42 and
43. Green after: 1 passed, 0 failed (minipc:~/d12-final-green.log).

## Residual risk

- Anchoring the sequence id to `target_frame` uses the allocator's one-sequence-
  per-frame position as the id. A frame that legitimately allocates several
  sequences before the target frame could make that id collide with an earlier
  sequence; `insert_target_frame_anchored_sequence` then moves the wrong entry.
  The teleport star id uses a scaled `target_frame * 64` range and does not have
  this exposure.
- The star-phase seed skips stars whose countdown would already have expired
  before this element executed (`remaining <= 1`). For the authored phases
  `[4, 12, 20, 12, 4]` and a one-frame lag used here both seats add and expire
  the same stars. A much longer apply lag with a short authored phase could
  still make one seat add a star the other skips.
- `H38` does not force late input and is not a D12 check. This is an
  engine-level convergence proof; it does not replace a process-layer replay
  with skewed cursors.

Tracking: [#83](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/83).
