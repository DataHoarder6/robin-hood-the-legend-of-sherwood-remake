//! The process's durable observability surface: the run log every process
//! writes, the session event ring served at `/events`, and the last `/state`
//! answer.
//!
//! These three facts must outlive the mission that produced them and stay
//! readable while the game thread is stuck, so they are owned here — one
//! process-lifetime structure, the same lifetime class as
//! [`crate::diagnostic_context`] and [`crate::bug_report`] — rather than by a
//! mission, a host or a transport. A mission-scoped owner would be dropped at
//! exactly the moment its evidence is needed (a failed admission, a deadlocked
//! frame loop, a mission that ended before the next RPC drain), and the HTTP
//! listener thread could not answer from it.
//!
//! Nothing here is simulation state: no engine value is derived from it, and a
//! lost event changes no gameplay. The contract is documented in
//! `docs/JSON_SERVER.md` and `docs/MULTIPLAYER.md`.

mod events;
#[cfg(not(target_arch = "wasm32"))]
mod log_file;
mod state;

use std::sync::{LazyLock, Mutex, MutexGuard};

pub(crate) use events::{DEFAULT_PAGE, Event, EventKind, EventsPage, MAX_PAGE};

/// Everything this process knows about itself that survives a mission.
struct Surface {
    events: events::EventRing,
    state: state::StateSurface,
    #[cfg(not(target_arch = "wasm32"))]
    log: log_file::RunLog,
    /// This run's event file, named beside the ring so a reader never has to
    /// take the event writer's lock to learn where the file is.
    #[cfg(not(target_arch = "wasm32"))]
    event_log: Option<std::path::PathBuf>,
}

impl Surface {
    fn new() -> Self {
        Self {
            events: events::EventRing::default(),
            state: state::StateSurface::default(),
            #[cfg(not(target_arch = "wasm32"))]
            log: log_file::RunLog::default(),
            #[cfg(not(target_arch = "wasm32"))]
            event_log: None,
        }
    }
}

static SURFACE: LazyLock<Mutex<Surface>> = LazyLock::new(|| Mutex::new(Surface::new()));

fn surface() -> MutexGuard<'static, Surface> {
    SURFACE.lock().unwrap_or_else(|error| error.into_inner())
}

// ── Run log ────────────────────────────────────────────────────────────

/// Open this run's log file under the install home's `logs/`, prune older runs,
/// and return the writer the tracing subscriber appends to.
///
/// Called before the subscriber is installed, so nothing here can be reported
/// through tracing yet; [`announce_run_log`] reports the outcome afterwards.
///
/// Both files are opened before the surface is touched: a lock that a reader of
/// `/state` or `/events` also needs must never be held across filesystem work.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_run_log() -> Option<log_file::TraceWriter> {
    let Some(directory) = crate::install_home::log_directory() else {
        let mut surface = surface();
        surface.log = log_file::RunLog::default();
        surface.log.deny("no install home for <install home>/logs");
        return None;
    };
    install_run_log_at(&directory)
}

/// Install this run's evidence under an explicit directory. Tests name their
/// own, so observing the surface never requires touching the environment.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_run_log_at(directory: &std::path::Path) -> Option<log_file::TraceWriter> {
    let opened = log_file::open(directory);
    let event_log = events::install_file(directory);
    let mut surface = surface();
    // The ring's file is a fact even when the run log failed to open: name it
    // so `/state` and the failure bundle can point at it.
    surface.event_log = event_log.as_ref().ok().cloned();
    let opened = match opened {
        Ok(opened) => opened,
        Err(error) => {
            surface.log = log_file::RunLog::default();
            surface
                .log
                .deny(&format!("cannot open {}: {error}", directory.display()));
            return None;
        }
    };
    let writer = opened.writer();
    surface.log = opened;
    if let Err(error) = event_log {
        // The ring keeps working; only durability is lost.
        surface.log.deny(&format!(
            "cannot open {}: {error}",
            directory.join(events::EVENT_FILE_NAME).display()
        ));
    }
    writer
}

/// Report the run log and event file paths once, at startup, now that the
/// subscriber can carry the lines.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn announce_run_log() {
    let (path, events, failure) = {
        let mut surface = surface();
        (
            surface.log.path().map(|path| path.to_path_buf()),
            surface.event_log.clone(),
            surface.log.take_failure(),
        )
    };
    if let Some(failure) = failure {
        tracing::warn!("run log unavailable: {failure}");
    }
    if let Some(path) = path {
        tracing::info!("run log → {}", path.display());
    }
    if let Some(path) = events {
        tracing::info!("session events → {}", path.display());
    }
}

/// The run log this process writes, for `/state` and the failure bundle.
pub(crate) fn run_log_path() -> Option<std::path::PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        surface().log.path().map(|path| path.to_path_buf())
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
}

// ── Session events ─────────────────────────────────────────────────────

/// Start one session event. Finish it with [`Event::record`] (or its level
/// variants), which appends the JSON line and emits the tracing line.
pub(crate) fn event(kind: EventKind, message: impl Into<String>) -> Event {
    Event::new(kind, message)
}

/// Events with `seq >= since`, oldest first, at most `limit`.
pub(crate) fn events_page(since: Option<u64>, limit: usize) -> EventsPage {
    #[cfg(test)]
    if let Some(page) = isolated_ring(|ring| ring.page(since, limit.min(MAX_PAGE))) {
        return page;
    }
    surface().events.page(since, limit.min(MAX_PAGE))
}

// ── A test's own event ring ────────────────────────────────────────────
//
// The ring is process-global because one process is one run. The lib test
// binary runs hundreds of runs in one process, so a test that asserts on an
// exact sequence reads the records its neighbours emit in the same window
// (measured 2026-11-19: a reconnect test saw another test's disconnect and
// concluded it had evicted a seat). A test that needs the exact sequence
// installs a ring of its own for the duration; production never does, and the
// shared file keeps recording the process narrative either way.

#[cfg(test)]
thread_local! {
    static ISOLATED_EVENTS: std::cell::RefCell<Option<events::EventRing>> =
        const { std::cell::RefCell::new(None) };
}

/// Restores the shared ring when the test that installed an isolated one ends.
#[cfg(test)]
pub(crate) struct SessionEventsIsolation(());

#[cfg(test)]
impl Drop for SessionEventsIsolation {
    fn drop(&mut self) {
        ISOLATED_EVENTS.with(|ring| *ring.borrow_mut() = None);
    }
}

/// Record this thread's events in a fresh ring instead of the process's.
#[cfg(test)]
pub(crate) fn isolate_session_events() -> SessionEventsIsolation {
    ISOLATED_EVENTS.with(|ring| *ring.borrow_mut() = Some(events::EventRing::default()));
    SessionEventsIsolation(())
}

#[cfg(test)]
fn isolated_ring<T>(use_ring: impl FnOnce(&mut events::EventRing) -> T) -> Option<T> {
    ISOLATED_EVENTS.with(|ring| ring.borrow_mut().as_mut().map(use_ring))
}

/// True when the event went to this thread's isolated ring, so the caller can
/// leave the process ring and the shared file alone.
#[cfg(test)]
pub(crate) fn record_in_isolated_ring(
    kind: EventKind,
    seat: Option<u8>,
    frame: Option<u32>,
    detail: &serde_json::Value,
) -> bool {
    isolated_ring(|ring| ring.push(kind, seat, frame, detail.clone())).is_some()
}

/// The application is about to end, and not because it crashed: name the
/// boundary that decided it. A report of "the game just turned off" otherwise
/// leaves only `game future returned, exit_code=0`, which is also the last line
/// of an ordinary quit (#47).
pub(crate) fn application_exit(reason: &str) {
    event(
        EventKind::ApplicationExit,
        format!("application exiting: {reason}"),
    )
    .warn()
    .field("reason", reason)
    .record();
}

/// Publish the page this peer is presenting, or `None` when nothing is on
/// screen. Called by the terminal flow with the decision it validates commands
/// against, so `/state` and `/command` cannot disagree about the screen.
pub(crate) fn publish_presentation(presentation: Option<serde_json::Value>) {
    surface().state.publish_presentation(presentation);
}

// ── `/state` ───────────────────────────────────────────────────────────

/// A mission attempt begins: its own generation, its own terminal, and no
/// snapshot borrowed from the mission before it.
pub(crate) fn mission_started(mission: &str) {
    surface().state.mission_started();
    event(
        EventKind::MissionStarted,
        format!("mission started: {mission}"),
    )
    .field("mission", mission)
    .record();
}

/// Publish the snapshot `/state` answers with, including while no game thread
/// is available to build another one.
pub(crate) fn publish_state(snapshot: serde_json::Value) {
    surface().state.publish(snapshot);
}

/// Record why this process stopped producing live snapshots.
///
/// The surface decides whether this call is the attempt's terminal: a second
/// writer for the same outcome is ignored, so one outcome leaves one
/// `session_ended` event and one `terminal` block.
pub(crate) fn record_terminal(operation: Option<String>, reason: impl Into<String>) {
    let reason = reason.into();
    let recorded = surface()
        .state
        .record_terminal(operation.clone(), reason.clone());
    if !recorded {
        return;
    }
    event(EventKind::SessionEnded, format!("session ended: {reason}"))
        .field("operation", operation)
        .field("reason", reason)
        .record();
}

/// The full `/state` document: last snapshot, `terminal`, and this run's
/// evidence paths. `None` before the process has attempted a mission.
pub(crate) fn state_document() -> Option<serde_json::Value> {
    document(&surface())
}

/// [`state_document`] without waiting for the surface lock.
///
/// The crash hook runs inside a panic, which may have happened while this very
/// thread held the surface: blocking there would turn a crash into a hang.
/// `None` means "not available right now", not "nothing happened".
pub(crate) fn state_document_if_unlocked() -> Option<serde_json::Value> {
    match SURFACE.try_lock() {
        Ok(surface) => document(&surface),
        Err(std::sync::TryLockError::Poisoned(error)) => document(&error.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => None,
    }
}

fn document(surface: &Surface) -> Option<serde_json::Value> {
    let mut document = surface.state.document()?;
    #[cfg(not(target_arch = "wasm32"))]
    let trace_log = surface.log.path().map(std::path::Path::to_path_buf);
    #[cfg(not(target_arch = "wasm32"))]
    let event_log = surface.event_log.clone();
    #[cfg(target_arch = "wasm32")]
    let trace_log: Option<std::path::PathBuf> = None;
    #[cfg(target_arch = "wasm32")]
    let event_log: Option<std::path::PathBuf> = None;
    // `next_event_seq` is the cursor a poller passes to `/events` to see only
    // what happens after this snapshot; `event_run` names the run it belongs
    // to, so a cursor from an earlier run is recognisable. A test with its own
    // ring must see that ring's cursor, or the cursor it reads here would page
    // the process ring instead.
    #[cfg(test)]
    let (event_run, next_event_seq) =
        isolated_ring(|ring| (ring.run().to_owned(), ring.next_seq()))
            .unwrap_or_else(|| (surface.events.run().to_owned(), surface.events.next_seq()));
    #[cfg(not(test))]
    let (event_run, next_event_seq) = (surface.events.run().to_owned(), surface.events.next_seq());
    document["observability"] = serde_json::json!({
        "trace_log": trace_log,
        "event_log": event_log,
        "event_run": event_run,
        "next_event_seq": next_event_seq,
    });
    Some(document)
}

// ── Shared helpers ─────────────────────────────────────────────────────

/// Wall-clock stamp used by session events and the `terminal` block:
/// `YYYY-MM-DDTHH:MM:SS.mmmZ`, which `jiff` parses and a reader can sort.
fn events_utc_now() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ring is the process's narrative: an event recorded here is served by
    /// the next cursor, whether or not a mission is alive to own it. `/state`
    /// names the same run's evidence even when the mission is gone — that is
    /// what a failure bundle reads.
    ///
    /// Tests share the process surface, so each one asserts only what its own
    /// cursor and its own event fix; nothing here requires another test to be
    /// quiet.
    #[test]
    fn a_recorded_event_reaches_the_ring_and_the_document_names_the_evidence() {
        let home = tempfile::tempdir().unwrap();
        install_run_log_at(home.path());
        let before = events_page(None, MAX_PAGE).next_seq;
        event(EventKind::Admission, "multiplayer: admission test marker")
            .seat_number_opt(Some(1))
            .frame(9)
            .field("to", "running")
            .record();
        let page = events_page(Some(before), MAX_PAGE);
        let recorded = page
            .events
            .iter()
            .find(|event| event["detail"]["message"] == "multiplayer: admission test marker")
            .expect("recorded event is served");
        assert_eq!(recorded["kind"], "admission");
        assert_eq!(recorded["seat"], 1);
        assert_eq!(recorded["frame"], 9);
        assert_eq!(recorded["detail"]["to"], "running");

        mission_started("Dem_Lei_MP");
        let document = state_document().expect("an attempt has started");
        // The crash hook's non-blocking read answers from the same surface. It
        // is compared by presence only: other tests record into the same ring,
        // so the cursor in a second read is theirs, not this test's.
        assert!(state_document_if_unlocked().is_some());
        let observability = &document["observability"];
        assert!(observability["trace_log"].is_string(), "{document}");
        assert!(observability["event_log"].is_string(), "{document}");
        assert!(observability["event_run"].is_string(), "{document}");
        assert!(observability["next_event_seq"].is_u64(), "{document}");
    }

    /// A cursor is a sequence position, not a count: a page starts at the cursor
    /// it was given and reports where the next page starts.
    #[test]
    fn a_cursor_walks_the_ring_in_order() {
        event(EventKind::BarrierReleased, "multiplayer: barrier released")
            .frame(12)
            .record();
        let mark = events_page(None, MAX_PAGE).next_seq;
        event(EventKind::Desync, "multiplayer: DESYNC").record();
        let page = events_page(Some(mark), MAX_PAGE);
        let desync = page
            .events
            .iter()
            .find(|event| event["kind"] == "desync")
            .expect("the event recorded after the cursor is served");
        let desync_seq = desync["seq"].as_u64().unwrap();
        // The ring is process-global and other tests record into it, so this
        // surface can only state ordering: the page starts at or after the
        // cursor, and the next cursor follows the last event served. Exact
        // adjacency of two records is pinned on a private ring in
        // `observability::events`.
        assert!(
            desync_seq >= mark,
            "an event recorded after the cursor is never served below it"
        );
        assert!(
            page.events
                .iter()
                .all(|event| event["seq"].as_u64().unwrap() >= mark),
            "{page:?}"
        );
        assert!(
            page.next_seq > desync_seq,
            "the next cursor follows the last event this page served"
        );
        assert!(
            events_page(Some(u64::MAX), MAX_PAGE).events.is_empty(),
            "a cursor past every event serves nothing"
        );
    }

    /// `/state` and `/events` exist for a game thread that is stuck, so neither
    /// may wait on the event file writer's lock.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_state_read_does_not_wait_for_the_event_writer() {
        let home = tempfile::tempdir().unwrap();
        install_run_log_at(home.path());
        mission_started("Dem_Lei_MP");
        publish_state(serde_json::json!({"frame": 12, "map": "Dem_Lei_MP"}));
        // A writer that never releases its lock stands in for a stalled disk.
        let stalled = events::stall_writer_for_test();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            tx.send(state_document().is_some()).unwrap();
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_secs(5))
                .expect("the state read is not blocked by the event writer"),
            "the published snapshot is still served"
        );
        drop(stalled);
    }
}
