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
    surface().events.page(since, limit.min(MAX_PAGE))
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
    let surface = surface();
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
    // to, so a cursor from an earlier run is recognisable.
    document["observability"] = serde_json::json!({
        "trace_log": trace_log,
        "event_log": event_log,
        "event_run": surface.events.run(),
        "next_event_seq": surface.events.next_seq(),
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
        assert_eq!(
            desync["seq"].as_u64().unwrap(),
            mark,
            "the event lands at the cursor that preceded it"
        );
        assert!(
            page.events
                .iter()
                .all(|event| event["seq"].as_u64().unwrap() >= mark),
            "{page:?}"
        );
        assert_eq!(page.next_seq, mark + 1);
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
