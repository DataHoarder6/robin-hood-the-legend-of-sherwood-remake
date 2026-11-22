//! The one multiplayer client session state machine, shared by the native
//! (iroh on tokio) and browser (iroh relay on wasm) clients.
//!
//! Handshake -> content admission -> Welcome -> session -> reconnect is
//! implemented once here, generic over [`ClientTransport`]. Each platform
//! supplies only what genuinely differs:
//!
//! - [`ClientTimer`]: how to sleep (tokio timers vs browser timers).
//! - [`ClientTransport`]: endpoint and `Hello` contents, how local outbound
//!   commands are received (tokio bridge vs polled `std::sync::mpsc`), how
//!   startup progress/failure is reported to the caller (blocking handshake
//!   channel vs handle slots), and timings.
//!
//! Session rules decided here, once, for both platforms:
//!
//! - A clean stream close is a transport drop and reconnects ([`reader_outcome`]).
//!   A committed snapshot transition instead retires the old mission transport.
//! - A host `ReconnectRequired` directive or a failed local publication drops
//!   the session and reconnects; a trust violation (`Reject`, wrong-direction
//!   message) is fatal.

use super::client_gameplay::{deliver, deliver_lifecycle};
use super::client_protocol::{
    ClientHandshake, ClientSessionMetadata, HandshakeAction, ReconnectIdentity, WelcomeData,
    validate_reconnect_content, validate_reconnect_state,
};
use super::content_transfer::{ContentDecision, accept_chunk};
use super::framing::{read_frame, write_frame};
use super::identity::GAME_ALPN;
use super::{InboundFramePolicy, MultiplayerError, NetEvent, NetMsg, NetOutbound};
use futures::future::{Either, select};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr};
use robin_engine::multiplayer::{DistributedModOffer, MultiplayerSessionId, NetFatal};
use robin_engine::player_command::PlayerId;
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// An intentional host exit must not trigger the reconnect loop.
pub(super) const HOST_LEFT_CLOSE_CODE: u32 = 0x5248;

/// Cancellation is a flag set by [`ClientHandle::shutdown`]; waiters poll it.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(20);
const INITIAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const INITIAL_BACKOFF: Duration = Duration::from_millis(50);
const MAX_INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const RECONNECT_BACKOFF: Duration = Duration::from_millis(500);
const MAX_RECONNECT_BACKOFF: Duration = Duration::from_secs(10);
/// Total time a dropped client may keep retrying before the session is
/// declared gone. Without a cap the client retries forever and the player sits
/// on a frozen frame instead of returning to the menu.
pub(super) const RECONNECT_BUDGET: Duration = Duration::from_secs(60);

// ─── Handle ──────────────────────────────────────────────────────

/// State published by the client worker and read by the game through its
/// [`ClientHandle`]. Cloned into the worker.
///
/// Not serde: live shared slots.
#[derive(Clone)]
pub(super) struct ClientSlots {
    pub(super) session_metadata: Arc<Mutex<Option<ClientSessionMetadata>>>,
    /// Present when the host requires content admission before Welcome. The
    /// game/menu must explicitly trust, download, validate, mount, and answer
    /// this exact offer; the transport never silently approves it.
    pub(super) content_offer: Arc<Mutex<Option<DistributedModOffer>>>,
    pub(super) startup_error: Arc<Mutex<Option<MultiplayerError>>>,
    pub(super) cancellation: Arc<AtomicBool>,
}

impl ClientSlots {
    pub(super) fn new() -> Self {
        Self {
            session_metadata: Arc::new(Mutex::new(None)),
            content_offer: Arc::new(Mutex::new(None)),
            startup_error: Arc::new(Mutex::new(None)),
            cancellation: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) fn set_startup_error(&self, error: MultiplayerError) {
        *slot(&self.startup_error) = Some(error);
    }
}

/// Lock a published slot. A panicking holder cannot leave a half-written
/// value (every write is a single assignment), so poisoning is ignored like
/// the native transport's non-poisoning locks.
fn slot<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Joins the platform worker (the native I/O thread) once cancellation is set.
pub(super) type ClientWorker = Box<dyn FnOnce() + Send + Sync>;

/// Handle to an active client connection.
pub struct ClientHandle {
    slots: ClientSlots,
    worker: Option<ClientWorker>,
}

impl ClientHandle {
    pub(super) fn new(slots: ClientSlots, worker: Option<ClientWorker>) -> Self {
        Self { slots, worker }
    }

    pub fn session_metadata(&self) -> Option<ClientSessionMetadata> {
        slot(&self.slots.session_metadata).clone()
    }

    pub fn content_offer(&self) -> Option<DistributedModOffer> {
        slot(&self.slots.content_offer).clone()
    }

    /// Error that ended the connection before an authoritative Welcome.
    pub fn startup_error(&self) -> Option<MultiplayerError> {
        slot(&self.slots.startup_error).clone()
    }

    pub fn shutdown(&mut self) {
        self.slots.cancellation.store(true, Ordering::Release);
        if let Some(join_worker) = self.worker.take() {
            join_worker();
        }
    }
}

impl Drop for ClientHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ─── Platform seams ──────────────────────────────────────────────

pub(super) trait ClientTimer {
    fn sleep(duration: Duration) -> impl Future<Output = ()>;
}

/// Deadlines of one platform. `None` means the phase is unbounded apart from
/// cancellation — never the whole of an attempt, because the reconnect budget
/// is only read between attempts: an attempt without a deadline makes the
/// budget unreachable and the loop permanent.
pub(super) struct ClientTimings {
    /// Each Hello write and Welcome/offer read.
    pub(super) handshake_frame: Option<Duration>,
    /// One whole initial connect + Hello + Welcome attempt.
    pub(super) initial_attempt: Duration,
    /// One whole reconnect attempt: the handshake and the content prelude it
    /// opened, capped further by whatever is left of [`Self::reconnect_budget`].
    pub(super) reconnect_attempt: Duration,
    /// How long repeated reconnect attempts may continue after a drop.
    pub(super) reconnect_budget: Duration,
    pub(super) post_content_welcome: Duration,
    pub(super) content_write: Option<Duration>,
    pub(super) content_chunk_idle: Duration,
    pub(super) content_transfer: Duration,
    pub(super) content_decision: Option<Duration>,
    pub(super) content_readiness: Option<Duration>,
}

/// Why a client session ended.
#[derive(Debug)]
pub(super) enum SessionEnd {
    /// The next mission owns a new transport. Keep ingress alive until the
    /// game consumes the commit and drops this transport.
    TransitionCommitted,
    /// Network error, unexpected drop or clean stream close — caller retries.
    /// A clean close is not an authoritative end of the session: the host ends
    /// sessions with `Reject`, `ReconnectRequired`, or a snapshot commit,
    /// so a bare FIN is treated
    /// like any other transport loss (see [`reader_outcome`]).
    Drop(MultiplayerError),
    /// A direction or session invariant failed. Retrying the same
    /// authenticated session cannot repair this violation.
    Fatal(MultiplayerError),
    /// The game loop dropped the outgoing channel or shutdown began — stop
    /// the I/O task entirely (no retry).
    OutgoingClosed,
}

/// Progress of the first handshake, reported to a caller that blocks on it.
#[derive(Debug)]
pub(super) enum InitialHandshake {
    Welcomed { seat: PlayerId, mission_seed: u64 },
    ContentOffered { full_mod_sha256: [u8; 32] },
}

/// Phase in which the connection failed before gameplay started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StartupFailure {
    /// No handshake completed.
    Connect,
    /// Host content admission failed after the offer was published.
    Admission,
    /// The Welcome could not be admitted.
    Welcome,
}

pub(super) trait ClientTransport {
    type Timer: ClientTimer;
    /// Receiver of the game loop's outbound commands.
    type Outbound;

    const TIMINGS: ClientTimings;
    /// Startup error when shutdown begins before the first handshake.
    const CANCELLED: &'static str;

    fn cancellation(&self) -> &AtomicBool;
    fn endpoint(&self) -> &Endpoint;
    fn server_addr(&self) -> &EndpointAddr;
    fn hello(&self) -> NetMsg;
    /// Session the host Welcome must belong to (signed browser invitation).
    fn expected_session(&self) -> Option<MultiplayerSessionId>;

    /// Next local outbound command; `None` once the game loop dropped its
    /// sender.
    async fn recv_outbound(outbound: &mut Self::Outbound) -> Option<NetOutbound>;
    /// Throw away every queued outbound command; returns how many.
    fn discard_outbound(outbound: &mut Self::Outbound) -> usize;

    /// Wrap the last failure once the initial connect deadline has passed.
    fn initial_handshake_exhausted(last_error: MultiplayerError) -> MultiplayerError;

    fn publish_initial_handshake(&self, progress: InitialHandshake) -> Result<(), ()>;
    fn startup_failed(
        &self,
        slots: &ClientSlots,
        incoming: &Sender<NetEvent>,
        failure: StartupFailure,
        error: MultiplayerError,
    );
    /// Platform work between an admitted Welcome and its publication.
    async fn after_welcome(&self) -> Result<(), MultiplayerError>;
}

// ─── Timing helpers ──────────────────────────────────────────────

pub(super) async fn with_timeout<Tm: ClientTimer, F: Future>(
    timeout: Duration,
    future: F,
) -> Result<F::Output, ()> {
    let future = pin!(future);
    let timer = pin!(Tm::sleep(timeout));
    match select(future, timer).await {
        Either::Left((output, _)) => Ok(output),
        Either::Right(((), _)) => Err(()),
    }
}

async fn wait_for_cancel<Tm: ClientTimer>(cancellation: &AtomicBool) {
    while !cancellation.load(Ordering::Acquire) {
        Tm::sleep(CANCEL_POLL_INTERVAL).await;
    }
}

/// `None` when cancellation won the race.
async fn cancellable<Tm: ClientTimer, F: Future>(
    cancellation: &AtomicBool,
    future: F,
) -> Option<F::Output> {
    let future = pin!(future);
    let cancel = pin!(wait_for_cancel::<Tm>(cancellation));
    match select(future, cancel).await {
        Either::Left((output, _)) => Some(output),
        Either::Right(((), _)) => None,
    }
}

/// Sleep for a backoff, returning `true` when shutdown began first.
async fn sleep_or_cancel<Tm: ClientTimer>(cancellation: &AtomicBool, duration: Duration) -> bool {
    cancellable::<Tm, _>(cancellation, Tm::sleep(duration))
        .await
        .is_none()
}

async fn write_frame_within<Tm: ClientTimer>(
    send: &mut SendStream,
    message: &NetMsg,
    timeout: Option<Duration>,
    phase: &'static str,
) -> Result<(), MultiplayerError> {
    match timeout {
        None => write_frame(send, message).await,
        Some(timeout) => with_timeout::<Tm, _>(timeout, write_frame(send, message))
            .await
            .map_err(|()| MultiplayerError::timeout(phase, timeout))?,
    }
}

async fn read_frame_within<Tm: ClientTimer>(
    recv: &mut RecvStream,
    timeout: Option<Duration>,
    phase: &'static str,
) -> Result<Option<NetMsg>, MultiplayerError> {
    let read = read_frame(recv, InboundFramePolicy::ServerToClient);
    match timeout {
        None => read.await,
        Some(timeout) => with_timeout::<Tm, _>(timeout, read)
            .await
            .map_err(|()| MultiplayerError::timeout(phase, timeout))?,
    }
}

// ─── Handshake and content admission ─────────────────────────────

/// A live client session: the connection plus its single bidirectional
/// message stream.
#[derive(Debug)]
pub(super) struct ClientSession {
    // Held so the QUIC connection stays open for the streams' lifetime.
    _conn: Connection,
    send: SendStream,
    recv: RecvStream,
    protocol: ClientHandshake,
}

#[derive(Debug)]
pub(super) enum HandshakePrelude {
    Welcome {
        session: ClientSession,
        welcome: WelcomeData,
    },
    Content {
        session: ClientSession,
        offer: DistributedModOffer,
    },
}

/// One round of (connect → open stream → Hello → Welcome or content offer).
async fn handshake<T: ClientTransport>(
    transport: &T,
) -> Result<HandshakePrelude, MultiplayerError> {
    let server_addr = transport.server_addr();
    let conn = transport
        .endpoint()
        .connect(server_addr.clone(), GAME_ALPN)
        .await
        .map_err(|e| MultiplayerError::transport("connect", e))?;
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|e| MultiplayerError::transport("open stream", e))?;
    write_frame_within::<T::Timer>(
        &mut send,
        &transport.hello(),
        T::TIMINGS.handshake_frame,
        "client Hello",
    )
    .await
    .map_err(|e| e.context("send Hello"))?;
    let message = read_frame_within::<T::Timer>(
        &mut recv,
        T::TIMINGS.handshake_frame,
        "Welcome/content offer",
    )
    .await?;
    let mut protocol =
        ClientHandshake::new(server_addr.id.to_string(), transport.expected_session());
    let action = protocol.receive(message)?;
    let session = ClientSession {
        _conn: conn,
        send,
        recv,
        protocol,
    };
    Ok(match action {
        HandshakeAction::Welcome(welcome) => HandshakePrelude::Welcome { session, welcome },
        HandshakeAction::PrepareContent(offer) => HandshakePrelude::Content { session, offer },
    })
}

/// One bounded, cancellable handshake attempt; `None` when cancelled.
async fn attempt_handshake<T: ClientTransport>(
    transport: &T,
    timeout: Duration,
) -> Option<Result<HandshakePrelude, MultiplayerError>> {
    let attempt = async {
        with_timeout::<T::Timer, _>(timeout, handshake(transport))
            .await
            .unwrap_or_else(|()| Err(MultiplayerError::timeout("multiplayer handshake", timeout)))
    };
    cancellable::<T::Timer, _>(transport.cancellation(), attempt).await
}

async fn initial_handshake<T: ClientTransport>(
    transport: &T,
) -> Result<HandshakePrelude, MultiplayerError> {
    let started = web_time::Instant::now();
    let mut backoff = INITIAL_BACKOFF;
    loop {
        if transport.cancellation().load(Ordering::Acquire) {
            return Err(MultiplayerError::Cancelled(T::CANCELLED.into()));
        }
        let Some(attempt) = attempt_handshake(transport, T::TIMINGS.initial_attempt).await else {
            return Err(MultiplayerError::Cancelled(T::CANCELLED.into()));
        };
        let error = match attempt {
            Ok(prelude) => return Ok(prelude),
            Err(error) => error,
        };
        if started.elapsed() >= INITIAL_CONNECT_TIMEOUT {
            return Err(T::initial_handshake_exhausted(error));
        }
        tracing::debug!("initial multiplayer handshake failed: {error}; retrying");
        if sleep_or_cancel::<T::Timer>(transport.cancellation(), backoff).await {
            return Err(MultiplayerError::Cancelled(T::CANCELLED.into()));
        }
        backoff = (backoff * 2).min(MAX_INITIAL_BACKOFF);
    }
}

async fn read_welcome<T: ClientTransport>(
    mut session: ClientSession,
) -> Result<(ClientSession, WelcomeData), MultiplayerError> {
    session.protocol.content_ready()?;
    let message = read_frame_within::<T::Timer>(
        &mut session.recv,
        Some(T::TIMINGS.post_content_welcome),
        "post-content Welcome",
    )
    .await?;
    match session.protocol.receive(message)? {
        HandshakeAction::Welcome(welcome) => Ok((session, welcome)),
        HandshakeAction::PrepareContent(_) => {
            unreachable!("post-content phase only accepts Welcome")
        }
    }
}

async fn next_admission_outbound<T: ClientTransport>(
    transport: &T,
    outbound: &mut T::Outbound,
    timeout: Option<Duration>,
    phase: &'static str,
) -> Result<NetOutbound, MultiplayerError> {
    let received = cancellable::<T::Timer, _>(transport.cancellation(), async {
        match timeout {
            None => Ok(T::recv_outbound(outbound).await),
            Some(timeout) => with_timeout::<T::Timer, _>(timeout, T::recv_outbound(outbound))
                .await
                .map_err(|()| MultiplayerError::timeout(phase, timeout)),
        }
    })
    .await
    .ok_or_else(|| MultiplayerError::Cancelled(format!("{phase} cancelled").into()))??;
    received
        .ok_or_else(|| MultiplayerError::ChannelClosed(format!("{phase} channel closed").into()))
}

/// Complete first-use admission under game/menu control. The transport
/// validates every wire invariant and does not send `ContentReady` itself;
/// that acknowledgement must come from the consumer after durable staging,
/// full-package hash validation, and deterministic mount preparation.
enum ContentAdmissionCompletion {
    Join(ClientSession, WelcomeData),
    Prepared,
}

async fn complete_content_admission<T: ClientTransport>(
    transport: &T,
    mut session: ClientSession,
    offer: &DistributedModOffer,
    incoming: &Sender<NetEvent>,
    outbound: &mut T::Outbound,
) -> Result<ContentAdmissionCompletion, MultiplayerError> {
    let timings = &T::TIMINGS;
    let decision = next_admission_outbound(
        transport,
        outbound,
        timings.content_decision,
        "content decision",
    )
    .await?;
    let decision = ContentDecision::decode(offer, decision)?;
    write_frame_within::<T::Timer>(
        &mut session.send,
        &decision.message(offer),
        timings.content_write,
        decision.operation(),
    )
    .await?;
    let mut received = decision.resume_offset()?;

    let transfer_started = web_time::Instant::now();
    let transfer_exceeded = || MultiplayerError::DeadlineExceeded {
        phase: "content transfer".into(),
        limit: timings.content_transfer,
    };
    while received < offer.encoded_bytes {
        let remaining = timings
            .content_transfer
            .checked_sub(transfer_started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(transfer_exceeded)?;
        let read = read_frame_within::<T::Timer>(
            &mut session.recv,
            Some(timings.content_chunk_idle),
            "content chunk",
        );
        let message = cancellable::<T::Timer, _>(
            transport.cancellation(),
            with_timeout::<T::Timer, _>(remaining, read),
        )
        .await
        .ok_or_else(|| MultiplayerError::Cancelled("content transfer cancelled".into()))?
        .map_err(|()| transfer_exceeded())??;
        let (end, event) = accept_chunk(offer, received, message)?;
        deliver(incoming, event)?;
        received = end;
    }

    let ready = next_admission_outbound(
        transport,
        outbound,
        timings.content_readiness,
        "content readiness",
    )
    .await?;
    match ready {
        NetOutbound::ContentReady { full_mod_sha256 }
            if full_mod_sha256 == offer.full_mod_sha256 =>
        {
            write_frame_within::<T::Timer>(
                &mut session.send,
                &NetMsg::ContentReady { full_mod_sha256 },
                timings.content_write,
                "content readiness",
            )
            .await?;
            let (session, welcome) = read_welcome::<T>(session).await?;
            Ok(ContentAdmissionCompletion::Join(session, welcome))
        }
        NetOutbound::ContentPrepared { full_mod_sha256 }
            if full_mod_sha256 == offer.full_mod_sha256 =>
        {
            write_frame_within::<T::Timer>(
                &mut session.send,
                &NetMsg::ContentPrepared { full_mod_sha256 },
                timings.content_write,
                "content prepared acknowledgement",
            )
            .await?;
            Ok(ContentAdmissionCompletion::Prepared)
        }
        NetOutbound::ContentReject(reject) if reject.full_mod_sha256 == offer.full_mod_sha256 => {
            write_frame_within::<T::Timer>(
                &mut session.send,
                &NetMsg::ContentReject(reject.clone()),
                timings.content_write,
                "content rejection",
            )
            .await?;
            Err(MultiplayerError::ContentDeclined(
                format!(
                    "downloaded host content failed local admission: {}",
                    reject.reason
                )
                .into(),
            ))
        }
        other => Err(MultiplayerError::LocalState(
            format!("expected local ContentReady/ContentPrepared/ContentReject, got {other:?}")
                .into(),
        )),
    }
}

/// A reconnect may bypass byte transfer only for the identical offer that
/// this same live client session already admitted and mounted. Any content
/// change or content/no-content downgrade is a hard reconnect failure.
async fn resolve_reconnect_prelude<T: ClientTransport>(
    prelude: HandshakePrelude,
    admitted: Option<&DistributedModOffer>,
) -> Result<(ClientSession, WelcomeData), MultiplayerError> {
    let offered = match &prelude {
        HandshakePrelude::Welcome { .. } => None,
        HandshakePrelude::Content { offer, .. } => Some(offer),
    };
    validate_reconnect_content(offered, admitted)?;
    match prelude {
        HandshakePrelude::Welcome { session, welcome } => Ok((session, welcome)),
        HandshakePrelude::Content { mut session, offer } => {
            write_frame_within::<T::Timer>(
                &mut session.send,
                &NetMsg::ContentRequest(robin_engine::multiplayer::ContentRequest {
                    full_mod_sha256: offer.full_mod_sha256,
                    resume_offset: offer.encoded_bytes,
                }),
                T::TIMINGS.content_write,
                "reconnect content request",
            )
            .await?;
            write_frame_within::<T::Timer>(
                &mut session.send,
                &NetMsg::ContentReady {
                    full_mod_sha256: offer.full_mod_sha256,
                },
                T::TIMINGS.content_write,
                "reconnect content readiness",
            )
            .await?;
            read_welcome::<T>(session).await
        }
    }
}

/// One whole reconnect attempt under one deadline: the handshake and the
/// content prelude that handshake opened belong to the same attempt, so the
/// deadline covers both. Bounding only the handshake would leave the prelude's
/// reads and writes outside every deadline, where the budget is not read and no
/// later check can run.
async fn attempt_reconnect<T: ClientTransport>(
    transport: &T,
    admitted: Option<&DistributedModOffer>,
    deadline: Duration,
) -> Option<Result<(ClientSession, WelcomeData), MultiplayerError>> {
    let work = async {
        let prelude = handshake(transport).await?;
        resolve_reconnect_prelude::<T>(prelude, admitted).await
    };
    let attempt = async {
        with_timeout::<T::Timer, _>(deadline, work)
            .await
            .unwrap_or_else(|()| {
                Err(MultiplayerError::timeout(
                    "reconnecting to the host",
                    deadline,
                ))
            })
    };
    cancellable::<T::Timer, _>(transport.cancellation(), attempt).await
}

// ─── Connection lifecycle ────────────────────────────────────────

/// What the reconnect loop may do next: one attempt with this deadline, or the
/// named end of the session.
enum ReconnectStep {
    Attempt(Duration),
    Spent(MultiplayerError),
}

/// One pass of the reconnect loop against the budget the drop opened.
///
/// The budget is a wall-clock cap on the whole loop, so an attempt is capped by
/// whatever is left of it: a host that never returns ends the session at the
/// budget, not one whole attempt after it. `started` is `None` while no budget
/// is open — a live session, which has nothing to spend — and a whole attempt
/// is then the right answer.
fn reconnect_step(
    started: Option<web_time::Instant>,
    budget: Duration,
    attempt: Duration,
    host: &str,
) -> ReconnectStep {
    match started.map(|started| budget.saturating_sub(started.elapsed())) {
        Some(remaining) if remaining.is_zero() => ReconnectStep::Spent(MultiplayerError::timeout(
            format!("reconnecting to host {host}"),
            budget,
        )),
        Some(remaining) => ReconnectStep::Attempt(attempt.min(remaining)),
        None => ReconnectStep::Attempt(attempt),
    }
}

/// Leave the session because the host did not come back in time.
///
/// The published fatal is the outcome, not the log: it is what the game loop
/// turns into leaving the mission, and therefore what releases the
/// `pause_simulation` that `reconnecting()` holds for the whole retry.
fn give_up_on_reconnect(incoming: &Sender<NetEvent>, error: MultiplayerError) {
    tracing::warn!(%error, "multiplayer: giving up on reconnection");
    let _ = incoming.send(NetEvent::Fatal(NetFatal::new(error)));
}

/// Drive one connection: initial handshake and content admission, then each
/// session until it ends, auto-reconnecting with exponential backoff. Returns
/// when the game loop drops the outgoing queue, shutdown is requested, or a
/// fatal error was published.
pub(super) async fn run_client_io<T: ClientTransport>(
    transport: &T,
    outbound: &mut T::Outbound,
    incoming: Sender<NetEvent>,
    slots: &ClientSlots,
) {
    let prelude = match initial_handshake(transport).await {
        Ok(prelude) => prelude,
        Err(error) => {
            transport.startup_failed(slots, &incoming, StartupFailure::Connect, error);
            return;
        }
    };

    let (mut session, welcome, admitted_offer) = match prelude {
        HandshakePrelude::Welcome { session, welcome } => (session, welcome, None),
        HandshakePrelude::Content { session, offer } => {
            *slot(&slots.content_offer) = Some(offer.clone());
            if deliver(&incoming, NetEvent::ContentOffer(offer.clone())).is_err() {
                return;
            }
            if transport
                .publish_initial_handshake(InitialHandshake::ContentOffered {
                    full_mod_sha256: offer.full_mod_sha256,
                })
                .is_err()
            {
                return;
            }
            match complete_content_admission(transport, session, &offer, &incoming, outbound).await
            {
                Ok(ContentAdmissionCompletion::Join(session, welcome)) => {
                    (session, welcome, Some(offer))
                }
                Ok(ContentAdmissionCompletion::Prepared) => {
                    let _ = incoming.send(NetEvent::Note(format!(
                        "verified and cached host content {} without joining a gameplay seat",
                        robin_engine::spellforge::hex_hash(&offer.full_mod_sha256)
                    )));
                    return;
                }
                Err(error) => {
                    transport.startup_failed(
                        slots,
                        &incoming,
                        StartupFailure::Admission,
                        error.context("distributed-mod admission failed"),
                    );
                    return;
                }
            }
        }
    };
    let admitted_session =
        match ClientSessionMetadata::from_welcome(&welcome, admitted_offer.clone()) {
            Ok(session) => session,
            Err(error) => {
                transport.startup_failed(slots, &incoming, StartupFailure::Welcome, error);
                return;
            }
        };
    if let Err(error) = transport.after_welcome().await {
        transport.startup_failed(slots, &incoming, StartupFailure::Welcome, error);
        return;
    }
    let WelcomeData {
        seat: your_seat,
        mission_id,
        mission_seed,
        sim_config,
        speech_timing_locale,
        session_id,
    } = welcome;

    *slot(&slots.session_metadata) = Some(admitted_session);
    *slot(&slots.content_offer) = admitted_offer.clone();
    if admitted_offer.is_none()
        && transport
            .publish_initial_handshake(InitialHandshake::Welcomed {
                seat: your_seat,
                mission_seed,
            })
            .is_err()
    {
        return;
    }
    if deliver_lifecycle(
        &incoming,
        [
            NetEvent::AssignedLocalSeat(your_seat),
            NetEvent::MissionConfig {
                mission_id: mission_id.clone(),
                rng_seed: mission_seed,
                sim_config,
                speech_timing_locale: speech_timing_locale.clone(),
            },
        ],
    )
    .is_err()
    {
        return;
    }

    let mut backoff = RECONNECT_BACKOFF;
    let mut reconnect_started: Option<web_time::Instant> = None;
    let host = transport.server_addr().id.to_string();
    loop {
        match run_session(transport, session, outbound, &incoming).await {
            SessionEnd::Drop(reason) => {
                let discarded = T::discard_outbound(outbound);
                tracing::warn!("client session ended: {reason}; reconnecting...");
                if discarded != 0 {
                    tracing::warn!(
                        discarded,
                        "multiplayer: discarded outbound commands from abandoned prediction session"
                    );
                }
                // A drop opens the reconnect budget; a successful reconnect
                // closes it. A budget already spent ends the session before the
                // reconnect is announced, so the player never sees a
                // "reconnecting" that cannot happen.
                reconnect_started.get_or_insert_with(web_time::Instant::now);
                if let ReconnectStep::Spent(error) = reconnect_step(
                    reconnect_started,
                    T::TIMINGS.reconnect_budget,
                    T::TIMINGS.reconnect_attempt,
                    &host,
                ) {
                    give_up_on_reconnect(&incoming, error);
                    return;
                }
                if deliver_lifecycle(
                    &incoming,
                    [
                        NetEvent::Note(format!("{reason}; reconnecting...")),
                        NetEvent::Disconnected,
                    ],
                )
                .is_err()
                {
                    return;
                }
            }
            SessionEnd::Fatal(error) => {
                let _ = incoming.send(NetEvent::Fatal(NetFatal::new(error)));
                return;
            }
            SessionEnd::TransitionCommitted => {
                wait_for_cancel::<T::Timer>(transport.cancellation()).await;
                return;
            }
            SessionEnd::OutgoingClosed => return,
        }

        if sleep_or_cancel::<T::Timer>(transport.cancellation(), backoff).await {
            return;
        }
        backoff = (backoff * 2).min(MAX_RECONNECT_BACKOFF);

        session = loop {
            if transport.cancellation().load(Ordering::Acquire) {
                return;
            }
            let deadline = match reconnect_step(
                reconnect_started,
                T::TIMINGS.reconnect_budget,
                T::TIMINGS.reconnect_attempt,
                &host,
            ) {
                ReconnectStep::Attempt(deadline) => deadline,
                ReconnectStep::Spent(error) => {
                    give_up_on_reconnect(&incoming, error);
                    return;
                }
            };
            let Some(attempt) =
                attempt_reconnect(transport, admitted_offer.as_ref(), deadline).await
            else {
                return;
            };
            match attempt {
                Ok((new_session, welcome)) => {
                    let WelcomeData {
                        seat: new_seat,
                        mission_id: new_mission_id,
                        mission_seed: new_seed,
                        sim_config: new_config,
                        speech_timing_locale: new_speech_timing_locale,
                        session_id: new_session_id,
                    } = welcome;
                    if let Err(error) = validate_reconnect_state(
                        ReconnectIdentity {
                            seat: your_seat,
                            mission_id: &mission_id,
                            seed: mission_seed,
                            config: sim_config,
                            speech_timing_locale: speech_timing_locale.as_deref(),
                            session_id,
                        },
                        ReconnectIdentity {
                            seat: new_seat,
                            mission_id: &new_mission_id,
                            seed: new_seed,
                            config: new_config,
                            speech_timing_locale: new_speech_timing_locale.as_deref(),
                            session_id: new_session_id,
                        },
                        admitted_offer.as_ref(),
                    ) {
                        let _ = incoming.send(NetEvent::Fatal(NetFatal::new(error)));
                        return;
                    }
                    tracing::info!(?new_seat, seed = new_seed, "client reconnected");
                    // Reconnect validation proved the published identity is unchanged.
                    if deliver_lifecycle(
                        &incoming,
                        [
                            NetEvent::Reconnected,
                            NetEvent::AssignedLocalSeat(new_seat),
                            NetEvent::MissionConfig {
                                mission_id: new_mission_id,
                                rng_seed: new_seed,
                                sim_config: new_config,
                                speech_timing_locale: new_speech_timing_locale,
                            },
                        ],
                    )
                    .is_err()
                    {
                        return;
                    }
                    let discarded = T::discard_outbound(outbound);
                    if discarded != 0 {
                        tracing::warn!(
                            discarded,
                            "multiplayer: discarded commands queued while transport was reconnecting"
                        );
                    }
                    backoff = RECONNECT_BACKOFF;
                    reconnect_started = None;
                    break new_session;
                }
                Err(error) => {
                    tracing::warn!("reconnect failed: {error}; will retry in {backoff:?}");
                }
            }
            if sleep_or_cancel::<T::Timer>(transport.cancellation(), backoff).await {
                return;
            }
            backoff = (backoff * 2).min(MAX_RECONNECT_BACKOFF);
        };
    }
}

/// Run one client session by racing a whole-session reader loop against a
/// whole-session writer loop, so local inputs are sent as soon as the game
/// loop queues them. The reader and writer each own their stream half for the
/// session's lifetime — a select over individual `read_frame` calls would drop
/// partially-read frames when another branch fires first.
async fn run_session<T: ClientTransport>(
    transport: &T,
    session: ClientSession,
    outbound: &mut T::Outbound,
    incoming: &Sender<NetEvent>,
) -> SessionEnd {
    let ClientSession {
        _conn,
        mut send,
        mut recv,
        protocol: _,
    } = session;
    let reader = async {
        loop {
            let message = match reader_outcome(
                read_frame(&mut recv, InboundFramePolicy::ServerToClient).await,
            ) {
                Ok(message) => message,
                Err(end) => return end,
            };
            let committed = matches!(message, NetMsg::CommitSnapshotTransition { .. });
            if let Err(error) = handle_client_wire_msg(incoming, message) {
                return if matches!(error, MultiplayerError::ReconnectRequired { .. }) {
                    SessionEnd::Drop(error)
                } else {
                    SessionEnd::Fatal(error)
                };
            }
            if committed {
                return SessionEnd::TransitionCommitted;
            }
        }
    };
    let writer = async {
        loop {
            let Some(outgoing) = T::recv_outbound(outbound).await else {
                return SessionEnd::OutgoingClosed;
            };
            if let Err(error) = send_client_outgoing(&mut send, outgoing).await {
                return SessionEnd::Drop(error);
            }
        }
    };
    let cancel = pin!(wait_for_cancel::<T::Timer>(transport.cancellation()));
    let reader = pin!(reader);
    let writer = pin!(writer);
    let end = match select(cancel, select(reader, writer)).await {
        Either::Left(((), _)) => SessionEnd::OutgoingClosed,
        Either::Right((Either::Left((end, _)) | Either::Right((end, _)), _)) => end,
    };
    if matches!(end, SessionEnd::Drop(_))
        && matches!(_conn.close_reason(), Some(iroh::endpoint::ConnectionError::ApplicationClosed(ref close))
            if close.error_code == HOST_LEFT_CLOSE_CODE.into())
    {
        return SessionEnd::Fatal(MultiplayerError::HostRejected {
            stage: "session ended",
            reason: "The host has left the game.".into(),
        });
    }
    end
}

// ─── In-session messages ─────────────────────────────────────────

/// Handle one host message received during a session.
pub(super) fn handle_client_wire_msg(
    incoming: &Sender<NetEvent>,
    message: NetMsg,
) -> Result<(), MultiplayerError> {
    let remote = |message: &'static str| MultiplayerError::RemoteProtocol(message.into());
    let Some(message) = super::client_gameplay::forward(message, incoming)? else {
        return Ok(());
    };
    match message {
        NetMsg::ModalProposal { .. } => Err(remote("server sent a client-only modal proposal")),
        NetMsg::ReconnectRequired { reason } => Err(MultiplayerError::ReconnectRequired { reason }),
        NetMsg::SnapshotTransitionReady { .. } => Err(remote(
            "server sent a client-only snapshot transition acknowledgement",
        )),
        NetMsg::CoopPauseAck { .. } => Err(remote(
            "server sent a client-only cooperative-pause acknowledgement",
        )),
        NetMsg::Reject { reason } => Err(MultiplayerError::HostRejected {
            stage: "session",
            reason,
        }),
        other => Err(MultiplayerError::RemoteProtocol(
            format!("host sent invalid session message {other:?}").into(),
        )),
    }
}

/// Turn one session read into the next host message or the end of the
/// session.
///
/// A clean close at a frame boundary is a drop and reconnects, like any other
/// transport loss. The host never half-closes a live session: it ends one with
/// `Reject` (fatal) or `ReconnectRequired`, or closes the whole connection (a
/// transport error), so a bare FIN carries no authoritative decision.
/// Ending here instead would leave the game holding simulation after
/// `Disconnected` with neither a reconnect nor a fatal error.
fn reader_outcome(read: Result<Option<NetMsg>, MultiplayerError>) -> Result<NetMsg, SessionEnd> {
    match read {
        Ok(Some(message)) => Ok(message),
        Ok(None) => Err(SessionEnd::Drop(MultiplayerError::transport(
            "host closed the multiplayer stream",
            std::io::Error::from(std::io::ErrorKind::UnexpectedEof),
        ))),
        Err(error) => Err(SessionEnd::Drop(error)),
    }
}

async fn send_client_outgoing(
    send: &mut SendStream,
    outgoing: NetOutbound,
) -> Result<(), MultiplayerError> {
    // A refused publication stays a typed `MultiplayerError::Protocol` all the
    // way into `SessionEnd`.
    let message = super::client_outgoing::prepare(outgoing)?;
    write_frame(send, &message).await
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Run the shared session handler.
    pub(in crate::multiplayer) fn handle(
        incoming: &Sender<NetEvent>,
        message: NetMsg,
    ) -> Result<(), MultiplayerError> {
        handle_client_wire_msg(incoming, message)
    }

    pub(in crate::multiplayer) fn begin_sim() -> NetMsg {
        NetMsg::BeginSim {
            frame: 9,
            start_epoch_ms: 12,
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_spent_reconnect_budget_ends_the_session_with_the_timeout() {
        let host = "endpoint-under-test";
        assert!(
            matches!(
                reconnect_step(None, RECONNECT_BUDGET, RECONNECT_BUDGET, host),
                ReconnectStep::Attempt(_)
            ),
            "a live session has no reconnect budget open"
        );
        assert!(
            matches!(
                reconnect_step(
                    Some(web_time::Instant::now()),
                    RECONNECT_BUDGET,
                    RECONNECT_BUDGET,
                    host
                ),
                ReconnectStep::Attempt(_)
            ),
            "a reconnect that has not spent its budget keeps retrying"
        );
        // A budget already spent is the observable end of the session: the
        // fatal timeout the player sees when the host stays away. It names the
        // host it was dialling, because the log line alone does not say which
        // session ended.
        let ReconnectStep::Spent(error) = reconnect_step(
            Some(web_time::Instant::now()),
            Duration::ZERO,
            RECONNECT_BUDGET,
            host,
        ) else {
            panic!("a spent budget ends the session");
        };
        assert!(error.to_string().contains(host), "{error}");
    }

    /// The budget is a wall-clock cap on the whole loop, so one attempt is
    /// never longer than what is left of it. This is the arithmetic that keeps
    /// a host that never returns from costing the budget plus one attempt.
    #[test]
    fn an_attempt_is_capped_by_what_is_left_of_the_budget() {
        let host = "endpoint-under-test";
        let attempt = Duration::from_secs(15);
        let budget = Duration::from_secs(60);
        let ago = |elapsed: Duration| {
            web_time::Instant::now()
                .checked_sub(elapsed)
                .expect("a test clock that has run that long")
        };
        let deadline =
            |elapsed: Duration| match reconnect_step(Some(ago(elapsed)), budget, attempt, host) {
                ReconnectStep::Attempt(deadline) => deadline,
                ReconnectStep::Spent(error) => panic!("budget not spent: {error}"),
            };
        assert_eq!(
            deadline(Duration::ZERO),
            attempt,
            "a fresh budget allows a whole attempt"
        );
        assert!(
            deadline(Duration::from_secs(55)) <= Duration::from_secs(5),
            "what is left of the budget caps the attempt"
        );
        let ReconnectStep::Spent(error) = reconnect_step(Some(ago(budget)), budget, attempt, host)
        else {
            panic!("a spent budget is the end, not a shorter attempt");
        };
        let message = error.to_string();
        assert!(message.contains(host), "{message}");
        assert!(message.contains("60s"), "{message}");
    }

    /// Giving up is the published fatal, not the log: that event is what the
    /// game loop turns into leaving the mission, and so what releases the
    /// `pause_simulation` that `reconnecting()` holds while the retry runs.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn giving_up_publishes_the_fatal_that_ends_the_session() {
        let (tx, rx) = std::sync::mpsc::channel();
        let ReconnectStep::Spent(error) = reconnect_step(
            Some(web_time::Instant::now()),
            Duration::ZERO,
            Duration::from_secs(15),
            "host-under-test",
        ) else {
            panic!("a spent budget ends the session");
        };
        give_up_on_reconnect(&tx, error);
        let NetEvent::Fatal(fatal) = rx.try_recv().expect("the fatal is the outcome") else {
            panic!("giving up must publish a fatal");
        };
        assert!(fatal.to_string().contains("host-under-test"), "{fatal}");
        assert!(rx.try_recv().is_err(), "one outcome, one event");
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn clean_stream_close_is_a_reconnectable_drop() {
        assert!(matches!(
            reader_outcome(Ok(None)),
            Err(SessionEnd::Drop(MultiplayerError::Transport { .. }))
        ));
        assert!(matches!(
            reader_outcome(Err(MultiplayerError::Handshake("reset".into()))),
            Err(SessionEnd::Drop(_))
        ));
        assert!(matches!(
            reader_outcome(Ok(Some(NetMsg::Note("next".into())))),
            Ok(NetMsg::Note(_))
        ));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn begin_sim_is_released_immediately() {
        let (tx, rx) = std::sync::mpsc::channel();
        handle(&tx, begin_sim()).unwrap();
        assert!(matches!(
            rx.try_recv().unwrap(),
            NetEvent::BeginSim {
                frame: 9,
                start_epoch_ms: 12
            }
        ));
        assert!(rx.try_recv().is_err());
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn begin_sim_requires_a_live_local_receiver() {
        let (tx, rx) = std::sync::mpsc::channel();
        drop(rx);
        assert!(
            handle(&tx, begin_sim())
                .unwrap_err()
                .to_string()
                .contains("channel is closed")
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn wrong_direction_messages_are_client_only_errors() {
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(
            handle(
                &tx,
                NetMsg::SnapshotTransitionReady {
                    id: robin_engine::multiplayer::SnapshotTransitionId {
                        session_id: MultiplayerSessionId([5; 32]),
                        sequence: 1,
                    },
                },
            )
            .unwrap_err()
            .to_string()
            .contains("client-only")
        );
        assert!(
            handle(&tx, NetMsg::Note("late".into())).is_ok(),
            "gameplay notes are forwarded"
        );
        assert!(matches!(rx.try_recv().unwrap(), NetEvent::Note(_)));
        assert!(
            handle(
                &tx,
                NetMsg::ContentOffer {
                    offer: DistributedModOffer {
                        schema_version: 1,
                        full_mod_sha256: [1; 32],
                        spellforge_package_sha256: None,
                        spellforge_vm_abi: None,
                        encoded_bytes: 1,
                        mission_basename: "Mission".into(),
                        mission_rhm_entry: "Data/Levels/Mission.rhm".into(),
                        map_filename: "Mission".into(),
                        title: "Mission".into(),
                        claimed_author: "Author".into(),
                        version: "1".into(),
                        source_url: "https://example.invalid".into(),
                        license: "CC0-1.0".into(),
                        host_endpoint_id: "endpoint-key".into(),
                    },
                },
            )
            .unwrap_err()
            .to_string()
            .contains("invalid session message")
        );
    }
}
