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

## Fixed now

`37841147c` stamps the command's authoritative target frame into
`PcData.teleport_target_frame` and broadcasts the same stamp to every seat.
`execute_teleport` derives the fade countdown and reserves star-titbit ids
from that anchor. No field is excluded from the state hash.

The 2026-11-23 round-2 fix closes the two remaining gaps for any number of
selected PCs:

- **Sequence ids.** Commands run before the hourglass increments
  `frame_counter`, so the element's authoritative execution frame is
  `frame_counter + 1`. The forward tick reaches that frame with no fallback
  installed and launches the command's elements through the ordinary monotonic
  allocator. A seat admitting the command a frame late has already allocated
  one idle `Wait` per selected actor. `manage_input_process_teleport` calls
  `SequenceManager::rewind_latest_wait_slots`, which removes those newest
  sequences, rewinds the sequence and element counters to their oldest id and
  returns the fallbacks; the teleports launch through the same ordinary path;
  `rehome_sequences` then re-inserts the fallbacks at fresh ids. Both seats end
  with identical sequence ids, element ids, allocator counters and `IndexMap`
  order at the same absolute frame. Stamping ids from `target_frame` instead
  collides with whatever the allocator had reached by then — frame number and
  allocator position coincide only when one sequence is launched per frame,
  which is why round 1 passed the single-PC check but not two PCs.
- **Star phases.** The no-supplier `UnconsciousStar` countdown is advanced once
  per tick, so `execute_teleport` seeds it from
  `phase_elapsed = (frame_counter + 1) - target_frame` rather than
  `frame_counter - target_frame`. A forward tick seeds elapsed 0 and receives
  one more tick; a one-frame-late replay seeds elapsed 1 and receives one
  fewer. Both reach the same phase at the same absolute frame. The fade
  counter keeps its original `elapsed` seed; it is a hashed `PcData` field,
  seeded by `execute_teleport` and decremented by `tick_pc_teleport_fades`
  under the engine's `Entities` hourglass phase, so the full state-hash
  assertions cover fade convergence too.

No field is excluded from the state hash and neither check's assertion was
narrowed.

## Check

Two non-ignored tests in `crates/robin_engine/src/engine/rollback_safe/tests.rs`
(`engine` suite) apply the same host-authored teleport at its target frame 42
on one engine and a frame late on the other, then assert full
`EngineInner::state_hash` equality at absolute frame 50:

- `teleport_fade_state_matches_at_target_frame_despite_one_frame_apply_lag` —
  one selected PC. Red before round 1: `15345386212305945320` vs
  `14066127454569604982` (minipc:~/d12-red-test.log).
- `two_pc_teleport_matches_at_target_frame_despite_one_frame_apply_lag` — two
  selected PCs moved by one command. Red on round 1's tree `99832e64e`:
  `9397893188934422043` vs `12246651237733334055`
  (minipc:~/d12-a2-red.log); green after round 2: 2 passed, 0 failed
  (minipc:~/d12-a2-final-green.log). The full `engine` suite is also green
  on the fixed tree: 4501 passed, 0 failed, 76 ignored. Both focused runs
  used an isolated private copy on the build host, not the serialized gate.

## Residual risk

- The late rewind assumes each selected actor's newest sequence is its
  target-frame `Wait` and that only one frame of fallbacks exists.
  `rewind_latest_wait_slots` asserts the owner, command and counter shape and
  fails otherwise. A PC busy with a real order, and a lag beyond one frame,
  are TODOs in `manage_input_process_teleport`.
- The star-phase seed skips stars whose countdown would already have expired
  before this element executed (`remaining <= 1`). For the authored phases
  `[4, 12, 20, 12, 4]` and a one-frame lag used here both seats add and expire
  the same stars. A much longer apply lag with a short authored phase could
  still make one seat add a star the other skips.
- `H38` does not force late input and is not a D12 check. These are
  engine-level convergence proofs; they do not replace a process-layer replay
  with skewed cursors.

Tracking: [#83](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/83).
