//! Bounded host-side barrier for cooperative-pause decisions.
//!
//! When the host commits a pause at an exact simulation frame it waits for
//! every seat in the session to acknowledge that boundary (their authoritative
//! state hash at `decision.frame`). A seat that never acknowledges — a frozen
//! or departed peer — must not hold the whole session paused forever. The
//! barrier stamps a monotonic deadline from [`super::COOP_PAUSE_BARRIER_MS`];
//! once it passes, the host evicts the missing seats and auto-resumes the
//! session with a fresh authoritative decision. A late acknowledgement from an
//! evicted seat is a no-op. This mirrors the modal acknowledgement budget and
//! the snapshot-transition deadline without being persisted game state: a
//! rollback or snapshot restore resets it, and the durable pause itself lives
//! in the engine's hashed simulation gate.

use super::*;

/// What a barrier tick asks the host to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PauseBarrierOutcome {
    /// No pause is pending, or every seat acknowledged — the pause holds.
    Idle,
    /// Still awaiting acknowledgements; `remaining_ms` of budget is left.
    Waiting { remaining_ms: u64 },
    /// Deadline passed: `evicted` seats never acknowledged and must be dropped.
    /// `resume` is the deterministic decision that frees the remaining seats.
    TimedOut {
        evicted: Vec<PlayerId>,
        resume: CoopPauseDecision,
    },
}

#[derive(Debug, Clone)]
struct PendingPause {
    decision: CoopPauseDecision,
    /// Seats whose acknowledgement the pause awaits (host bit always set).
    required: u32,
    /// Seats that acknowledged, including the authoring host.
    acknowledged: u32,
    started_ms: u64,
}

/// Host-only acknowledgement tracker. Not serialized, not hashed, not on the
/// wire — a peer never runs a pause barrier; it only adopts host decisions.
#[derive(Debug, Clone, Default)]
pub struct CoopPauseBarrier {
    pending: Option<PendingPause>,
}

impl CoopPauseBarrier {
    pub fn new() -> Self {
        Self { pending: None }
    }

    /// Begin awaiting acknowledgements for `decision` from `required` seats at
    /// wall-clock `now_ms`. The host is always counted as self-acknowledged.
    pub fn begin(&mut self, decision: CoopPauseDecision, required: u32, now_ms: u64) {
        self.pending = Some(PendingPause {
            decision,
            required: required | super::HOST_SEAT_BIT,
            acknowledged: super::HOST_SEAT_BIT,
            started_ms: now_ms,
        });
    }

    /// A seat acknowledged reaching `frame` for `decision_id`. Only the exact
    /// pending decision is accepted: a stale nonce, or a frame off the boundary,
    /// cannot release the barrier.
    pub fn acknowledge(&mut self, seat: PlayerId, decision_id: u64, frame: u32) {
        if seat == PlayerId::HOST {
            return;
        }
        if let Some(pending) = &mut self.pending
            && pending.decision.decision_id == decision_id
            && pending.decision.frame == frame
        {
            pending.acknowledged |= 1u32 << seat.0;
        }
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Advance the deadline at wall-clock `now_ms`. `next_frame` is the frame
    /// at which an auto-resume takes effect.
    pub fn tick(&mut self, now_ms: u64, next_frame: u32) -> PauseBarrierOutcome {
        let Some(pending) = self.pending.as_ref() else {
            return PauseBarrierOutcome::Idle;
        };
        if pending.acknowledged & pending.required == pending.required {
            self.pending = None;
            return PauseBarrierOutcome::Idle;
        }
        let elapsed = now_ms.saturating_sub(pending.started_ms);
        if elapsed < super::COOP_PAUSE_BARRIER_MS {
            return PauseBarrierOutcome::Waiting {
                remaining_ms: super::COOP_PAUSE_BARRIER_MS - elapsed,
            };
        }
        let missing = pending.required & !pending.acknowledged;
        let evicted: Vec<PlayerId> = (0..crate::coop::MAX_PLAYERS as u8)
            .filter(|seat| seat_in_mask(missing, PlayerId(*seat)))
            .map(PlayerId)
            .collect();
        // Auto-resume is itself a fresh, host-authored decision, so every seat
        // frees at the same exact frame and replays reproduce the release.
        let resume = CoopPauseDecision {
            decision_id: pending.decision.decision_id + 1,
            frame: next_frame,
            paused: false,
        };
        self.pending = None;
        PauseBarrierOutcome::TimedOut { evicted, resume }
    }

    /// Forget any pending barrier (session dropped, snapshot restored or the
    /// decision completed). Used so a late ack cannot acknowledge a new pause.
    pub fn clear(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision(id: u64, frame: u32) -> CoopPauseDecision {
        CoopPauseDecision {
            decision_id: id,
            frame,
            paused: true,
        }
    }

    #[test]
    fn all_seats_acknowledging_completes_the_barrier() {
        let mut barrier = CoopPauseBarrier::new();
        barrier.begin(decision(1, 40), 0b0111, 1_000);
        assert!(barrier.is_pending());
        // A stale frame or decision id cannot acknowledge the boundary.
        barrier.acknowledge(PlayerId(1), 1, 39);
        barrier.acknowledge(PlayerId(1), 2, 40);
        assert!(barrier.is_pending());
        barrier.acknowledge(PlayerId(1), 1, 40);
        barrier.acknowledge(PlayerId(2), 1, 40);
        assert_eq!(barrier.tick(1_000, 41), PauseBarrierOutcome::Idle);
        assert!(!barrier.is_pending());
    }

    #[test]
    fn deadline_evicts_missing_seats_and_auto_resumes() {
        let mut barrier = CoopPauseBarrier::new();
        barrier.begin(decision(7, 90), 0b0111, 0);
        // Seat 1 acknowledges; seat 2 never does.
        barrier.acknowledge(PlayerId(1), 7, 90);
        assert!(matches!(
            barrier.tick(COOP_PAUSE_BARRIER_MS - 1, 91),
            PauseBarrierOutcome::Waiting { remaining_ms: 1 }
        ));
        match barrier.tick(COOP_PAUSE_BARRIER_MS, 91) {
            PauseBarrierOutcome::TimedOut { evicted, resume } => {
                assert_eq!(evicted, vec![PlayerId(2)]);
                assert_eq!(
                    resume,
                    CoopPauseDecision {
                        decision_id: 8,
                        frame: 91,
                        paused: false,
                    }
                );
            }
            other => panic!("expected timeout, got {other:?}"),
        }
        assert!(!barrier.is_pending());
    }

    #[test]
    fn late_acknowledgement_after_eviction_is_a_no_op() {
        let mut barrier = CoopPauseBarrier::new();
        barrier.begin(decision(3, 12), 0b0011, 0);
        let _ = barrier.tick(COOP_PAUSE_BARRIER_MS, 13);
        barrier.acknowledge(PlayerId(1), 3, 12);
        assert!(!barrier.is_pending());
        assert_eq!(
            barrier.tick(COOP_PAUSE_BARRIER_MS + 1, 14),
            PauseBarrierOutcome::Idle
        );
    }
}
