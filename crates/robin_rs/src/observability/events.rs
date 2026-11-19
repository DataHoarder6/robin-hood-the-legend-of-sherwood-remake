//! The session event ring: one JSON object per multiplayer-relevant event.
//!
//! The ring is bounded in memory so `/events` can page it while the game runs;
//! the file is append-only so a crash still leaves the narrative up to the last
//! event. Both are fed by [`Event`], and the same call site emits the tracing
//! line, so the log wording and the JSON `detail.message` cannot drift apart.

use robin_engine::player_command::PlayerId;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::VecDeque;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
use tracing::Level;

/// Events retained in memory. A session's narrative is tens of events, so this
/// holds a whole failed session with room to spare; the file holds the rest.
pub(crate) const RING_CAPACITY: usize = 512;

/// Default and maximum `/events` page sizes.
pub(crate) const DEFAULT_PAGE: usize = 100;
pub(crate) const MAX_PAGE: usize = 1000;

/// This run's file stops growing at this size; the ring keeps serving and the
/// log says once that the file stopped. An install cannot fill its disk with
/// one session.
#[cfg(not(target_arch = "wasm32"))]
const MAX_EVENT_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Previous runs' files kept beside the live `events.jsonl`.
#[cfg(not(target_arch = "wasm32"))]
const RETAINED_EVENT_ARCHIVES: usize = 2;
#[cfg(not(target_arch = "wasm32"))]
pub(super) const EVENT_FILE_NAME: &str = "events.jsonl";

/// Every kind this process records. The strings are wire vocabulary: `/events`
/// consumers match on them, so they change only with a documented schema move.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EventKind {
    MissionStarted,
    /// This process hosts the session; the endpoint and seat are its own.
    HostStarted,
    Admission,
    /// A peer joined, either by its `ConnectSeat` or this process dialing out.
    SeatConnected,
    /// The authoritative host assigned this process its seat.
    SeatAssigned,
    /// Only an admitted `DisconnectSeat` removes a participant.
    SeatEvicted,
    /// The transport lost its peer link and will retry; no seat was evicted.
    TransportDisconnected,
    TransportReconnected,
    BarrierReached,
    BarrierReleased,
    BarrierExpired,
    TransitionPrepared,
    TransitionCommitted,
    TransitionAborted,
    TransitionDropped,
    ModalAnnounced,
    ModalProposed,
    ModalAcked,
    ModalExpired,
    Desync,
    SessionFailed,
    SessionEnded,
}

/// One event before it is recorded. Built and finished at a single call site so
/// the tiered tracing line and the JSONL object carry the same facts.
pub(crate) struct Event {
    kind: EventKind,
    seat: Option<u8>,
    frame: Option<u32>,
    message: String,
    detail: Map<String, Value>,
    level: Level,
}

impl Event {
    pub(crate) fn new(kind: EventKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            seat: None,
            frame: None,
            message: message.into(),
            detail: Map::new(),
            level: Level::INFO,
        }
    }

    pub(crate) fn seat(mut self, seat: PlayerId) -> Self {
        self.seat = Some(seat.0);
        self
    }

    /// From a caller that has a wire seat number rather than a [`PlayerId`].
    pub(crate) fn seat_number_opt(mut self, seat: Option<u8>) -> Self {
        self.seat = seat;
        self
    }

    pub(crate) fn frame(mut self, frame: u32) -> Self {
        self.frame = Some(frame);
        self
    }

    pub(crate) fn field(mut self, key: &str, value: impl Serialize) -> Self {
        self.detail.insert(
            key.to_owned(),
            serde_json::to_value(value).expect("session event detail serializes"),
        );
        self
    }

    /// Failures and refusals stay at `warn`, the level the replaced tracing line
    /// used, so the log still surface-scans the way it did.
    pub(crate) fn warn(mut self) -> Self {
        self.level = Level::WARN;
        self
    }

    pub(crate) fn error(mut self) -> Self {
        self.level = Level::ERROR;
        self
    }

    /// Append to the ring and the file, then emit the matching tracing event.
    pub(crate) fn record(self) {
        let Event {
            kind,
            seat,
            frame,
            message,
            mut detail,
            level,
        } = self;
        detail.insert("message".into(), Value::String(message.clone()));
        let detail = Value::Object(detail);
        // One source for the wire name: the traced `kind` is the serialized
        // one, so the two cannot name a variant differently.
        let Value::String(kind_name) =
            serde_json::to_value(kind).expect("an event kind serializes")
        else {
            unreachable!("an event kind serializes as its wire name")
        };
        #[cfg(test)]
        let isolated = super::record_in_isolated_ring(kind, seat, frame, &detail);
        #[cfg(not(test))]
        let isolated = false;
        if !isolated {
            let record = super::surface()
                .events
                .push(kind, seat, frame, detail.clone());
            // The ring is already consistent; the file has its own lock so a
            // stalled install home blocks neither this thread's readers nor
            // `/state`. A test with its own ring is not reading the file.
            #[cfg(not(target_arch = "wasm32"))]
            append(&record);
        }
        let detail_text = serde_json::to_string(&detail).expect("session event detail serializes");
        match level {
            Level::WARN => tracing::warn!(
                kind = %kind_name,
                seat,
                frame,
                detail = %detail_text,
                "{message}"
            ),
            Level::ERROR => tracing::error!(
                kind = %kind_name,
                seat,
                frame,
                detail = %detail_text,
                "{message}"
            ),
            _ => tracing::info!(
                kind = %kind_name,
                seat,
                frame,
                detail = %detail_text,
                "{message}"
            ),
        }
    }
}

/// The bounded in-memory ring. `seq` restarts with every process, so the page
/// also names the run that served it: a poller holding an earlier cursor reads
/// the changed `run` instead of a quiet page.
pub(super) struct EventRing {
    run: String,
    next_seq: u64,
    retained: VecDeque<Value>,
}

impl Default for EventRing {
    fn default() -> Self {
        Self {
            run: new_run_id(),
            next_seq: 0,
            retained: VecDeque::new(),
        }
    }
}

impl EventRing {
    pub(super) fn run(&self) -> &str {
        &self.run
    }

    pub(super) fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub(super) fn push(
        &mut self,
        kind: EventKind,
        seat: Option<u8>,
        frame: Option<u32>,
        detail: Value,
    ) -> Value {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        let record = serde_json::json!({
            "seq": seq,
            "uptime_ms": crate::window::process_uptime_ms(),
            "utc": super::events_utc_now(),
            "kind": kind,
            "seat": seat,
            "frame": frame,
            "detail": detail,
        });
        self.retained.push_back(record.clone());
        while self.retained.len() > RING_CAPACITY {
            self.retained.pop_front();
        }
        record
    }

    /// Events with `seq >= since`, oldest first, at most `limit`. `next_seq` is
    /// always this ring's true cursor, so a `since` from another run cannot be
    /// echoed back as if the session were merely quiet.
    pub(super) fn page(&self, since: Option<u64>, limit: usize) -> EventsPage {
        let since = since.unwrap_or(0);
        let events: Vec<Value> = self
            .retained
            .iter()
            .filter(|event| event["seq"].as_u64().is_some_and(|seq| seq >= since))
            .take(limit)
            .cloned()
            .collect();
        let next_seq = events
            .last()
            .and_then(|event| event["seq"].as_u64())
            .map_or(self.next_seq, |seq| seq + 1);
        EventsPage {
            run: self.run.clone(),
            events,
            next_seq,
        }
    }
}

/// This ring's run identifier. `seq` is per process, so the value that changes
/// when a new process starts is what a poller compares.
fn new_run_id() -> String {
    static GENERATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static PROCESS: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "{}-{}",
            jiff::Zoned::now().strftime("%Y%m%dT%H%M%S").to_string(),
            std::process::id()
        )
    });
    let generation = GENERATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}#{generation}", *PROCESS)
}

/// One `/events` page. `run` identifies the process run serving it and
/// `next_seq` is the cursor to pass back as `since`.
#[derive(Debug, Serialize)]
pub(crate) struct EventsPage {
    pub(crate) run: String,
    pub(crate) events: Vec<Value>,
    pub(crate) next_seq: u64,
}

/// This run's append-only event file.
///
/// Its own lock, deliberately not the process surface: a write that stalls on a
/// slow or disconnected install home must not hold up `/state` or `/events`,
/// which exist precisely for a game thread that is stuck.
#[cfg(not(target_arch = "wasm32"))]
pub(super) struct EventFile {
    file: Option<std::fs::File>,
    path: Option<PathBuf>,
    /// Bytes appended to this run's file. The budget stops the appender rather
    /// than letting one runaway session fill the install's disk.
    written: u64,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for EventFile {
    fn default() -> Self {
        Self {
            file: None,
            path: None,
            written: 0,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
static EVENT_FILE: std::sync::LazyLock<std::sync::Mutex<EventFile>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(EventFile::default()));

#[cfg(not(target_arch = "wasm32"))]
fn event_file() -> std::sync::MutexGuard<'static, EventFile> {
    EVENT_FILE.lock().unwrap_or_else(|error| error.into_inner())
}

/// Open `<directory>/events.jsonl` for this run and return its path. A previous
/// run's file is archived first, so every `seq` in the live file belongs to one
/// process. The caller keeps the path: reading it must not need this writer's
/// lock, which may be held across a stalled write.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn install_file(directory: &Path) -> std::io::Result<PathBuf> {
    event_file().install(directory)
}

/// Hold the writer's lock for a test standing in for a stalled disk.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(super) fn stall_writer_for_test() -> std::sync::MutexGuard<'static, EventFile> {
    event_file()
}

/// Append one recorded event, once. The writer owns the file's budget and its
/// failure: after either, later events stay in the ring only.
#[cfg(not(target_arch = "wasm32"))]
fn append(record: &Value) {
    event_file().append(record);
}

#[cfg(not(target_arch = "wasm32"))]
impl EventFile {
    fn install(&mut self, directory: &Path) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(directory)?;
        let path = directory.join(EVENT_FILE_NAME);
        archive_previous(&path)?;
        self.file = Some(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?,
        );
        self.path = Some(path.clone());
        self.written = 0;
        Ok(path)
    }

    fn append(&mut self, record: &Value) {
        if self.file.is_none() {
            return;
        }
        let mut line = serde_json::to_vec(record).expect("session event serializes");
        line.push(b'\n');
        let bytes = line.len() as u64;
        if self.written + bytes > MAX_EVENT_FILE_BYTES {
            tracing::warn!(
                path = %self.path.as_deref().unwrap_or(Path::new(EVENT_FILE_NAME)).display(),
                bytes = self.written,
                budget = MAX_EVENT_FILE_BYTES,
                "session events: event file reached its budget; later events stay in memory only"
            );
            self.file = None;
            return;
        }
        let Some(file) = self.file.as_mut() else {
            return;
        };
        // One line, flushed: a crash keeps every event before it.
        match file.write_all(&line).and_then(|()| file.flush()) {
            Ok(()) => self.written += bytes,
            Err(error) => {
                tracing::warn!(%error, "session events: could not append to the event file");
                self.file = None;
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
use std::io::Write as _;

/// Move a previous run's narrative aside, so `events.jsonl` holds one process's
/// `seq` sequence and a reader never sees two runs interleaved.
#[cfg(not(target_arch = "wasm32"))]
fn archive_previous(path: &Path) -> std::io::Result<()> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Ok(());
    };
    if metadata.len() == 0 {
        return Ok(());
    }
    let stamp = jiff::Zoned::now().strftime("%Y%m%dT%H%M%S").to_string();
    let archive = path.with_file_name(format!("events-{stamp}-{}.jsonl", std::process::id()));
    std::fs::rename(path, &archive)?;
    prune_archives(path.parent().expect("event file has a parent"))?;
    Ok(())
}

/// Keep the newest [`RETAINED_EVENT_ARCHIVES`] rotated event files.
#[cfg(not(target_arch = "wasm32"))]
fn prune_archives(directory: &Path) -> std::io::Result<()> {
    let mut archives: Vec<PathBuf> = std::fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("events-") && name.ends_with(".jsonl"))
        })
        .collect();
    archives.sort();
    let excess = archives.len().saturating_sub(RETAINED_EVENT_ARCHIVES);
    for path in archives.into_iter().take(excess) {
        if let Err(error) = std::fs::remove_file(&path) {
            tracing::warn!(
                path = %path.display(),
                %error,
                "session events: could not prune an older event archive"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(ring: &mut EventRing, index: u32) {
        ring.push(
            EventKind::Admission,
            Some(1),
            Some(index),
            serde_json::json!({"message": format!("phase {index}"), "to": index}),
        );
    }

    #[test]
    fn ring_pages_in_order_and_stops_at_the_limit() {
        let mut ring = EventRing::default();
        for index in 0..5u32 {
            push(&mut ring, index);
        }
        assert_eq!(ring.next_seq(), 5);
        let first = ring.page(None, 2);
        assert_eq!(first.events.len(), 2);
        assert_eq!(first.events[0]["seq"], 0);
        assert_eq!(first.events[1]["frame"], 1);
        assert_eq!(first.next_seq, 2);

        let rest = ring.page(Some(first.next_seq), 10);
        assert_eq!(rest.events.len(), 3);
        assert_eq!(rest.events[0]["seq"], 2);
        assert_eq!(rest.next_seq, 5);

        let empty = ring.page(Some(5), 10);
        assert!(empty.events.is_empty());
        assert_eq!(empty.next_seq, 5);
    }

    #[test]
    fn a_cursor_from_another_run_names_the_new_run_and_is_not_echoed() {
        let mut previous = EventRing::default();
        for index in 0..4u32 {
            push(&mut previous, index);
        }
        let stale = previous.page(None, 10).next_seq;
        assert_eq!(stale, 4);

        let mut restarted = EventRing::default();
        push(&mut restarted, 0);
        let page = restarted.page(Some(stale), 10);
        assert!(
            page.events.is_empty(),
            "the stale cursor selects nothing from the new run"
        );
        assert_ne!(page.run, previous.page(None, 1).run);
        assert_eq!(page.run, restarted.run());
        assert_eq!(
            page.next_seq, 1,
            "the page reports the new run's true cursor, not the stale one"
        );
    }

    #[test]
    fn ring_is_bounded_and_keeps_the_newest_events() {
        let mut ring = EventRing::default();
        for index in 0..RING_CAPACITY + 3 {
            ring.push(
                EventKind::SeatEvicted,
                None,
                None,
                serde_json::json!({"message": format!("eviction {index}")}),
            );
        }
        assert_eq!(ring.retained.len(), RING_CAPACITY);
        let page = ring.page(None, MAX_PAGE);
        assert_eq!(page.events[0]["seq"], 3);
        assert_eq!(page.next_seq, (RING_CAPACITY + 3) as u64);
    }

    #[test]
    fn every_recorded_event_carries_the_documented_fields() {
        let mut ring = EventRing::default();
        ring.push(
            EventKind::Desync,
            Some(1),
            Some(75),
            serde_json::json!({"message": "multiplayer DESYNC: frame=75", "local": "00", "host": "01"}),
        );
        let event = &ring.page(None, 1).events[0];
        for key in ["seq", "uptime_ms", "utc", "kind", "seat", "frame", "detail"] {
            assert!(event.get(key).is_some(), "missing {key}: {event}");
        }
        assert_eq!(event["kind"], "desync");
        assert_eq!(event["seat"], 1);
        assert_eq!(event["frame"], 75);
        assert_eq!(event["detail"]["local"], "00");
        let utc = event["utc"].as_str().unwrap();
        assert!(utc.ends_with('Z'), "{utc}");
        assert!(utc.parse::<jiff::Timestamp>().is_ok(), "{utc}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn record(index: u32) -> Value {
        serde_json::json!({"seq": index, "detail": {"message": format!("event {index}")}})
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_new_run_archives_the_previous_file_and_starts_its_own_sequence() {
        let home = tempfile::tempdir().unwrap();
        let mut first = EventFile::default();
        first.install(home.path()).unwrap();
        first.append(&record(0));
        drop(first);

        let mut second = EventFile::default();
        second.install(home.path()).unwrap();
        second.append(&record(0));
        let live = std::fs::read_to_string(home.path().join(EVENT_FILE_NAME)).unwrap();
        assert_eq!(live.lines().count(), 1, "{live}");
        assert_eq!(
            serde_json::from_str::<Value>(live.lines().next().unwrap()).unwrap()["seq"],
            0
        );
        let archives: Vec<String> = std::fs::read_dir(home.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("events-"))
            .collect();
        assert_eq!(archives.len(), 1, "{archives:?}");
        assert!(archives[0].ends_with(".jsonl"), "{archives:?}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_open_event_file_receives_one_json_object_per_line() {
        let home = tempfile::tempdir().unwrap();
        let mut file = EventFile::default();
        file.install(home.path()).unwrap();
        for index in 0..3u32 {
            file.append(&record(index));
        }
        let text = std::fs::read_to_string(home.path().join(EVENT_FILE_NAME)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        for (expected, line) in lines.iter().enumerate() {
            let value: Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["seq"], expected as u64);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[derive(Clone)]
    struct CaptureWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    #[cfg(not(target_arch = "wasm32"))]
    impl std::io::Write for CaptureWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
        type Writer = CaptureWriter;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Past the budget the appender stops and says so once. It used to keep
    /// re-warning for every later event, because the condition stayed true after
    /// the file was dropped. The ring is a separate owner now, so it keeps
    /// serving all of them.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_file_budget_warns_once_and_the_ring_keeps_serving() {
        use tracing_subscriber::prelude::*;

        let home = tempfile::tempdir().unwrap();
        let mut file = EventFile::default();
        file.install(home.path()).unwrap();
        file.written = MAX_EVENT_FILE_BYTES;
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(CaptureWriter(captured.clone())),
        );
        tracing::subscriber::with_default(subscriber, || {
            for index in 0..3u32 {
                file.append(&record(index));
            }
        });
        let log = String::from_utf8(
            captured
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone(),
        )
        .unwrap();
        assert_eq!(
            log.matches("reached its budget").count(),
            1,
            "one warning per run: {log}"
        );

        let mut ring = EventRing::default();
        for index in 0..3u32 {
            ring.push(
                EventKind::Admission,
                None,
                Some(index),
                serde_json::json!({"message": format!("budget {index}")}),
            );
        }
        assert_eq!(
            ring.page(None, 10).events.len(),
            3,
            "the ring serves events the file stopped keeping"
        );
    }
}
