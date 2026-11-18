//! Multiplayer session gate state owned by the timeline: ordered admission,
//! the host frame clock with its exactly-once hash publication, and the last
//! rollback telemetry sample.
//!
//! None of this participates in deterministic engine state or the wire format.
//! Transitions that must also discard network reconciliation samples stay on
//! `TimelineRuntime`, which sequences both owners.
use super::MultiplayerAdmission;
use super::timing::MultiplayerTiming;
use crate::game_session::multiplayer::{
    MultiplayerAdmissionEvent, MultiplayerRollbackTelemetry, MultiplayerSessionError,
    SessionProtocolFailure,
};
use serde::Serialize;

/// How long one admission phase may wait for the next message before the
/// session is declared broken. Sized above the host's 120 s load budget and its
/// one ready-barrier extension, so a healthy but slow peer is never cut off.
pub(in crate::game_session) const ADMISSION_BUDGET_MS: u64 = 180_000;

#[derive(Debug, Serialize)]
pub(in crate::game_session) struct MultiplayerSync {
    pub(super) admission: MultiplayerAdmission,
    pub(super) timing: MultiplayerTiming,
    /// Frame of the accepted `BeginSim` barrier. Retained after the gate opens
    /// so `/state` can report the one start boundary this mission used.
    begin_sim_frame: Option<u32>,
    /// The admission phase that is waiting and when it started, in process
    /// uptime milliseconds. Cleared whenever the session runs, so each phase
    /// gets its own budget instead of sharing one long wait.
    #[serde(skip)]
    admission_wait: Option<(MultiplayerAdmission, u32)>,
    /// Process-side diagnostics from the most recent network rollback.
    #[serde(skip)]
    last_rollback: Option<MultiplayerRollbackTelemetry>,
}

impl MultiplayerSync {
    pub(super) fn new(wait_for_multiplayer_start: bool, local_is_host: bool) -> Self {
        Self {
            admission: match (wait_for_multiplayer_start, local_is_host) {
                (false, _) => MultiplayerAdmission::NotRequired,
                (true, true) => MultiplayerAdmission::HostWaitingForBegin,
                (true, false) => MultiplayerAdmission::PeerWaitingForSnapshot,
            },
            timing: MultiplayerTiming::default(),
            begin_sim_frame: None,
            admission_wait: None,
            last_rollback: None,
        }
    }

    pub(in crate::game_session) fn admission(&self) -> MultiplayerAdmission {
        self.admission
    }

    pub(in crate::game_session) fn begin_sim_frame(&self) -> Option<u32> {
        self.begin_sim_frame
    }

    /// One mission accepts one start barrier; a resynchronized replacement
    /// mission records its own release.
    pub(in crate::game_session) fn record_begin_sim(&mut self, frame: u32) {
        self.begin_sim_frame = Some(frame);
    }

    pub(in crate::game_session) fn timing(&self) -> &MultiplayerTiming {
        &self.timing
    }

    pub(in crate::game_session) fn timing_mut(&mut self) -> &mut MultiplayerTiming {
        &mut self.timing
    }

    pub(in crate::game_session) fn last_rollback(&self) -> Option<&MultiplayerRollbackTelemetry> {
        self.last_rollback.as_ref()
    }

    pub(in crate::game_session) fn record_rollback(
        &mut self,
        rollback: MultiplayerRollbackTelemetry,
    ) {
        self.last_rollback = Some(rollback);
    }

    /// Advance the wall-clock release gate and report whether simulation must
    /// remain held for multiplayer admission.
    ///
    /// A phase that waits longer than [`ADMISSION_BUDGET_MS`] fails the session:
    /// the message that would release it belongs to a peer that is gone, and
    /// waiting forever leaves the player on a loaded but frozen mission.
    pub(in crate::game_session) fn admission_paused(
        &mut self,
        now_epoch_ms: u64,
        now_uptime_ms: u32,
    ) -> Result<bool, MultiplayerSessionError> {
        if let MultiplayerAdmission::WaitingForStart {
            frame,
            start_epoch_ms,
        } = self.admission
            && now_epoch_ms >= start_epoch_ms
        {
            self.admission
                .apply(MultiplayerAdmissionEvent::StartGateOpened {
                    frame,
                    start_epoch_ms,
                })?;
        }
        if matches!(
            self.admission,
            MultiplayerAdmission::NotRequired | MultiplayerAdmission::Running
        ) {
            self.admission_wait = None;
            return Ok(false);
        }
        let state = self.admission;
        let started = match self.admission_wait {
            Some((waiting, started)) if waiting == state => started,
            _ => {
                self.admission_wait = Some((state, now_uptime_ms));
                now_uptime_ms
            }
        };
        let waited_ms = u64::from(now_uptime_ms.saturating_sub(started));
        if waited_ms >= ADMISSION_BUDGET_MS {
            crate::observability::event(
                crate::observability::EventKind::BarrierExpired,
                format!(
                    "multiplayer: admission barrier expired after {waited_ms} ms in {}",
                    state.label()
                ),
            )
            .field("state", state.wire_name())
            .field("waited_ms", waited_ms)
            .field("budget_ms", ADMISSION_BUDGET_MS)
            .warn()
            .record();
            return Err(SessionProtocolFailure::AdmissionTimeout { state, waited_ms }.into());
        }
        Ok(true)
    }

    /// `local_frame` is the timeline cursor at receipt; it only feeds the
    /// diagnostic deadline delta, never the accepted schedule itself.
    pub(in crate::game_session) fn accept_host_frame_schedule(
        &mut self,
        frame: u32,
        delay_ms: u32,
        local_frame: u32,
    ) {
        let now_ms = crate::window::process_uptime_ms();
        if !self.timing.accept_schedule(frame, delay_ms, now_ms) {
            tracing::trace!(
                clock_frame = frame,
                current_sample_frame = self.timing.schedule_frame(),
                "multiplayer: ignored stale host frame schedule"
            );
            return;
        }
        tracing::info!(
            host_clock_frame = frame,
            ms_until_next_frame = delay_ms,
            local_frame_at_receive = local_frame,
            deadline_delta_ms_for_local_frame = self
                .timing
                .deadline_ms(local_frame)
                .expect("schedule just installed")
                - i64::from(now_ms),
            "multiplayer: received host frame schedule"
        );
    }

    /// Both graphical and headless drivers publish through this consuming
    /// transition. A failed send is not retried: NetChannels latches worker
    /// failure for the next ingress poll, just as before this extraction.
    pub(in crate::game_session) fn publish_timing(
        &mut self,
        transport: &crate::host::HostTransport,
        clock_frame: u32,
        remaining_sleep_ms: u32,
    ) {
        let Some(net) = transport.net() else { return };
        if transport.local_seat() != robin_engine::player_command::PlayerId::HOST {
            return;
        }
        let Some(sample) = self.timing.take_publication() else {
            return;
        };
        net.publish_frame(clock_frame);
        tracing::info!(
            hash_frame = sample.frame,
            clock_frame,
            remaining_sleep_ms,
            "multiplayer: host sending state hash timing sample"
        );
        if let Err(error) =
            net.send_state_hash(sample.frame, sample.hash, clock_frame, remaining_sleep_ms)
        {
            tracing::error!(%error, "multiplayer state hash publication failed");
        }
    }
}

robin_util::deny_deserialize!(
    MultiplayerSync,
    "multiplayer session gates are live timeline authority, not a saved game"
);
