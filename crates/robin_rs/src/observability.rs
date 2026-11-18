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
}

impl Surface {
    fn new() -> Self {
        Self {
            events: events::EventRing::default(),
            state: state::StateSurface::default(),
            #[cfg(not(target_arch = "wasm32"))]
            log: log_file::RunLog::default(),
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
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_run_log() -> Option<log_file::TraceWriter> {
    let mut surface = surface();
    let Some(directory) = crate::install_home::log_directory() else {
        surface.log = log_file::RunLog::default();
        surface.log.deny("no install home for <install home>/logs");
        return None;
    };
    let opened = match log_file::open(&directory) {
        Ok(opened) => opened,
        Err(error) => {
            surface
                .log
                .deny(&format!("cannot open {}: {error}", directory.display()));
            return None;
        }
    };
    let writer = opened.writer();
    surface.log = opened;
    if let Err(error) = surface.events.install(&directory) {
        // The ring keeps working; only durability is lost.
        surface.log.deny(&format!(
            "cannot open {}: {error}",
            directory.join("events.jsonl").display()
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
            surface.events.path().map(|path| path.to_path_buf()),
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

/// A mission's first RPC boundary opens a new narrative.
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
pub(crate) fn record_terminal(operation: Option<String>, reason: impl Into<String>) {
    let reason = reason.into();
    surface()
        .state
        .record_terminal(operation.clone(), reason.clone());
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
    #[cfg(target_arch = "wasm32")]
    let trace_log: Option<std::path::PathBuf> = None;
    // `next_event_seq` is the cursor a poller passes to `/events` to see only
    // what happens after this snapshot.
    document["observability"] = serde_json::json!({
        "trace_log": trace_log,
        "event_log": surface.events.path().map(std::path::Path::to_path_buf),
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
pub(crate) use test_support::lock as lock_process_surface;

/// The surface and the install-home environment are process-global, so every
/// test that observes either takes this guard instead of seeing another test's
/// session.
#[cfg(test)]
mod test_support {
    use std::sync::{Mutex, MutexGuard};

    pub(crate) struct ProcessSurface {
        _surface: MutexGuard<'static, ()>,
        _environment: MutexGuard<'static, ()>,
    }

    pub(crate) fn lock() -> ProcessSurface {
        static TEST_LOCK: Mutex<()> = Mutex::new(());
        ProcessSurface {
            _surface: TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner()),
            _environment: crate::install_home::ENV_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_event_reaches_the_ring_and_the_document_names_the_evidence() {
        let _process = test_support::lock();
        let home = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("ROBINHOOD_INSTALL_HOME", home.path()) };
        install_run_log();
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
        assert_eq!(
            run_log_path().unwrap().parent().unwrap(),
            home.path().join("logs")
        );
        unsafe { std::env::remove_var("ROBINHOOD_INSTALL_HOME") };
    }

    #[test]
    fn the_ring_outlives_the_mission_and_serves_a_cursor_in_order() {
        let _process = test_support::lock();
        let home = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("ROBINHOOD_INSTALL_HOME", home.path()) };
        install_run_log();
        mission_started("Dem_Lei_MP");
        let first = events_page(None, MAX_PAGE).next_seq;
        event(EventKind::BarrierReleased, "multiplayer: barrier released")
            .frame(12)
            .record();
        let second = events_page(None, MAX_PAGE).next_seq;
        event(EventKind::Desync, "multiplayer: DESYNC").record();
        record_terminal(Some("Quit".into()), "mission ended");
        // The mission is gone; the narrative it left is not.
        let page = events_page(Some(first), 16);
        let kinds: Vec<&str> = page
            .events
            .iter()
            .filter_map(|event| event["kind"].as_str())
            .collect();
        assert_eq!(
            kinds,
            ["barrier_released", "desync", "session_ended"],
            "{page:?}"
        );
        let seqs: Vec<u64> = page
            .events
            .iter()
            .filter_map(|event| event["seq"].as_u64())
            .collect();
        assert!(seqs.windows(2).all(|pair| pair[0] < pair[1]), "{seqs:?}");
        assert_eq!(page.next_seq, seqs.last().unwrap() + 1);
        let tail = events_page(Some(second), 16);
        assert!(
            tail.events
                .iter()
                .all(|event| event["seq"].as_u64().unwrap() >= second),
            "{tail:?}"
        );
        assert_eq!(
            events_page(Some(page.next_seq), 16).events.len(),
            0,
            "the cursor past the end serves nothing"
        );
        unsafe { std::env::remove_var("ROBINHOOD_INSTALL_HOME") };
    }
}
