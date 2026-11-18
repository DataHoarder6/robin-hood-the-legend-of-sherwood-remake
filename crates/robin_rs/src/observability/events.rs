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
use std::path::{Path, PathBuf};
use tracing::Level;

/// Events retained in memory. A session's narrative is tens of events, so this
/// holds a whole failed session with room to spare; the file holds the rest.
pub(crate) const RING_CAPACITY: usize = 512;

/// Default and maximum `/events` page sizes.
pub(crate) const DEFAULT_PAGE: usize = 100;
pub(crate) const MAX_PAGE: usize = 1000;

/// This run's file stops growing at this size; the ring keeps serving and the
/// log says the file stopped. An install cannot fill its disk with one session.
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
    Admission,
    SeatConnected,
    SeatAssigned,
    SeatEvicted,
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

impl EventKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::MissionStarted => "mission_started",
            Self::Admission => "admission",
            Self::SeatConnected => "seat_connected",
            Self::SeatAssigned => "seat_assigned",
            Self::SeatEvicted => "seat_evicted",
            Self::BarrierReached => "barrier_reached",
            Self::BarrierReleased => "barrier_released",
            Self::BarrierExpired => "barrier_expired",
            Self::TransitionPrepared => "transition_prepared",
            Self::TransitionCommitted => "transition_committed",
            Self::TransitionAborted => "transition_aborted",
            Self::TransitionDropped => "transition_dropped",
            Self::ModalAnnounced => "modal_announced",
            Self::ModalProposed => "modal_proposed",
            Self::ModalAcked => "modal_acked",
            Self::ModalExpired => "modal_expired",
            Self::Desync => "desync",
            Self::SessionFailed => "session_failed",
            Self::SessionEnded => "session_ended",
        }
    }
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
        let line = {
            let mut surface = super::surface();
            surface.events.push(kind, seat, frame, detail.clone())
        };
        let detail_text = serde_json::to_string(&detail).expect("session event detail serializes");
        match level {
            Level::WARN => tracing::warn!(
                kind = kind.as_str(),
                seat,
                frame,
                detail = %detail_text,
                "{line}"
            ),
            Level::ERROR => tracing::error!(
                kind = kind.as_str(),
                seat,
                frame,
                detail = %detail_text,
                "{line}"
            ),
            _ => tracing::info!(
                kind = kind.as_str(),
                seat,
                frame,
                detail = %detail_text,
                "{line}"
            ),
        }
    }
}

/// The bounded in-memory ring plus this run's append-only file.
#[derive(Default)]
pub(super) struct EventRing {
    next_seq: u64,
    retained: VecDeque<Value>,
    #[cfg(not(target_arch = "wasm32"))]
    file: Option<std::fs::File>,
    #[cfg(not(target_arch = "wasm32"))]
    path: Option<PathBuf>,
    /// Bytes appended to this run's file. The budget stops the appender rather
    /// than letting one runaway session fill the install's disk.
    #[cfg(not(target_arch = "wasm32"))]
    written: u64,
}

impl EventRing {
    /// Open `<directory>/events.jsonl` for this run. A previous run's file is
    /// archived first, so every `seq` in the live file belongs to one process.
    /// Browser sessions keep the ring only.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn install(&mut self, directory: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(directory)?;
        let path = directory.join(EVENT_FILE_NAME);
        archive_previous(&path)?;
        self.file = Some(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?,
        );
        self.path = Some(path);
        self.written = 0;
        Ok(())
    }

    pub(super) fn path(&self) -> Option<&Path> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.path.as_deref()
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    pub(super) fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Returns the human-readable message, which the caller also traces.
    fn push(
        &mut self,
        kind: EventKind,
        seat: Option<u8>,
        frame: Option<u32>,
        detail: Value,
    ) -> String {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        let message = detail
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let record = serde_json::json!({
            "seq": seq,
            "uptime_ms": crate::window::process_uptime_ms(),
            "utc": super::events_utc_now(),
            "kind": kind,
            "seat": seat,
            "frame": frame,
            "detail": detail,
        });
        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::io::Write as _;
            // One line, flushed: a crash keeps every event before it.
            let mut line = serde_json::to_vec(&record).expect("session event serializes");
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
            } else if let Some(file) = self.file.as_mut() {
                match file.write_all(&line).and_then(|()| file.flush()) {
                    Ok(()) => self.written += bytes,
                    Err(error) => {
                        tracing::warn!(%error, "session events: could not append to the event file");
                        self.file = None;
                    }
                }
            }
        }
        self.retained.push_back(record);
        while self.retained.len() > RING_CAPACITY {
            self.retained.pop_front();
        }
        message
    }

    /// Events with `seq >= since`, oldest first, at most `limit`.
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
            .map_or_else(|| since.max(self.next_seq), |seq| seq + 1);
        EventsPage { events, next_seq }
    }
}

/// One `/events` page. `next_seq` is the cursor to pass back as `since`.
#[derive(Debug, Serialize)]
pub(crate) struct EventsPage {
    pub(crate) events: Vec<Value>,
    pub(crate) next_seq: u64,
}

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

    #[test]
    fn ring_pages_in_order_and_stops_at_the_limit() {
        let mut ring = EventRing::default();
        for index in 0..5u32 {
            ring.push(
                EventKind::Admission,
                Some(1),
                Some(index),
                serde_json::json!({"message": format!("phase {index}"), "to": index}),
            );
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
    #[test]
    fn a_new_run_archives_the_previous_file_and_starts_its_own_sequence() {
        let home = tempfile::tempdir().unwrap();
        let mut first = EventRing::default();
        first.install(home.path()).unwrap();
        first.push(
            EventKind::SeatConnected,
            Some(0),
            None,
            serde_json::json!({"message": "previous run"}),
        );
        drop(first);

        let mut second = EventRing::default();
        second.install(home.path()).unwrap();
        second.push(
            EventKind::Admission,
            None,
            None,
            serde_json::json!({"message": "this run"}),
        );
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
        let mut ring = EventRing::default();
        ring.install(home.path()).unwrap();
        for index in 0..3u32 {
            ring.push(
                EventKind::BarrierReleased,
                Some(0),
                Some(index),
                serde_json::json!({"message": format!("release {index}")}),
            );
        }
        let text = std::fs::read_to_string(home.path().join(EVENT_FILE_NAME)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        for (expected, line) in lines.iter().enumerate() {
            let value: Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["seq"], expected as u64);
        }
    }
}
