//! Native server transport lifecycle; shared framing stays in the parent.
use super::*;

/// The campaign-scoped iroh endpoint and its long-lived accept runtime. Bound
/// once for the whole host-owned campaign; each mission only rotates the
/// dispatch slot, so the direct address a joiner dials never changes (D07/H36).
pub(super) struct CampaignEndpoint {
    pub(super) endpoint_id: EndpointId,
    pub(super) endpoint_addr: EndpointAddr,
    /// Shared with the long-lived accept loop: every mission installs its
    /// context here, so openings reach the currently active generation.
    pub(super) dispatch: Arc<Mutex<Option<Arc<ServerContext>>>>,
    pub(super) shutdown_tx: tokio::sync::watch::Sender<bool>,
    pub(super) runtime_thread: Option<JoinHandle<()>>,
}

/// Bind the one endpoint a campaign owns and run its accept loop for the whole
/// campaign. The runtime thread outlives every per-mission runtime; only
/// campaign teardown signals `shutdown_tx` and closes the endpoint.
pub(super) fn bind_campaign_endpoint(
    key: SecretKey,
    retained_relay: Option<iroh::RelayUrl>,
    browser_join_enabled: bool,
    dispatch: Arc<Mutex<Option<Arc<ServerContext>>>>,
) -> std::io::Result<(
    EndpointId,
    EndpointAddr,
    JoinHandle<()>,
    tokio::sync::watch::Sender<bool>,
)> {
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    let (startup_tx, startup_rx) =
        std::sync::mpsc::sync_channel::<Result<(EndpointId, EndpointAddr), MultiplayerError>>(1);
    let runtime_thread = thread::Builder::new()
        .name("mp-campaign-endpoint".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = startup_tx
                        .send(Err(MultiplayerError::transport("build tokio runtime", e)));
                    return;
                }
            };
            rt.block_on(async move {
                let endpoint = match bind_endpoint_with_relay(key, GAME_ALPN, retained_relay).await {
                    Ok(endpoint) => endpoint,
                    Err(e) => {
                        let _ = startup_tx.send(Err(e));
                        return;
                    }
                };
                if browser_join_enabled {
                    if tokio::time::timeout(Duration::from_secs(15), endpoint.online())
                        .await
                        .is_err()
                    {
                        endpoint.close().await;
                        let _ = startup_tx.send(Err(MultiplayerError::Unavailable(
                            "iroh relay did not become reachable within 15 seconds; disable browser join-link publication for a native-only game"
                                .into(),
                        )));
                        return;
                    }
                    if endpoint.addr().relay_urls().next().is_none() {
                        endpoint.close().await;
                        let _ = startup_tx.send(Err(MultiplayerError::Unavailable(
                            "iroh reported online without a relay URL; a browser invitation cannot be published"
                                .into(),
                        )));
                        return;
                    }
                }
                let id = endpoint.id();
                let addr = endpoint.addr();
                if startup_tx.send(Ok((id, addr.clone()))).is_err() {
                    endpoint.close().await;
                    return;
                }
                let accept = tokio::spawn(run_campaign_accept_loop(
                    Arc::clone(&dispatch),
                    endpoint.clone(),
                ));
                let _ = shutdown_rx.wait_for(|stop| *stop).await;
                accept.abort();
                let _ = accept.await;
                endpoint.close().await;
                tracing::info!("campaign multiplayer endpoint stopped");
            });
        })?;

    let (id, addr) = match startup_rx.recv() {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => {
            let _ = shutdown_tx.send(true);
            let _ = runtime_thread.join();
            return Err(std::io::Error::other(err));
        }
        Err(e) => {
            let _ = shutdown_tx.send(true);
            let _ = runtime_thread.join();
            return Err(std::io::Error::other(MultiplayerError::transport(
                "campaign endpoint startup channel closed",
                e,
            )));
        }
    };
    Ok((id, addr, runtime_thread, shutdown_tx))
}

/// Install this mission as the campaign accept loop's dispatch only once its
/// runtime thread exists. A failed spawn must leave no dead context installed
/// and must not retain the campaign lease through a stale dispatch.
pub(super) fn install_mission_dispatch(
    dispatch: &Mutex<Option<Arc<ServerContext>>>,
    context: Arc<ServerContext>,
    spawned: std::io::Result<JoinHandle<()>>,
) -> std::io::Result<JoinHandle<()>> {
    let handle = spawned?;
    *dispatch.lock() = Some(context);
    Ok(handle)
}

/// Retire this mission's admission authority. Called at the very start of
/// `shutdown`, before the commit drain or thread joins, so an in-flight
/// `handle_incoming_peer` cannot admit a frozen-address reconnect into a
/// generation that is already ending. Live generations keep H17 reconnect.
pub(super) fn retire_dispatch(
    dispatch: &Mutex<Option<Arc<ServerContext>>>,
    cancellation: &AtomicBool,
) {
    cancellation.store(true, Ordering::Release);
    *dispatch.lock() = None;
}

/// Accept loop for the campaign's one endpoint. Each opening is handed to the
/// mission whose dispatch is currently installed; with no active mission the
/// connection is dropped rather than admitted into a stale generation.
async fn run_campaign_accept_loop(
    dispatch: Arc<Mutex<Option<Arc<ServerContext>>>>,
    endpoint: Endpoint,
) {
    while let Some(incoming) = endpoint.accept().await {
        let Some(context) = dispatch.lock().clone() else {
            tracing::warn!("multiplayer: incoming connection with no active mission dispatch");
            continue;
        };
        tokio::spawn(async move {
            let cancelled = context.cancellation.load(Ordering::Acquire);
            if let Err(e) = handle_incoming_peer(Arc::clone(&context), incoming).await
                && !cancelled
                && !context.cancellation.load(Ordering::Acquire)
            {
                tracing::warn!("incoming peer handler ended: {e}");
            }
        });
    }
    tracing::info!("multiplayer accept loop stopped");
}

// ─── Server ──────────────────────────────────────────────────────

/// Handle to one mission's dispatch on a campaign-owned iroh endpoint.
///
/// Shutdown is deterministic: the shutdown signal ends the mission's outgoing
/// pump and watchdog, the commit drain finishes, and the mission runtime plus
/// its bridge thread are joined before `shutdown` returns. The endpoint stays
/// bound — it belongs to the campaign and is closed only at teardown, so a
/// joiner's frozen connect string survives mission rotation (D07 / H36).
/// Drop the handle before creating the next mission transport: retained
/// handles keep the campaign lease so they cannot publish stale handoffs.
pub struct ServerHandle {
    /// `(local_seat, mission_seed)` the server is operating with.
    pub local_seat: PlayerId,
    pub mission_seed: u64,
    pub(super) session_id: MultiplayerSessionId,
    pub(super) endpoint_id: EndpointId,
    pub(super) endpoint_addr: EndpointAddr,
    pub(super) host_key: SecretKey,
    pub(super) mission_id: String,
    pub(super) context: Arc<ServerContext>,
    pub(super) preserve_on_shutdown: bool,
    pub(super) cancellation: Arc<AtomicBool>,
    pub(super) shutdown_tx: tokio::sync::watch::Sender<bool>,
    pub(super) runtime_thread: Option<JoinHandle<()>>,
    pub(super) bridge_thread: Option<JoinHandle<()>>,
    /// The campaign accept loop reads this to route openings to the active
    /// mission. A shutdown clears it; the next mission installs its own.
    pub(super) dispatch: Arc<Mutex<Option<Arc<ServerContext>>>>,
}

impl ServerHandle {
    pub fn session_id(&self) -> MultiplayerSessionId {
        self.session_id
    }

    /// The stable public id peers dial to reach this server.
    pub fn endpoint_id(&self) -> EndpointId {
        self.endpoint_id
    }

    /// Full endpoint address (id + transport addresses) as a connect
    /// string for [`connect_client`].  Lets peers dial explicit direct
    /// addresses without relay/DNS lookup (tests, LAN-only setups).
    pub fn connect_string(&self) -> String {
        serde_json::to_string(&self.endpoint_addr).expect("EndpointAddr serialization cannot fail")
    }

    pub fn browser_join_ticket(
        &self,
        content_edition: crate::multiplayer::join_ticket::BrowserContentEdition,
        content_identity_sha256: String,
        mission_profile_id: Option<u32>,
        expected_players: u32,
        lifetime: crate::multiplayer::join_ticket::InvitationLifetime,
    ) -> Result<crate::multiplayer::join_ticket::BrowserJoinTicket, MultiplayerError> {
        crate::multiplayer::join_ticket::BrowserJoinTicket::issue(
            &self.host_key,
            &self.endpoint_addr,
            self.session_id.0,
            try_current_epoch_ms()? / 1000,
            crate::multiplayer::join_ticket::BrowserJoinTicketContent {
                content_edition,
                content_identity_sha256,
                mission_id: self.mission_id.clone(),
                mission_profile_id,
                expected_players,
            },
            lifetime,
        )
    }

    pub fn shutdown(&mut self) {
        // Retire admission before the commit drain or any join: a connection
        // already in flight must not claim a seat on this dying mission.
        retire_dispatch(&self.dispatch, &self.cancellation);
        if self.preserve_on_shutdown {
            publish_context_continuation(&self.context);
            self.preserve_on_shutdown = false;
        } else {
            self.context.host_left.store(true, Ordering::Release);
        }
        let _ = self.shutdown_tx.send(true);
        if let Some(handle) = self.runtime_thread.take()
            && handle.join().is_err()
        {
            tracing::error!("multiplayer mission runtime panicked during shutdown");
        }
        if let Some(handle) = self.bridge_thread.take()
            && handle.join().is_err()
        {
            tracing::error!("multiplayer server outgoing bridge panicked during shutdown");
        }
        // The socket stays bound until campaign teardown so the joiner's frozen
        // connect string remains valid across the handoff (D07 / H36). Admission
        // was already retired at the top of this method.
    }

    pub(in crate::multiplayer) fn preserve_session_for_next_mission(&mut self) {
        self.preserve_on_shutdown = true;
    }

    /// Seats whose acknowledgement the pending snapshot-transition commit
    /// still waits for, or `None` when no transition is pending.
    pub(crate) fn transition_awaiting_seats(&self) -> Option<Vec<u8>> {
        let peers = self.context.peers.lock();
        let mut awaiting = peers
            .transitions
            .pending()?
            .awaiting
            .iter()
            .copied()
            .collect::<Vec<_>>();
        awaiting.sort_unstable();
        Some(awaiting)
    }
}

pub(super) fn publish_context_continuation(context: &ServerContext) {
    let peers = context.peers.lock();
    let owner_seats = peers.sessions.owner_seats();
    publish_host_session_continuation(
        &context.campaign,
        HostSessionContinuation {
            host_endpoint_id: context.host_endpoint_id,
            session_id: context.session_id,
            expected_players: peers.sessions.expected_players(),
            owner_seats,
            relay_url: context.relay_url.lock().clone(),
        },
    );
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Per-peer state tracked by the server.  Wrapped in an `Arc<Mutex<>>`
/// so the accept task, the outgoing pump, and each per-peer task can
/// share access.
pub(super) struct ServerPeers {
    pub(super) sessions: PeerSessions,
    pub(super) readiness: ReadyBarrier,
    pub(super) transitions: SnapshotTransitions,
    pub(super) modals: robin_engine::multiplayer::ModalRecoveryState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) enum SeatClaimKind {
    Fresh,
    Reconnect,
    ActiveReplacement,
}

#[derive(Debug)]
pub(super) struct SeatClaim {
    pub(super) seat: u8,
    pub(super) generation: u64,
    pub(super) kind: SeatClaimKind,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum InactivePeerSession {
    Released,
    Superseded { current_generation: u64 },
    Detached,
}

/// Not serde: `Protocol` carries the typed transport error.
#[derive(Clone, Debug)]
pub(super) enum PeerDispatchFailure {
    Inactive {
        seat: PlayerId,
        generation: u64,
        kind: InactivePeerSession,
    },
    Protocol(MultiplayerError),
}

impl std::error::Error for PeerDispatchFailure {}

impl std::fmt::Display for PeerDispatchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inactive {
                seat,
                generation,
                kind,
            } => {
                write!(f, "peer {seat:?} generation {generation} ")?;
                match kind {
                    InactivePeerSession::Released => {
                        f.write_str("has no active authenticated session")
                    }
                    InactivePeerSession::Superseded { current_generation } => {
                        write!(f, "was superseded by generation {current_generation}")
                    }
                    InactivePeerSession::Detached => f.write_str("has a detached writer"),
                }
            }
            Self::Protocol(error) => error.fmt(f),
        }
    }
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum PeerReaderExit {
    Closed,
    /// No new reader effects are authorized, but the old writer may own
    /// terminal frames queued before detachment, replacement, or release.
    Inactive,
}

pub(super) fn peer_reader_dispatch_result(
    result: Result<(), PeerDispatchFailure>,
) -> Result<Option<PeerReaderExit>, MultiplayerError> {
    match result {
        Ok(()) => Ok(None),
        Err(error @ PeerDispatchFailure::Inactive { .. }) => {
            tracing::debug!(%error, "peer reader lost authority; draining its existing writer");
            Ok(Some(PeerReaderExit::Inactive))
        }
        Err(PeerDispatchFailure::Protocol(error)) => Err(error),
    }
}

impl ServerPeers {
    pub(super) fn new(expected_players: u32) -> Self {
        Self {
            sessions: PeerSessions::new(expected_players),
            readiness: ReadyBarrier::default(),
            transitions: SnapshotTransitions::default(),
            modals: Default::default(),
        }
    }

    pub(super) fn from_continuation(continuation: &HostSessionContinuation) -> Self {
        Self {
            sessions: PeerSessions::from_continuation(continuation),
            readiness: ReadyBarrier::default(),
            transitions: SnapshotTransitions::default(),
            modals: Default::default(),
        }
    }

    /// Publish the seats this session's simulation contains.
    ///
    /// A seat is in it once the transport released it into the mission: the
    /// start barrier has committed (so `BeginSim` was sent to every attached
    /// admitted seat), no snapshot transition is pending (a seat admitted
    /// during one has not been released into either mission), and its writer is
    /// still attached (a detached one cannot be sent the page). Before that,
    /// every admitted peer is still waiting for `BeginSim` — the state the #43
    /// capture found a shared page waiting on — so the mask is the host alone.
    /// A shared page then decides on the host's own outcome instead of waiting
    /// for a seat that cannot see it.
    pub(super) fn publish_session_seats(&self, seats: &robin_engine::multiplayer::SessionSeats) {
        let mut mask = 1u32;
        if self.readiness.begun.is_some() && self.transitions.pending().is_none() {
            for seat in self.sessions.simulation_seats() {
                mask |= 1u32 << *seat;
            }
        }
        seats.publish(mask);
    }
}

/// Publish [`ServerPeers::publish_session_seats`] to the game loop.
///
/// Called after every mutation of the admitted seat set, outside the peer lock:
/// admission (`connect_all_provisional_seats`), release
/// (`release_peer_session`), eviction (every `maybe_begin_sim_locked` caller),
/// the start barrier's release (`retry_begin_sim`, the ready dispatch) and a
/// full-snapshot resynchronization, which drops every seat.
pub(super) fn publish_session_seats(context: &ServerContext) {
    let peers = context.peers.lock();
    peers.publish_session_seats(&context.session_seats);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(super) enum PeerOwner {
    Native([u8; 32]),
    Browser([u8; 32]),
}

pub(super) fn take_committed_snapshot_transition(
    peers: &mut ServerPeers,
) -> Option<(
    robin_engine::multiplayer::SnapshotTransitionId,
    Vec<(u8, UnboundedSender<NetMsg>)>,
)> {
    let id = peers.transitions.take_completed()?;
    let senders = peers.sessions.detach_all_writers_with_seats();
    Some((id, senders))
}

pub(super) fn retain_transition_peer_for_reconnect(peers: &mut ServerPeers, seat: u8) {
    peers.transitions.retain_for_reconnect(seat);
}

pub(super) fn commit_snapshot_transition(
    context: &ServerContext,
    committed: Option<(
        robin_engine::multiplayer::SnapshotTransitionId,
        Vec<(u8, UnboundedSender<NetMsg>)>,
    )>,
) {
    let Some((id, senders)) = committed else {
        return;
    };
    tracing::info!(
        transition = ?id,
        peers = senders.len(),
        "multiplayer: snapshot transition committed"
    );
    publish_context_continuation(context);
    let mut written = HashMap::new();
    for (seat, _) in &senders {
        written.insert(*seat, Arc::new(tokio::sync::Notify::new()));
    }
    // Install the drain before queueing any frame so a writer that signals
    // immediately is never lost to a not-yet-installed drain.
    *context.commit_drain.lock() = Some(CommitDrain {
        senders: senders.clone(),
        written,
    });
    for (_, sender) in &senders {
        sender
            .send(NetMsg::CommitSnapshotTransition { id })
            .unwrap_or_else(|_| {
                panic!("snapshot transition commit queue closed before peer delivery")
            });
    }
    context
        .incoming_tx
        .send(NetEvent::CommitSnapshotTransition { id })
        .unwrap_or_else(|_| panic!("snapshot transition host event channel is closed"));
    // The detached writer queues are kept open by the drain above until the
    // runtime has seen every seat's commit frame. Dropping them here races the
    // writer and lets the client read `connection lost` instead of the commit
    // (D07 / H36).
    // Every writer is detached until the replacement mission admits its seats
    // again, so no shared page may await one in the meantime.
    publish_session_seats(context);
}

/// Bounded host-side drain state for one committed snapshot transition.
pub(super) struct CommitDrain {
    /// Detached writer queues held open so each peer can finish writing its
    /// commit frame. Released by the runtime after the bounded drain.
    pub(super) senders: Vec<(u8, UnboundedSender<NetMsg>)>,
    /// One signal per seat, fired once its commit frame is on the wire (or the
    /// writer ends). `Notify` stores a permit, so a signal arriving before the
    /// runtime starts awaiting is still observed.
    pub(super) written: HashMap<u8, Arc<tokio::sync::Notify>>,
}

/// Record that a seat can no longer produce a commit frame — either it wrote
/// one, or its connection/writer ended. Wakes the host's bounded drain so a
/// disconnected peer never stalls shutdown.
pub(super) fn notify_commit_frame_written(context: &ServerContext, seat: u8) {
    let guard = context.commit_drain.lock();
    if let Some(drain) = guard.as_ref()
        && let Some(written) = drain.written.get(&seat)
    {
        written.notify_one();
    }
}

/// Wait up to `budget` for every detached writer to report its commit frame
/// written, then release the retained queues. A peer that never writes is
/// released by the deadline — never an unbounded wait.
pub(super) async fn await_commit_frame_drains(context: &ServerContext, budget: Duration) {
    let notifies: Vec<(u8, Arc<tokio::sync::Notify>)> = {
        let guard = context.commit_drain.lock();
        match guard.as_ref() {
            Some(drain) => drain
                .written
                .iter()
                .map(|(seat, written)| (*seat, Arc::clone(written)))
                .collect(),
            None => return,
        }
    };
    for (seat, written) in notifies {
        if tokio::time::timeout(budget, written.notified())
            .await
            .is_err()
        {
            tracing::warn!(
                seat,
                budget_ms = budget.as_millis(),
                "multiplayer: commit frame was not written before the drain deadline; closing endpoint anyway"
            );
        }
    }
    // Closing the retained queues ends the peer writers. Every commit frame
    // is on the wire by now, or the deadline released a dead peer.
    if let Some(drain) = context.commit_drain.lock().take() {
        drop(drain.senders);
    }
}

/// Host monotonic milliseconds. Every barrier deadline is expressed in this
/// clock so tests can advance it without sleeping.
pub(super) fn elapsed_ms(context: &ServerContext) -> u64 {
    u64::try_from(context.started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

pub(super) fn maybe_begin_sim_locked(
    peers: &mut ServerPeers,
    now_ms: u64,
) -> Result<Option<(u32, u64, Vec<UnboundedSender<NetMsg>>)>, MultiplayerError> {
    if peers.transitions.pending().is_some() {
        return Ok(None);
    }
    let decision = peers.readiness.candidate(
        now_ms,
        peers.sessions.expected_players(),
        peers.sessions.readiness(),
    );
    let begin_frame = match decision {
        ReadyDecision::Wait => return Ok(None),
        ReadyDecision::Begin(frame) => frame,
        ReadyDecision::Expired => {
            let evicted = peers.sessions.evict_unready_seats();
            tracing::warn!(
                evicted = ?evicted,
                budget_ms = READY_BUDGET_MS,
                waited_ms = peers.readiness.waited_ms(now_ms),
                "multiplayer: ready barrier expired; evicting peers that never became ready"
            );
            peers
                .readiness
                .forced_frame(peers.sessions.readiness())
                .expect("the host frame exists whenever the barrier is evaluated")
        }
    };
    let start_epoch_ms = try_current_epoch_ms()?.checked_add(500).ok_or_else(|| {
        MultiplayerError::LocalState(
            "multiplayer BeginSim timestamp exceeds the u64 Unix range".into(),
        )
    })?;
    let senders = peers
        .sessions
        .senders()
        .map(|(_, sender)| sender)
        .cloned()
        .collect();
    peers.readiness.commit(begin_frame, start_epoch_ms);
    Ok(Some((begin_frame, start_epoch_ms, senders)))
}

/// Per-session context shared by every server task.
pub(super) struct ServerContext {
    pub(super) _campaign_lease: CampaignServerLease,
    pub(super) campaign: Arc<CampaignTransportState>,
    // Outermost lock: each synchronous protocol event is atomic relative to
    // claim/release/writer detachment. Never hold it across an await.
    pub(super) session_dispatch: Mutex<()>,
    pub(super) peers: Mutex<ServerPeers>,
    pub(super) incoming_tx: Sender<NetEvent>,
    pub(super) host_nickname: String,
    pub(super) mission_id: String,
    pub(super) mission_seed: u64,
    pub(super) sim_config: robin_engine::engine::SimConfig,
    pub(super) host_endpoint_id: EndpointId,
    pub(super) session_id: MultiplayerSessionId,
    pub(super) continued_session: bool,
    pub(super) relay_url: Mutex<Option<iroh::RelayUrl>>,
    pub(super) speech_timing_locale: Option<String>,
    pub(super) frame_cursor: FrameCursor,
    pub(super) initial_snapshot: InitialSnapshot,
    /// The game loop's seat mask: this session's simulation, published on every
    /// change so a shared page never awaits a seat that is not in it.
    pub(super) session_seats: robin_engine::multiplayer::SessionSeats,
    pub(super) content: Option<HostedModContent>,
    pub(super) cancellation: Arc<AtomicBool>,
    pub(super) shutdown_tx: tokio::sync::watch::Sender<bool>,
    pub(super) host_left: AtomicBool,
    pub(super) connections: Mutex<std::collections::HashMap<u8, iroh::endpoint::Connection>>,
    /// Writers detached at commit, held open until every commit frame is
    /// written (or the bounded drain deadline fires) before endpoint close.
    pub(super) commit_drain: Mutex<Option<CommitDrain>>,
    /// Monotonic anchor for barrier deadlines. Never serialized; deadlines
    /// stored in peer state are elapsed milliseconds from this instant.
    pub(super) started: std::time::Instant,
}

pub(super) fn fail_server(context: &ServerContext, error: MultiplayerError) {
    if !context.cancellation.swap(true, Ordering::AcqRel) {
        let _ = context
            .incoming_tx
            .send(NetEvent::Fatal(NetFatal::new(error)));
    }
    let _ = context.shutdown_tx.send(true);
}

/// Start with an explicit identity key and a throwaway campaign. Tests use
/// this to avoid touching the per-install on-disk identity; production hosting
/// uses [`start_server_in_campaign`].
#[cfg(test)]
pub(in crate::multiplayer) fn start_server_with_key(
    key: SecretKey,
    config: ServerConfig,
    channels: ServerChannels,
    content: Option<HostedModContent>,
) -> std::io::Result<ServerHandle> {
    // The campaign owns the endpoint for its whole lifetime. Test callers do
    // not hold one, so keep this throwaway campaign alive for the process.
    let campaign = Box::leak(Box::new(MultiplayerCampaignSession::default()));
    start_server_inner(campaign, key, config, channels, content)
}

/// Immutable identity and admission policy for one hosted mission.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerConfig {
    pub host_nickname: String,
    pub mission_id: String,
    pub mission_seed: u64,
    pub sim_config: robin_engine::engine::SimConfig,
    pub speech_timing_locale: Option<String>,
    pub expected_players: u32,
    pub browser_join_enabled: bool,
}

/// The transport-side ends of one mission's game-loop channels. The matching
/// ends stay in the [`NetChannels`](crate::multiplayer::NetChannels) created
/// alongside them by [`NetChannels::new_server`](crate::multiplayer::NetChannels::new_server).
///
/// Not serde: these are live channel ends and shared slots, not data.
pub struct ServerChannels {
    /// Events the server publishes to the game loop.
    pub incoming_tx: Sender<NetEvent>,
    /// Messages the game loop queues for broadcast.
    pub outgoing_rx: Receiver<NetOutbound>,
    /// The game loop's current simulation frame.
    pub frame_cursor: FrameCursor,
    /// Latest full snapshot handed to joining peers.
    pub initial_snapshot: InitialSnapshot,
    /// The seats in the game loop's simulation, which a shared page may await.
    pub session_seats: robin_engine::multiplayer::SessionSeats,
}

impl crate::multiplayer::NetChannels {
    /// [`NetChannels::new`](Self::new) with the transport-side ends bundled
    /// for [`start_server_in_campaign`].
    pub fn new_server() -> (Self, ServerChannels) {
        let (channels, incoming_tx, outgoing_rx, frame_cursor, initial_snapshot) = Self::new();
        let session_seats = channels.session_seats().clone();
        (
            channels,
            ServerChannels {
                incoming_tx,
                outgoing_rx,
                frame_cursor,
                initial_snapshot,
                session_seats,
            },
        )
    }
}

/// Start a mission transport within an explicitly owned campaign.
pub fn start_server_in_campaign(
    campaign: &MultiplayerCampaignSession,
    config: ServerConfig,
    channels: ServerChannels,
    content: Option<HostedModContent>,
) -> std::io::Result<ServerHandle> {
    start_server_inner(
        campaign,
        game_secret_key().map_err(std::io::Error::other)?,
        config,
        channels,
        content,
    )
}

pub(super) fn start_server_inner(
    campaign: &MultiplayerCampaignSession,
    key: SecretKey,
    config: ServerConfig,
    channels: ServerChannels,
    content: Option<HostedModContent>,
) -> std::io::Result<ServerHandle> {
    let ServerConfig {
        host_nickname,
        mission_id,
        mission_seed,
        sim_config,
        speech_timing_locale,
        expected_players,
        browser_join_enabled,
    } = config;
    // Unpacked before the first fallible step, so every channel end is moved
    // or dropped at exactly the points the former by-value parameters were.
    let ServerChannels {
        incoming_tx,
        outgoing_rx,
        frame_cursor,
        initial_snapshot,
        session_seats,
    } = channels;
    let campaign_lease = campaign.reserve_server()?;
    robin_engine::multiplayer::validate_display_name(&host_nickname)
        .map_err(std::io::Error::other)?;
    robin_engine::multiplayer::validate_mission_id(&mission_id).map_err(std::io::Error::other)?;
    if !(1..=crate::multiplayer::MAX_MULTIPLAYER_PLAYERS).contains(&expected_players) {
        return Err(std::io::Error::other(format!(
            "multiplayer expected-player count must be between 1 and {}, got {expected_players}",
            crate::multiplayer::MAX_MULTIPLAYER_PLAYERS
        )));
    }
    let host_endpoint_id = key.public();
    if let Some(content) = &content {
        if content.validated.package.manifest.mission_basename != mission_id {
            return Err(std::io::Error::other(format!(
                "hosted full mod contains mission `{}`, but server announces `{mission_id}`",
                content.validated.package.manifest.mission_basename
            )));
        }
        content
            .offer(host_endpoint_id.to_string())
            .map_err(std::io::Error::other)?;
    }
    let continuation =
        pending_host_session_continuation(campaign.state(), host_endpoint_id, expected_players)
            .map_err(std::io::Error::other)?;
    let session_id = continuation.as_ref().map_or_else(
        || MultiplayerSessionId(SecretKey::generate().to_bytes()),
        |continuation| continuation.session_id,
    );
    let handle_key = key.clone();
    let handle_mission_id = mission_id.clone();
    let cancellation = Arc::new(AtomicBool::new(false));
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let (bridge_thread, outgoing_async_rx) = spawn_outgoing_bridge(
        "mp-server-outgoing-bridge",
        outgoing_rx,
        Arc::clone(&cancellation),
    )?;

    // One endpoint for the whole campaign. The first mission binds it; every
    // later mission reuses the same id/address and only rotates dispatch.
    let campaign_state = Arc::clone(campaign.state());
    let (endpoint_id, endpoint_addr, dispatch) = {
        let mut slot = campaign_state.endpoint.lock();
        match slot.as_ref() {
            Some(existing) => {
                if let Some(continuation) = &continuation {
                    assert_eq!(
                        continuation.host_endpoint_id, existing.endpoint_id,
                        "campaign continuation belongs to a different host endpoint"
                    );
                }
                (
                    existing.endpoint_id,
                    existing.endpoint_addr.clone(),
                    Arc::clone(&existing.dispatch),
                )
            }
            None => {
                if continuation.is_some() {
                    // A decoded campaign has no live endpoint; never invent one.
                    cancellation.store(true, Ordering::Release);
                    let _ = shutdown_tx.send(true);
                    let _ = bridge_thread.join();
                    return Err(std::io::Error::other(
                        "decoded campaign has no live multiplayer endpoint",
                    ));
                }
                let dispatch: Arc<Mutex<Option<Arc<ServerContext>>>> = Arc::new(Mutex::new(None));
                let (id, addr, thread, campaign_shutdown_tx) = bind_campaign_endpoint(
                    key.clone(),
                    None,
                    browser_join_enabled,
                    Arc::clone(&dispatch),
                )
                .map_err(std::io::Error::other)?;
                *slot = Some(CampaignEndpoint {
                    endpoint_id: id,
                    endpoint_addr: addr.clone(),
                    dispatch: Arc::clone(&dispatch),
                    shutdown_tx: campaign_shutdown_tx,
                    runtime_thread: Some(thread),
                });
                (id, addr, dispatch)
            }
        }
    };

    let context = Arc::new(ServerContext {
        _campaign_lease: campaign_lease,
        campaign: Arc::clone(campaign.state()),
        session_dispatch: Mutex::new(()),
        peers: Mutex::new(continuation.as_ref().map_or_else(
            || ServerPeers::new(expected_players.max(1)),
            ServerPeers::from_continuation,
        )),
        incoming_tx,
        host_nickname,
        mission_id,
        mission_seed,
        sim_config,
        host_endpoint_id,
        session_id,
        continued_session: continuation.is_some(),
        host_left: AtomicBool::new(false),
        connections: Mutex::new(std::collections::HashMap::new()),
        commit_drain: Mutex::new(None),
        relay_url: Mutex::new(
            endpoint_addr
                .relay_urls()
                .next()
                .cloned()
                .or_else(|| continuation.as_ref().and_then(|c| c.relay_url.clone())),
        ),
        speech_timing_locale,
        frame_cursor,
        initial_snapshot,
        session_seats,
        content,
        cancellation: Arc::clone(&cancellation),
        shutdown_tx: shutdown_tx.clone(),
        started: std::time::Instant::now(),
    });
    // The session starts with its host alone in the simulation: no peer has
    // been released into it, so no shared page may await one.
    publish_session_seats(&context);

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(error) => {
            cancellation.store(true, Ordering::Release);
            let _ = shutdown_tx.send(true);
            let _ = bridge_thread.join();
            return Err(std::io::Error::other(MultiplayerError::transport(
                "build tokio runtime",
                error,
            )));
        }
    };
    let cancellation_for_thread = Arc::clone(&cancellation);
    let runtime_thread = match install_mission_dispatch(
        &dispatch,
        Arc::clone(&context),
        thread::Builder::new().name("mp-server".into()).spawn({
            let context = Arc::clone(&context);
            move || {
                rt.block_on(run_mission_runtime(context, outgoing_async_rx, shutdown_rx));
                cancellation_for_thread.store(true, Ordering::Release);
            }
        }),
    ) {
        Ok(handle) => handle,
        Err(error) => {
            cancellation.store(true, Ordering::Release);
            let _ = shutdown_tx.send(true);
            let _ = bridge_thread.join();
            return Err(error);
        }
    };

    // Only successful startup consumes the handoff. Validation, thread or
    // endpoint failures leave it available for an explicit retry.
    *campaign.state().continuation.lock() = None;
    tracing::info!(
        endpoint_id = %endpoint_id,
        seed = mission_seed,
        "multiplayer server listening on iroh"
    );

    Ok(ServerHandle {
        local_seat: PlayerId::HOST,
        mission_seed,
        session_id,
        endpoint_id,
        endpoint_addr,
        host_key: handle_key,
        mission_id: handle_mission_id,
        context,
        preserve_on_shutdown: false,
        cancellation,
        shutdown_tx,
        runtime_thread: Some(runtime_thread),
        bridge_thread: Some(bridge_thread),
        dispatch,
    })
}

/// Per-mission runtime on the campaign-owned endpoint. Owns only the outgoing
/// pump, the barrier watchdog and the shutdown/commit-drain lifecycle. The
/// socket and accept loop belong to the campaign and stay bound across mission
/// rotation, so a joiner's frozen connect string remains valid (D07 / H36).
pub(super) async fn run_mission_runtime(
    context: Arc<ServerContext>,
    outgoing_async_rx: UnboundedReceiver<NetOutbound>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut pump = tokio::spawn(run_server_outgoing_pump(
        Arc::clone(&context),
        outgoing_async_rx,
    ));
    let watchdog = tokio::spawn(run_barrier_watchdog(Arc::clone(&context)));

    let pump_result = tokio::select! {
        _ = shutdown_rx.wait_for(|stop| *stop) => None,
        result = &mut pump => Some(result),
    };
    let pump_finished = pump_result.is_some();
    if let Some(result) = pump_result {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                context.cancellation.store(true, Ordering::Release);
                let _ = context
                    .incoming_tx
                    .send(NetEvent::Fatal(NetFatal::new(error)));
            }
            Err(error) => {
                context.cancellation.store(true, Ordering::Release);
                let _ = context.incoming_tx.send(NetEvent::Fatal(NetFatal::new(
                    MultiplayerError::transport("multiplayer server outgoing pump failed", error),
                )));
            }
        }
    }
    if context.host_left.load(Ordering::Acquire) {
        for connection in context.connections.lock().values() {
            connection.close(
                crate::multiplayer::client_session::HOST_LEFT_CLOSE_CODE.into(),
                b"The host has left the game.",
            );
        }
    }
    // Drain every seat's commit frame before this mission's dispatch ends. The
    // endpoint stays bound for the replacement mission (D07 / H36).
    await_commit_frame_drains(&context, COMMIT_FRAME_DRAIN_TIMEOUT).await;
    watchdog.abort();
    let _ = watchdog.await;
    if !pump_finished {
        pump.abort();
        let _ = pump.await;
    }
    tracing::info!("multiplayer mission runtime stopped");
}

pub(super) fn publish_connect_seats(context: &ServerContext, seats: Vec<(u8, String)>) {
    for (seat, nickname) in seats {
        let now = context.frame_cursor.load(Ordering::Relaxed);
        let target = now.saturating_add(INPUT_DELAY_FRAMES);
        broadcast_input(
            context,
            now,
            now,
            target,
            PlayerInput::new(
                PlayerId::HOST,
                PlayerCommand::ConnectSeat {
                    player_id: PlayerId(seat),
                    nickname,
                },
            ),
        );
    }
}

pub(super) fn retry_begin_sim(context: &ServerContext) {
    let begin = {
        let mut peers = context.peers.lock();
        maybe_begin_sim_locked(&mut peers, elapsed_ms(context))
    };
    match begin {
        Ok(begin) => announce_begin_sim(context, begin),
        Err(error) => {
            tracing::error!(%error, "multiplayer ready barrier could not produce a start time");
            fail_server(context, error);
            return;
        }
    }
    // Releasing the barrier puts every admitted peer into the simulation; an
    // expiry instead evicts the peers that never became ready.
    publish_session_seats(context);
}

/// Release newly connected seats into simulation: replay the cached start
/// barrier to them, or try to release the barrier if it has not begun yet.
pub(super) fn finish_seat_connections(context: &ServerContext, seats: &[u8]) {
    let cached_begin = {
        let peers = context.peers.lock();
        if peers.transitions.pending().is_some() {
            return;
        }
        peers.readiness.begun.map(|(frame, start_epoch_ms)| {
            let senders = seats
                .iter()
                .filter_map(|seat| peers.sessions.sender(seat).cloned())
                .collect::<Vec<_>>();
            (frame, start_epoch_ms, senders)
        })
    };
    let Some((frame, start_epoch_ms, senders)) = cached_begin else {
        retry_begin_sim(context);
        return;
    };
    let snapshot_frame = context
        .initial_snapshot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .map(|(frame, _)| *frame);
    let begin_frame = snapshot_frame.map_or(frame, |snapshot_frame| snapshot_frame.max(frame));
    let begin_start_epoch_ms = if begin_frame != frame {
        match try_current_epoch_ms()
            .map_err(MultiplayerError::from)
            .and_then(|now| {
                now.checked_add(100).ok_or_else(|| {
                    MultiplayerError::LocalState(
                        "multiplayer seat connection timestamp exceeds the u64 Unix range".into(),
                    )
                })
            }) {
            Ok(start_epoch_ms) => start_epoch_ms,
            Err(error) => {
                tracing::error!(%error, "seat connection could not produce a start time");
                fail_server(context, error);
                return;
            }
        }
    } else {
        start_epoch_ms
    };
    for sender in senders {
        server_dispatch::queue_cached_begin(&sender, begin_frame, begin_start_epoch_ms);
    }
}

pub(super) fn connect_all_provisional_seats(context: &ServerContext) {
    let seats = {
        let mut peers = context.peers.lock();
        // Authentication can finish while the host is still loading a save.
        // Seat commands must use the restored timeline, never the default cursor.
        if peers.readiness.host_frame.is_none() {
            return;
        }
        peers.sessions.admit_provisional_sessions()
    };
    let connected_seats = seats.iter().map(|(seat, _)| *seat).collect::<Vec<_>>();
    publish_connect_seats(context, seats);
    finish_seat_connections(context, &connected_seats);
    // A seat admitted after the mission began is in its simulation as soon as
    // the cached `BeginSim` is queued for it; a seat admitted before it is not,
    // and `publish_session_seats` keeps it out until the barrier releases.
    publish_session_seats(context);
}

/// Poll every bounded barrier so a peer that stops answering cannot freeze
/// the session.
///
/// Snapshot transitions expire into eviction plus a commit for the peers that
/// remain; the ready barrier expires into eviction plus a start for the peers
/// that loaded. Both are no-ops on a tick where nothing is pending, so one
/// ticker serves both. Eviction is safe because an evicted seat keeps its
/// authenticated owner and re-admits from an authoritative snapshot.
async fn run_barrier_watchdog(context: Arc<ServerContext>) {
    let mut ticker = tokio::time::interval(Duration::from_millis(500));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        if context.cancellation.load(Ordering::Acquire) {
            return;
        }
        let (committed, begin) = {
            let mut peers = context.peers.lock();
            let now_ms = elapsed_ms(&context);
            if let Some(expired) = peers.transitions.expire(now_ms) {
                tracing::warn!(
                    transition = ?expired.id,
                    evicted = ?expired.evicted,
                    waited_ms = expired.waited_ms,
                    payload_bytes = expired.payload_bytes,
                    "multiplayer: evicted unresponsive peers from snapshot transition"
                );
            }
            let committed = take_committed_snapshot_transition(&mut peers);
            // The replacement mission resets readiness; evaluating it in the
            // same tick would start the old state for the new transport.
            let begin = if committed.is_some() {
                Ok(None)
            } else {
                maybe_begin_sim_locked(&mut peers, now_ms)
            };
            (committed, begin)
        };
        if committed.is_some() {
            commit_snapshot_transition(&context, committed);
        }
        match begin {
            Ok(begin) => announce_begin_sim(&context, begin),
            Err(error) => {
                tracing::error!(%error, "multiplayer ready barrier could not produce a start time");
                fail_server(&context, error);
                return;
            }
        }
        // Expiry evicts the seats that never became ready; the commit above
        // ends the generation, so the replacement mission starts with its host
        // alone. Both are changes to the simulation's seat set.
        publish_session_seats(&context);
    }
}

pub(super) async fn handle_incoming_peer(
    context: Arc<ServerContext>,
    incoming: iroh::endpoint::Incoming,
) -> Result<(), MultiplayerError> {
    let conn = tokio::time::timeout(HANDSHAKE_FRAME_TIMEOUT, incoming)
        .await
        .map_err(|_| MultiplayerError::Handshake("peer QUIC handshake timed out".into()))?
        .map_err(|e| MultiplayerError::transport("peer connecting", e))?;
    let remote_id = conn.remote_id();
    let peer_id = remote_id.to_string();
    tracing::info!(peer = %peer_id, "incoming connection");

    let (mut send, mut recv) = tokio::time::timeout(HANDSHAKE_FRAME_TIMEOUT, conn.accept_bi())
        .await
        .map_err(|_| {
            MultiplayerError::Handshake(
                "peer did not open a game stream before handshake timeout".into(),
            )
        })?
        .map_err(|e| MultiplayerError::transport("accept peer stream", e))?;

    // Receive Hello.  Reject anything else.
    let (nickname, browser_auth) = match read_frame_bounded_with_timeout(
        &mut recv,
        InboundFramePolicy::ClientHello,
        HANDSHAKE_FRAME_TIMEOUT,
        "client Hello",
    )
    .await?
    {
        Some(NetMsg::Hello {
            protocol_version,
            nickname,
            browser_auth,
        }) => {
            if protocol_version != NET_PROTOCOL_VERSION {
                let reason = format!(
                    "protocol mismatch (peer={protocol_version}, server={NET_PROTOCOL_VERSION})"
                );
                reject_opening(&mut send, &reason).await;
                return Err(MultiplayerError::Handshake(reason.into()));
            }
            (nickname, browser_auth)
        }
        Some(other) => {
            let reason = format!("expected Hello, got {other:?}");
            reject_opening(&mut send, &reason).await;
            return Err(MultiplayerError::Handshake(reason.into()));
        }
        None => {
            return Err(MultiplayerError::Handshake(
                "connection closed before Hello".into(),
            ));
        }
    };

    let owner = match authenticate_peer(&context, remote_id, browser_auth.as_ref()) {
        Ok(owner) => owner,
        Err(error) => {
            // The wire rejection carries the reason as text.
            reject_opening(&mut send, &error.to_string()).await;
            return Err(error);
        }
    };
    if let Some(content) = &context.content {
        let joining = admit_distributed_mod(
            &mut send,
            &mut recv,
            content,
            context.host_endpoint_id.to_string(),
        )
        .await?;
        if !joining {
            conn.close(CLOSE_GRACEFUL.into(), b"content prepared");
            return Ok(());
        }
    }

    let (seat_claim, mut write_rx) = match prepare_peer_session(&context, owner, &nickname) {
        Ok(prepared) => prepared,
        Err(error) => {
            reject_opening(&mut send, &error.to_string()).await;
            return Err(error);
        }
    };
    let session_generation = seat_claim.generation;
    let assigned_seat = PlayerId(seat_claim.seat);
    context
        .connections
        .lock()
        .insert(assigned_seat.0, conn.clone());

    // Writer half: drain the peer's queue onto the stream.  Reader
    // half: every Input received gets stamped with the peer's
    // assigned seat (defensive — the client tags its own outgoing
    // too, but we don't trust the wire) and a target frame derived
    // from the server's current sim frame at receive time, before
    // broadcasting.  Both halves run in this task via `select!` so
    // either side ending tears the peer down.
    let result = {
        let writer = async {
            while let Some(msg) = write_rx.recv().await {
                let is_commit = matches!(&msg, NetMsg::CommitSnapshotTransition { .. });
                write_frame(&mut send, &msg).await?;
                if is_commit {
                    notify_commit_frame_written(&context, assigned_seat.0);
                }
            }
            // Queue closed: seat was dropped (shutdown or cleanup).
            Ok::<(), MultiplayerError>(())
        };
        let reader = run_server_peer_reader(&context, assigned_seat, session_generation, &mut recv);
        drive_server_peer_io(reader, writer, TERMINAL_WRITER_DRAIN_TIMEOUT).await
    };

    // A writer that ended without a commit frame still releases the host's
    // bounded drain: nothing more can be delivered on this connection.
    notify_commit_frame_written(&context, assigned_seat.0);
    release_peer_session(&context, assigned_seat, owner, session_generation);
    conn.close(CLOSE_GRACEFUL.into(), b"session over");

    result
}

/// Claim, opening queue publication and admission are one authority operation.
/// All network I/O happens later in the peer task's writer.
pub(super) fn prepare_peer_session(
    context: &ServerContext,
    owner: PeerOwner,
    nickname: &str,
) -> Result<(SeatClaim, UnboundedReceiver<NetMsg>), MultiplayerError> {
    // A connection that was in flight when shutdown retired this generation
    // must not claim a seat on a mission whose accept loop is already closed
    // to new admissions. Live generations are unaffected (H17 reconnect).
    if context.cancellation.load(Ordering::Acquire) {
        return Err(MultiplayerError::LocalState(
            "mission transport is shutting down".into(),
        ));
    }
    let _authority = context.session_dispatch.lock();
    // Claim/reclaim a seat by authenticated owner, never by editable nickname.
    let seat_claim = {
        let mut p = context.peers.lock();
        let returning_seat = p.sessions.owner_seat(owner);
        if let Some(transition) = p.transitions.pending()
            && !returning_seat.is_some_and(|seat| transition.awaiting.contains(&seat))
        {
            return Err(MultiplayerError::Handshake(
                "host is changing missions; only pending participants may reconnect".into(),
            ));
        }
        let (write_tx, write_rx) = unbounded_channel::<NetMsg>();
        p.sessions
            .claim_seat(owner, nickname, write_tx)
            .map(|claim| (claim, write_rx))
    };
    let (seat_claim, write_rx) = seat_claim?;
    let assigned_seat_u8 = seat_claim.seat;
    let session_generation = seat_claim.generation;
    let assigned_seat = PlayerId(assigned_seat_u8);
    tracing::debug!(
        seat = assigned_seat_u8,
        generation = session_generation,
        kind = ?seat_claim.kind,
        "multiplayer peer claimed seat"
    );

    // Queue Welcome for this peer.  Goes through the writer queue so
    // the writer task is the only thing that touches the outbound
    // half of the stream.  If the host has cached an initial-state
    // snapshot we follow up with that — mid-mission joiners adopt it
    // instead of trying to reproduce engine init from seed alone.
    let writer_closed = |message: &'static str| MultiplayerError::ChannelClosed(message.into());
    let opening_result = (|| -> Result<(), MultiplayerError> {
        let p = context.peers.lock();
        {
            let sender = p.sessions.sender(&assigned_seat_u8).ok_or_else(|| {
                MultiplayerError::LocalState(
                    format!("claimed peer {assigned_seat:?} has no writer for Welcome").into(),
                )
            })?;
            sender
                .send(NetMsg::Welcome {
                    your_seat: assigned_seat,
                    session_id: context.session_id,
                    mission_id: context.mission_id.clone(),
                    mission_seed: context.mission_seed,
                    sim_config: context.sim_config,
                    speech_timing_locale: context.speech_timing_locale.clone(),
                    host_nickname: context.host_nickname.clone(),
                })
                .map_err(|_| writer_closed("writer queue closed before Welcome"))?;
            // `InitialSnapshot` is a plain std mutex shared with the
            // game loop; the snapshot value is only ever replaced
            // wholesale, so recover it if a prior holder panicked
            // instead of silently skipping the snapshot send.
            let snapshot_frame = if let Some((frame, bytes)) = context
                .initial_snapshot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
            {
                tracing::info!(
                    seat = assigned_seat_u8,
                    frame,
                    bytes = bytes.len(),
                    "sending initial snapshot to peer"
                );
                sender
                    .send(NetMsg::InitialSnapshot {
                        frame,
                        engine_bytes: bytes,
                    })
                    .map_err(|_| writer_closed("writer queue closed before InitialSnapshot"))?;
                Some(frame)
            } else {
                None
            };
            // Session control is durable independently of simulation ticks.
            // In particular, a scroll may open or close while this engine
            // snapshot remains frozen at the same frame.
            for message in p.modals.messages() {
                sender
                    .send(message)
                    .map_err(|_| writer_closed("writer queue closed before modal recovery"))?;
            }
            if let Some(transition) = p.transitions.pending() {
                sender
                    .send(NetMsg::PrepareSnapshotTransition {
                        id: transition.id,
                        payload: transition.payload.clone(),
                    })
                    .map_err(|_| writer_closed("writer queue closed before transition Prepare"))?;
            }
            // TODO(mp-duplicate-cached-begin): a reconnecting (non-sim-connected)
            // seat also receives the cached BeginSim from
            // `connect_all_provisional_seats` below, so it sees it twice. Clients
            // tolerate the repeat; consider sending it from one place only.
            if p.transitions.pending().is_none()
                && let Some((frame, start_epoch_ms)) = p.readiness.begun
            {
                let begin_frame =
                    snapshot_frame.map_or(frame, |snapshot_frame| snapshot_frame.max(frame));
                let begin_start_epoch_ms = if begin_frame != frame {
                    try_current_epoch_ms()?.checked_add(100).ok_or_else(|| {
                        MultiplayerError::LocalState(
                            "multiplayer reconnect timestamp exceeds the u64 Unix range".into(),
                        )
                    })?
                } else {
                    start_epoch_ms
                };
                sender
                    .send(NetMsg::BeginSim {
                        frame: begin_frame,
                        start_epoch_ms: begin_start_epoch_ms,
                    })
                    .map_err(|_| writer_closed("writer queue closed before cached BeginSim"))?;
            }
        }
        Ok(())
    })();
    if let Err(error) = opening_result {
        let mut p = context.peers.lock();
        p.sessions
            .release_seat_if_owner(assigned_seat_u8, owner, session_generation);
        return Err(error);
    }

    // The stream is gameplay-compatible after Welcome: connect the new seat
    // into the deterministic simulation right away.
    connect_all_provisional_seats(context);
    Ok((seat_claim, write_rx))
}

pub(super) fn release_peer_session(
    context: &ServerContext,
    assigned_seat: PlayerId,
    owner: PeerOwner,
    session_generation: u64,
) {
    let _authority = context.session_dispatch.lock();
    let assigned_seat_u8 = assigned_seat.0;
    // On disconnect, park the authenticated owner identity so a future
    // reconnect reclaims the same deterministic seat. Nicknames are labels.
    let release = {
        let mut p = context.peers.lock();
        let release = p
            .sessions
            .release_seat_if_owner(assigned_seat_u8, owner, session_generation);
        if release.is_some() {
            retain_transition_peer_for_reconnect(&mut p, assigned_seat_u8);
        }
        release
    };
    // An obsolete reader's teardown must not affect a successor.
    let Some(was_sim_connected) = release else {
        return;
    };
    // The seat left this session's simulation: no shared page may await it.
    publish_session_seats(context);
    if was_sim_connected && !context.cancellation.load(Ordering::Acquire) {
        let now = context.frame_cursor.load(Ordering::Relaxed);
        let target = now.saturating_add(INPUT_DELAY_FRAMES);
        let inp = PlayerInput::new(
            PlayerId::HOST,
            PlayerCommand::DisconnectSeat {
                player_id: assigned_seat,
            },
        );
        broadcast_input(context, now, now, target, inp);
    }
}

pub(super) async fn admit_distributed_mod(
    send: &mut SendStream,
    recv: &mut RecvStream,
    content: &HostedModContent,
    host_endpoint_id: String,
) -> Result<bool, MultiplayerError> {
    let offer = content.offer(host_endpoint_id)?;
    write_frame_with_timeout(
        send,
        &NetMsg::ContentOffer {
            offer: offer.clone(),
        },
        CONTENT_TRANSFER_IDLE_TIMEOUT,
        "content offer",
    )
    .await?;
    let resume_offset = match read_frame_bounded_with_timeout(
        recv,
        InboundFramePolicy::ClientToServer,
        CONTENT_DECISION_TIMEOUT,
        "content decision",
    )
    .await?
    {
        Some(NetMsg::ContentRequest(robin_engine::multiplayer::ContentRequest {
            full_mod_sha256,
            resume_offset,
        })) if full_mod_sha256 == offer.full_mod_sha256 => resume_offset,
        Some(NetMsg::ContentRequest(robin_engine::multiplayer::ContentRequest {
            full_mod_sha256,
            ..
        })) => {
            return Err(MultiplayerError::ContentMismatch(
                format!(
                    "client requested distributed mod {}, offered {}",
                    robin_engine::spellforge::hex_hash(&full_mod_sha256),
                    robin_engine::spellforge::hex_hash(&offer.full_mod_sha256)
                )
                .into(),
            ));
        }
        Some(NetMsg::ContentReject(robin_engine::multiplayer::ContentReject {
            full_mod_sha256,
            reason,
        })) if full_mod_sha256 == offer.full_mod_sha256 => {
            return Err(MultiplayerError::ContentDeclined(
                format!("client declined exact host content: {reason}").into(),
            ));
        }
        Some(other) => {
            return Err(MultiplayerError::RemoteProtocol(
                format!("expected ContentRequest, got {other:?}").into(),
            ));
        }
        None => {
            return Err(MultiplayerError::Handshake(
                "connection closed before content decision".into(),
            ));
        }
    };
    if resume_offset > content.encoded.len() as u64 {
        return Err(MultiplayerError::ContentMismatch(
            format!(
                "client resume offset {resume_offset} exceeds content length {}",
                content.encoded.len()
            )
            .into(),
        ));
    }
    let mut offset = resume_offset as usize;
    let transfer_deadline = tokio::time::Instant::now() + CONTENT_DECISION_TIMEOUT;
    while offset < content.encoded.len() {
        let end = (offset + robin_engine::multiplayer::DISTRIBUTED_MOD_CHUNK_LIMIT)
            .min(content.encoded.len());
        let message = NetMsg::ContentChunk(robin_engine::multiplayer::ContentChunk {
            full_mod_sha256: offer.full_mod_sha256,
            offset: offset as u64,
            total_bytes: content.encoded.len() as u64,
            bytes: content.encoded[offset..end].to_vec(),
        });
        tokio::select! {
            result = write_frame_with_timeout(
                send,
                &message,
                CONTENT_TRANSFER_IDLE_TIMEOUT,
                "content chunk",
            ) => result?,
            _ = tokio::time::sleep_until(transfer_deadline) => {
                return Err(MultiplayerError::DeadlineExceeded {
                    phase: "content transfer".into(),
                    limit: CONTENT_DECISION_TIMEOUT,
                });
            }
        }
        offset = end;
    }
    match read_frame_bounded_with_timeout(
        recv,
        InboundFramePolicy::ClientToServer,
        CONTENT_READINESS_TIMEOUT,
        "content readiness",
    )
    .await?
    {
        Some(NetMsg::ContentReady { full_mod_sha256 })
            if full_mod_sha256 == offer.full_mod_sha256 =>
        {
            Ok(true)
        }
        Some(NetMsg::ContentPrepared { full_mod_sha256 })
            if full_mod_sha256 == offer.full_mod_sha256 =>
        {
            Ok(false)
        }
        Some(NetMsg::ContentReject(robin_engine::multiplayer::ContentReject {
            full_mod_sha256,
            reason,
        })) if full_mod_sha256 == offer.full_mod_sha256 => Err(MultiplayerError::ContentDeclined(
            format!("client rejected downloaded host content: {reason}").into(),
        )),
        Some(other) => Err(MultiplayerError::RemoteProtocol(
            format!("expected ContentReady/ContentPrepared, got {other:?}").into(),
        )),
        None => Err(MultiplayerError::Handshake(
            "connection closed before content readiness".into(),
        )),
    }
}

pub(super) async fn reject_opening(send: &mut SendStream, reason: &str) {
    let reason = robin_engine::multiplayer::bounded_safe_diagnostic(
        reason,
        robin_engine::multiplayer::MAX_REJECT_REASON_BYTES,
    );
    if let Err(error) = write_frame(send, &NetMsg::Reject { reason }).await {
        tracing::debug!(%error, "failed to send multiplayer opening rejection");
    }
}

pub(super) fn authenticate_peer(
    context: &ServerContext,
    remote_id: EndpointId,
    browser_auth: Option<&BrowserPeerAuth>,
) -> Result<PeerOwner, MultiplayerError> {
    let Some(auth) = browser_auth else {
        return Ok(PeerOwner::Native(*remote_id.as_bytes()));
    };
    let ticket =
        crate::multiplayer::join_ticket::BrowserJoinTicket::decode_authenticated(&auth.join_code)?;
    let payload = ticket.payload();
    if payload.host_endpoint_id != context.host_endpoint_id.to_string()
        || ticket.session_id()? != context.session_id.0
        || payload.expected_players != context.peers.lock().sessions.expected_players()
    {
        return Err(MultiplayerError::Invitation(
            "browser invitation does not belong to this exact hosted mission session".into(),
        ));
    }
    if payload.mission_id != context.mission_id && !context.continued_session {
        return Err(MultiplayerError::Invitation(
            "browser invitation belongs to another hosted mission".into(),
        ));
    }
    let owner = PeerOwner::Browser(auth.durable_public_key);
    let use_kind = if context.peers.lock().sessions.owner_seat(owner).is_some() {
        crate::multiplayer::join_ticket::InvitationUse::RedeemedReconnect
    } else {
        crate::multiplayer::join_ticket::InvitationUse::Initial
    };
    ticket.validate_use_at(try_current_epoch_ms()? / 1000, use_kind)?;
    let public_key = iroh::PublicKey::from_bytes(&auth.durable_public_key).map_err(|error| {
        MultiplayerError::invalid_address("invalid durable browser public key", error)
    })?;
    let signature_bytes: [u8; iroh::Signature::LENGTH] =
        auth.signature.as_slice().try_into().map_err(|_| {
            MultiplayerError::Identity("browser seat proof signature must be 64 bytes".into())
        })?;
    let signature = iroh::Signature::from_bytes(&signature_bytes);
    let message = browser_seat_proof_message(
        context.session_id.0,
        *context.host_endpoint_id.as_bytes(),
        *remote_id.as_bytes(),
    );
    public_key.verify(&message, &signature).map_err(|_| {
        MultiplayerError::Identity(
            "browser seat proof does not bind this session and transport".into(),
        )
    })?;
    Ok(owner)
}

/// Keep the writer future pinned when the reader loses authority. Dropping and
/// restarting write_frame could duplicate a partially written frame header.
pub(super) async fn drive_server_peer_io(
    reader: impl std::future::Future<Output = Result<PeerReaderExit, MultiplayerError>>,
    writer: impl std::future::Future<Output = Result<(), MultiplayerError>>,
    drain_timeout: Duration,
) -> Result<(), MultiplayerError> {
    tokio::pin!(writer);
    tokio::select! {
        result = reader => match result? {
            PeerReaderExit::Closed => Ok(()),
            PeerReaderExit::Inactive => {
                tokio::time::timeout(drain_timeout, writer.as_mut())
                    .await
                    .map_err(|_| MultiplayerError::LocalState(
                        "inactive peer writer timed out draining terminal frames".into(),
                    ))?
                    .map_err(|error| error.context("peer writer"))
            }
        },
        result = writer.as_mut() => result.map_err(|error| error.context("peer writer")),
    }
}

pub(super) async fn run_server_peer_reader(
    context: &ServerContext,
    seat: PlayerId,
    session_generation: u64,
    recv: &mut RecvStream,
) -> Result<PeerReaderExit, MultiplayerError> {
    loop {
        let message = match read_frame(recv, InboundFramePolicy::ClientToServer).await {
            Ok(Some(message)) => message,
            ended => {
                // A half-close/read error must not cancel a terminal write
                // already queued by the host while this read was suspended.
                let _authority = context.session_dispatch.lock();
                if let Some(exit) = peer_reader_dispatch_result(
                    context
                        .peers
                        .lock()
                        .sessions
                        .authorize_session(seat, session_generation),
                )? {
                    return Ok(exit);
                }
                return ended.map(|_| PeerReaderExit::Closed);
            }
        };
        if let Some(exit) = peer_reader_dispatch_result(dispatch_server_peer_message(
            context,
            seat,
            session_generation,
            message,
        ))? {
            return Ok(exit);
        }
    }
}

/// Execute one decoded peer event without yielding. The authority gate remains
/// held through validation, state transitions, and local/writer queue effects;
/// a superseding claim cannot slip between a generation check and its effect.
pub(super) fn dispatch_server_peer_message(
    context: &ServerContext,
    seat: PlayerId,
    session_generation: u64,
    message: NetMsg,
) -> Result<(), PeerDispatchFailure> {
    let _authority = context.session_dispatch.lock();
    context
        .peers
        .lock()
        .sessions
        .authorize_session(seat, session_generation)?;
    apply_authenticated_peer_message(context, seat, session_generation, message)
        .map_err(PeerDispatchFailure::Protocol)
}

/// Called only while dispatch_server_peer_message retains the authority gate.
pub(super) fn apply_authenticated_peer_message(
    context: &ServerContext,
    seat: PlayerId,
    session_generation: u64,
    message: NetMsg,
) -> Result<(), MultiplayerError> {
    let remote = |message: String| MultiplayerError::RemoteProtocol(message.into());
    let closed = |message: &'static str| MultiplayerError::ChannelClosed(message.into());
    validate_server_gameplay_wire_msg(&message)?;
    match message {
        NetMsg::Latency {
            to, nonce, reply, ..
        } => super::server_dispatch::route_latency(context, seat, to, nonce, reply)?,
        NetMsg::ChatSend { text } => {
            let nickname = context.peers.lock().sessions.nickname(seat).to_owned();
            super::server_dispatch::broadcast_chat(context, nickname, text)?;
        }
        NetMsg::Cursor { state, .. } => {
            // The authenticated seat wins: a client cannot claim another seat.
            super::server_dispatch::broadcast_teammate_cursor(context, seat, state)?;
        }
        NetMsg::Input {
            origin_frame,
            command,
        } => {
            validate_peer_command_authority(seat, &command)?;
            let ready = {
                let peers = context.peers.lock();
                peers.readiness.host_frame.is_some() && peers.sessions.ready_frame(seat.0).is_some()
            };
            if !ready {
                tracing::debug!(
                    ?seat,
                    "ignoring gameplay input before host and peer snapshot readiness"
                );
                return Ok(());
            }
            let now = context.frame_cursor.load(Ordering::Relaxed);
            let target = now.max(origin_frame).saturating_add(INPUT_DELAY_FRAMES);
            let inp = PlayerInput::new(seat, command);
            broadcast_input(context, now, origin_frame, target, inp);
        }
        NetMsg::Note(s) => {
            tracing::info!(?seat, note = %s, "peer note");
        }
        NetMsg::ModalProposal(proposal) => {
            if proposal.instance.session_id != context.session_id {
                return Err(remote(format!(
                    "peer {seat:?} submitted a modal proposal for another session"
                )));
            }
            context
                .incoming_tx
                .send(NetEvent::ModalProposal {
                    from: seat,
                    proposal,
                })
                .map_err(|_| closed("host modal proposal channel is closed"))?;
        }
        NetMsg::ModalDecision { .. } => {
            return Err(remote(format!(
                "peer {seat:?} attempted an authoritative modal decision"
            )));
        }
        NetMsg::ReadyToSim { frame } => {
            let begin = {
                let mut p = context.peers.lock();
                p.sessions.record_ready(seat.0, session_generation, frame)?;
                maybe_begin_sim_locked(&mut p, elapsed_ms(context))
            };
            let begin = match begin {
                Ok(begin) => begin,
                Err(error) => {
                    fail_server(context, error.clone());
                    return Err(error);
                }
            };
            announce_begin_sim(context, begin);
            // `maybe_begin_sim_locked` either releases the barrier or evicts
            // the seats that never became ready.
            publish_session_seats(context);
        }
        NetMsg::SnapshotTransitionReady { id } => {
            if id.session_id != context.session_id {
                return Err(remote(format!(
                    "peer {seat:?} acknowledged a snapshot transition for another session"
                )));
            }
            let committed = {
                let mut peers = context.peers.lock();
                peers.transitions.acknowledge(seat, id)?;
                take_committed_snapshot_transition(&mut peers)
            };
            commit_snapshot_transition(context, committed);
        }
        NetMsg::PrepareSnapshotTransition { .. } | NetMsg::CommitSnapshotTransition { .. } => {
            return Err(remote(format!(
                "peer {seat:?} attempted a host-only snapshot transition message"
            )));
        }
        _ => unreachable!("server gameplay message was validated before dispatch"),
    }
    Ok(())
}

pub(super) fn validate_server_gameplay_wire_msg(message: &NetMsg) -> Result<(), MultiplayerError> {
    match message {
        NetMsg::Latency { .. }
        | NetMsg::ChatSend { .. }
        | NetMsg::Cursor { .. }
        | NetMsg::Input { .. }
        | NetMsg::Note(_)
        | NetMsg::ModalProposal { .. }
        | NetMsg::ReadyToSim { .. }
        | NetMsg::SnapshotTransitionReady { .. } => Ok(()),
        NetMsg::ContentRequest { .. }
        | NetMsg::ContentReject { .. }
        | NetMsg::ContentReady { .. }
        | NetMsg::ContentPrepared { .. } => Err(MultiplayerError::RemoteProtocol(
            "content-admission message arrived in an ordinary peer session".into(),
        )),
        other => Err(MultiplayerError::RemoteProtocol(
            format!("client sent invalid server-session message {other:?}").into(),
        )),
    }
}

pub(super) fn validate_peer_command_authority(
    seat: PlayerId,
    command: &robin_engine::player_command::PlayerCommand,
) -> Result<(), MultiplayerError> {
    if command.requires_host_authority() {
        return Err(MultiplayerError::RemoteProtocol(
            format!("peer {seat:?} attempted host-authoritative command {command:?}").into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
