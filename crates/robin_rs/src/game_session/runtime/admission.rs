//! Ordered multiplayer admission, independent of timeline and history owners.
use crate::game_session::multiplayer::{
    MultiplayerAdmissionEvent, MultiplayerSessionError, SessionProtocolFailure,
};
use serde::{Deserialize, Serialize};

/// The transport owns handshakes and wire delivery; this state machine owns
/// the point at which a loaded mission may begin advancing simulation. Keeping
/// it in `TimelineRuntime` also keeps snapshot adoption ahead of replay and
/// rollback frame capture for both graphical and true-headless drivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum MultiplayerAdmission {
    NotRequired,
    HostWaitingForBegin,
    HostWaitingForResyncBegin { snapshot_frame: u32 },
    PeerWaitingForSnapshot,
    PeerWaitingForBegin { snapshot_frame: u32 },
    WaitingForStart { frame: u32, start_epoch_ms: u64 },
    Running,
}

impl MultiplayerAdmission {
    /// Stable lowercase phase name for `/state`; independent of `Debug`.
    pub(crate) fn wire_name(self) -> &'static str {
        match self {
            Self::NotRequired => "not_required",
            Self::HostWaitingForBegin => "host_waiting_for_begin",
            Self::HostWaitingForResyncBegin { .. } => "host_waiting_for_resync_begin",
            Self::PeerWaitingForSnapshot => "peer_waiting_for_snapshot",
            Self::PeerWaitingForBegin { .. } => "peer_waiting_for_begin",
            Self::WaitingForStart { .. } => "waiting_for_start",
            Self::Running => "running",
        }
    }

    /// Stable wording for logs and fatal diagnostics; `Debug` also carries the
    /// frame that the phase is waiting on.
    pub(in crate::game_session) fn label(&self) -> &'static str {
        match self {
            Self::NotRequired => "not required",
            Self::HostWaitingForBegin => "host waiting for BeginSim",
            Self::HostWaitingForResyncBegin { .. } => "host waiting for resync BeginSim",
            Self::PeerWaitingForSnapshot => "peer waiting for an initial snapshot",
            Self::PeerWaitingForBegin { .. } => "peer waiting for BeginSim",
            Self::WaitingForStart { .. } => "waiting for the synchronized start gate",
            Self::Running => "running",
        }
    }

    pub(super) fn apply(
        &mut self,
        event: MultiplayerAdmissionEvent,
    ) -> Result<(), MultiplayerSessionError> {
        // Debug, not Display: the trigger's own frames are the reason a phase
        // moved, and a session log without them cannot be replayed by hand.
        let trigger = format!("{event:?}");
        let from = *self;
        *self = match (*self, event) {
            (
                MultiplayerAdmission::WaitingForStart {
                    frame: waiting_frame,
                    start_epoch_ms: waiting_epoch,
                },
                MultiplayerAdmissionEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                },
            ) if frame == waiting_frame && start_epoch_ms == waiting_epoch => *self,

            (
                MultiplayerAdmission::Running | MultiplayerAdmission::WaitingForStart { .. },
                MultiplayerAdmissionEvent::HostResynchronizing { frame },
            ) => MultiplayerAdmission::HostWaitingForResyncBegin {
                snapshot_frame: frame,
            },
            (
                MultiplayerAdmission::HostWaitingForResyncBegin { snapshot_frame },
                MultiplayerAdmissionEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                },
            ) if snapshot_frame == frame => MultiplayerAdmission::WaitingForStart {
                frame,
                start_epoch_ms,
            },
            (_, MultiplayerAdmissionEvent::Disconnected) => {
                MultiplayerAdmission::PeerWaitingForSnapshot
            }
            (
                MultiplayerAdmission::PeerWaitingForSnapshot,
                MultiplayerAdmissionEvent::InitialSnapshotAdopted { frame },
            ) => MultiplayerAdmission::PeerWaitingForBegin {
                snapshot_frame: frame,
            },
            (
                MultiplayerAdmission::PeerWaitingForBegin { snapshot_frame },
                MultiplayerAdmissionEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                },
            ) if frame == snapshot_frame => MultiplayerAdmission::WaitingForStart {
                frame,
                start_epoch_ms,
            },
            (
                MultiplayerAdmission::HostWaitingForBegin,
                MultiplayerAdmissionEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                },
            ) => MultiplayerAdmission::WaitingForStart {
                frame,
                start_epoch_ms,
            },
            (
                MultiplayerAdmission::WaitingForStart {
                    frame: waiting_frame,
                    start_epoch_ms: waiting_epoch,
                },
                MultiplayerAdmissionEvent::StartGateOpened {
                    frame,
                    start_epoch_ms,
                },
            ) if frame == waiting_frame && start_epoch_ms == waiting_epoch => {
                MultiplayerAdmission::Running
            }
            (state, event) => {
                return Err(SessionProtocolFailure::AdmissionOrdering { state, event }.into());
            }
        };
        if from != *self {
            crate::observability::event(
                crate::observability::EventKind::Admission,
                format!(
                    "multiplayer: admission {} → {}",
                    from.wire_name(),
                    self.wire_name()
                ),
            )
            .field("from", from.wire_name())
            .field("to", self.wire_name())
            .field("trigger", trigger)
            .record();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_begin_is_idempotent_only_for_the_same_barrier() {
        let mut state = MultiplayerAdmission::PeerWaitingForBegin {
            snapshot_frame: 8101,
        };
        let begin = MultiplayerAdmissionEvent::BeginSim {
            frame: 8101,
            start_epoch_ms: 1234,
        };
        state.apply(begin).unwrap();
        state.apply(begin).unwrap();
        let waiting = state;
        for event in [
            MultiplayerAdmissionEvent::BeginSim {
                frame: 8102,
                start_epoch_ms: 1234,
            },
            MultiplayerAdmissionEvent::BeginSim {
                frame: 8101,
                start_epoch_ms: 1235,
            },
        ] {
            assert!(state.apply(event).is_err());
            assert_eq!(state, waiting);
        }
    }

    /// The start gate is a transition like any other: the same owner moves the
    /// phase and records the event, and an event that does not name the
    /// barrier the phase waits on is a protocol failure rather than a silent
    /// release.
    #[test]
    fn the_start_gate_opens_through_the_state_machine() {
        let _events = crate::observability::isolate_session_events();
        let barrier = MultiplayerAdmissionEvent::BeginSim {
            frame: 133,
            start_epoch_ms: 1_763_461_265_422,
        };
        let mut state = MultiplayerAdmission::HostWaitingForBegin;
        state.apply(barrier).unwrap();
        assert_eq!(
            state,
            MultiplayerAdmission::WaitingForStart {
                frame: 133,
                start_epoch_ms: 1_763_461_265_422,
            }
        );
        let before =
            crate::observability::events_page(None, crate::observability::MAX_PAGE).next_seq;
        state
            .apply(MultiplayerAdmissionEvent::StartGateOpened {
                frame: 133,
                start_epoch_ms: 1_763_461_265_422,
            })
            .unwrap();
        assert_eq!(state, MultiplayerAdmission::Running);
        let page = crate::observability::events_page(Some(before), crate::observability::MAX_PAGE);
        let opened: Vec<&serde_json::Value> = page
            .events
            .iter()
            .filter(|event| {
                event["detail"]["trigger"]
                    .as_str()
                    .is_some_and(|trigger| trigger.contains("StartGateOpened"))
            })
            .collect();
        assert_eq!(opened.len(), 1, "{page:?}");
        assert_eq!(opened[0]["kind"], "admission");
        assert_eq!(opened[0]["detail"]["from"], "waiting_for_start");
        assert_eq!(opened[0]["detail"]["to"], "running");

        let mut waiting = MultiplayerAdmission::WaitingForStart {
            frame: 800,
            start_epoch_ms: 5,
        };
        assert!(
            waiting
                .apply(MultiplayerAdmissionEvent::StartGateOpened {
                    frame: 801,
                    start_epoch_ms: 5,
                })
                .is_err(),
            "another barrier cannot open this gate"
        );
        assert_eq!(
            waiting,
            MultiplayerAdmission::WaitingForStart {
                frame: 800,
                start_epoch_ms: 5,
            }
        );
    }
}
