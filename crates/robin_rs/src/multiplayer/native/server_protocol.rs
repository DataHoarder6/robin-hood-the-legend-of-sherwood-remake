//! Pure, bounded server protocol state. Transport authority remains in the
//! parent dispatcher: deserializing these values cannot create an active stream.

use super::{MultiplayerError, PlayerId};
use robin_engine::multiplayer::{SnapshotTransitionId, SnapshotTransitionPayload};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Base budget for a host-authored transition commit.
pub(super) const TRANSITION_COMMIT_MS: u64 = 60_000;
/// Payload transfer allowance added on top of the base commit budget.
pub(super) const TRANSITION_PAYLOAD_BYTES_PER_SECOND: u64 = 64 * 1024;

fn payload_bytes(payload: &SnapshotTransitionPayload) -> u64 {
    match payload {
        SnapshotTransitionPayload::Save { save_bytes, .. } => save_bytes.len() as u64,
        SnapshotTransitionPayload::CampaignExit { engine_bytes, .. } => engine_bytes.len() as u64,
    }
}

/// Total time a transition may wait before absent peers are evicted.
///
/// The base budget covers round trips and validation; the payload term covers
/// the bytes a peer must receive, validate and retain before it can ack.
pub(super) fn transition_budget_ms(payload: &SnapshotTransitionPayload) -> u64 {
    TRANSITION_COMMIT_MS
        + payload_bytes(payload).saturating_mul(1000) / TRANSITION_PAYLOAD_BYTES_PER_SECOND
}

/// How long the host waits for peers to finish loading before it starts the
/// mission without the ones that have not.
pub(super) const READY_BUDGET_MS: u64 = 120_000;

#[derive(Debug, Eq, PartialEq)]
pub(super) enum ReadyDecision {
    /// Keep waiting.
    Wait,
    /// Every participant that counts is ready; begin at this frame.
    Begin(u32),
    /// The budget passed with peers still loading; the caller evicts them and
    /// releases with [`ReadyBarrier::forced_frame`].
    Expired,
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct ReadyBarrier {
    pub(super) host_frame: Option<u32>,
    pub(super) begun: Option<(u32, u64)>,
    /// Set when the host first observes a blocked barrier, so the wait is
    /// bounded even when no Ready frame ever arrives to re-evaluate it.
    pub(super) wait_since_ms: Option<u64>,
    /// Admitted seats at the last evaluation, to detect a peer joining mid-wait.
    pub(super) observed_connected: u32,
    /// A late joiner refreshes the wait once; after that the budget is final.
    pub(super) extension_used: bool,
}

impl ReadyBarrier {
    pub(super) fn reset(&mut self) {
        self.host_frame = None;
        self.begun = None;
        self.wait_since_ms = None;
        self.observed_connected = 0;
        self.extension_used = false;
    }

    /// Facts include all authenticated seats as `(admitted, attached, ready)`.
    /// Only admitted seats count toward quorum, but every ready frame
    /// contributes to the release frame, matching the existing
    /// reconnect/provisional-seat policy.
    pub(super) fn candidate(
        &mut self,
        now_ms: u64,
        expected_players: u32,
        facts: impl IntoIterator<Item = (bool, bool, Option<u32>)>,
    ) -> ReadyDecision {
        if self.begun.is_some() {
            return ReadyDecision::Wait;
        }
        let Some(host_frame) = self.host_frame else {
            return ReadyDecision::Wait;
        };
        let mut frame = host_frame;
        let mut connected = 0;
        let mut blocked = false;
        for (in_sim, attached, ready) in facts {
            if in_sim {
                connected += 1;
                blocked |= !attached || ready.is_none();
            }
            if let Some(ready) = ready {
                frame = frame.max(ready);
            }
        }
        if connected >= expected_players.saturating_sub(1) && !blocked {
            return ReadyDecision::Begin(frame);
        }
        match self.wait_since_ms {
            None => self.wait_since_ms = Some(now_ms),
            Some(_) if connected > self.observed_connected && !self.extension_used => {
                // A peer admitted after the wait began gets its own budget once.
                self.wait_since_ms = Some(now_ms);
                self.extension_used = true;
            }
            Some(_) => {}
        }
        self.observed_connected = connected;
        let wait_since_ms = self.wait_since_ms.unwrap_or(now_ms);
        if now_ms.saturating_sub(wait_since_ms) >= READY_BUDGET_MS {
            ReadyDecision::Expired
        } else {
            ReadyDecision::Wait
        }
    }

    /// Release frame once the wait expires: the host frame plus every ready
    /// frame that did arrive. Quorum is deliberately ignored here; the caller
    /// evicts the peers that were blocking the barrier.
    pub(super) fn forced_frame(
        &self,
        facts: impl IntoIterator<Item = (bool, bool, Option<u32>)>,
    ) -> Option<u32> {
        let mut frame = self.host_frame?;
        for (_, _, ready) in facts {
            if let Some(ready) = ready {
                frame = frame.max(ready);
            }
        }
        Some(frame)
    }

    pub(super) fn commit(&mut self, frame: u32, epoch_ms: u64) {
        assert!(self.begun.is_none(), "ready barrier released twice");
        self.begun = Some((frame, epoch_ms));
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct PendingSnapshotTransition {
    pub(super) id: SnapshotTransitionId,
    pub(super) payload: SnapshotTransitionPayload,
    pub(super) awaiting: HashSet<u8>,
    /// Seats whose acknowledgement was processed, or that were evicted when
    /// the barrier expired. A later ack from any of them is a no-op.
    pub(super) answered: HashSet<u8>,
    /// Monotonic milliseconds since server start at which absent peers are
    /// evicted. Set when the transition begins.
    pub(super) deadline_ms: u64,
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct SnapshotTransitions {
    pending: Option<PendingSnapshotTransition>,
}

impl SnapshotTransitions {
    pub(super) fn pending(&self) -> Option<&PendingSnapshotTransition> {
        self.pending.as_ref()
    }

    pub(super) fn begin(&mut self, transition: PendingSnapshotTransition) {
        assert!(
            self.pending.is_none(),
            "another snapshot transition is pending"
        );
        self.pending = Some(transition);
    }

    pub(super) fn acknowledge(
        &mut self,
        seat: PlayerId,
        id: SnapshotTransitionId,
    ) -> Result<(), MultiplayerError> {
        let remote = |message: String| MultiplayerError::RemoteProtocol(message.into());
        let transition = self.pending.as_mut().ok_or_else(|| {
            remote(format!(
                "peer {seat:?} acknowledged no active snapshot transition"
            ))
        })?;
        if transition.id != id {
            return Err(remote(format!(
                "peer {seat:?} acknowledged snapshot transition {id:?}, active is {:?}",
                transition.id
            )));
        }
        if transition.awaiting.remove(&seat.0) {
            transition.answered.insert(seat.0);
            return Ok(());
        }
        if transition.answered.contains(&seat.0) {
            // A duplicate ack, or an ack that raced an eviction, must not turn
            // a recoverable session into a protocol failure.
            return Ok(());
        }
        Err(remote(format!(
            "peer {seat:?} was not expected for snapshot transition {id:?}"
        )))
    }

    /// Evict every seat still waiting once the deadline passes.
    ///
    /// Returns the transition id and the evicted seats, sorted, so the caller
    /// can log and commit. Eviction is what makes a missed barrier bounded:
    /// the connected peers proceed, and an evicted seat re-admits later from a
    /// snapshot instead of freezing the session.
    pub(super) fn expire(&mut self, now_ms: u64) -> Option<(SnapshotTransitionId, Vec<u8>)> {
        let transition = self.pending.as_mut()?;
        if now_ms < transition.deadline_ms || transition.awaiting.is_empty() {
            return None;
        }
        let mut evicted: Vec<u8> = transition.awaiting.drain().collect();
        evicted.sort_unstable();
        transition.answered.extend(evicted.iter().copied());
        Some((transition.id, evicted))
    }

    pub(super) fn retain_for_reconnect(&mut self, seat: u8) {
        if let Some(transition) = self.pending.as_mut() {
            // Connectivity never shrinks the barrier, including after an ack.
            transition.awaiting.insert(seat);
        }
    }

    pub(super) fn take_completed(&mut self) -> Option<SnapshotTransitionId> {
        if !self.pending.as_ref()?.awaiting.is_empty() {
            return None;
        }
        Some(self.pending.take().expect("checked pending transition").id)
    }
}
