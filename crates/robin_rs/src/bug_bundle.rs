//! Voluntary bug-report bundles: the evidence a player's report carries.
//!
//! This is the sibling of [`crate::crash`], not a second mechanism. A crash
//! bundle is written by the panic hook and uploaded by a detached child; a
//! bug-report bundle is requested by a player (the pause menu, `/bug`, or
//! `POST /bug-report`) and follows the same shape: evidence on disk first, a
//! detached upload child second, one GitHub issue per report on the receiver
//! (`tools/crash-receiver/`). The contract between the two halves is
//! `docs/BUG_REPORTS.md`.
//!
//! The one thing a crash bundle does not need and this one does is *game*
//! state. It cannot be gathered here: the engine, the session and the modal
//! stack belong to the mission thread. So a surface that a player can reach
//! only *requests* a report ([`enqueue`]) and the mission frame writes it once
//! per boundary (`game_session::report`), from the state the tick just used.
//! Writing — copying saves, replay and log tails — happens on a worker thread,
//! so a report never stalls a frame.
//!
//! [`crate::bug_report`] is a different, older queue: it uploads leaderboard
//! diagnostics to the production VPS. Nothing here touches it.

use anyhow::{Context as _, Result};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Directory under the install home holding one child per report.
pub(crate) const BUNDLE_DIRECTORY: &str = "bug-reports";
/// Hidden argument that selects the upload child: `<exe> --upload-bug-report <dir>`.
pub(crate) const UPLOAD_ARGUMENT: &str = "--upload-bug-report";
/// Environment variable naming the bug-report route on the receiver. Unset
/// means the bundle is kept on disk and nothing is uploaded.
pub(crate) const ENDPOINT_ENV: &str = "ROBIN_BUG_ENDPOINT";
/// The receiver's bearer token; the same token the crash route demands.
pub(crate) const TOKEN_ENV: &str = "ROBIN_CRASH_TOKEN";
/// A report that outgrows this is not evidence any more.
const MAX_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SAVE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_REPLAY_BYTES: u64 = 8 * 1024 * 1024;
const RUN_LOG_TAIL_LINES: usize = 200;
const EVENTS_TAIL_LINES: usize = 200;
/// The upload's whole budget; the same shape as the crash uploader's.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(5);
/// Name of the file the upload child leaves beside the bundle. The parent
/// process reads it for the status line; the file is also the only record an
/// offline attempt makes.
pub(crate) const UPLOAD_STATUS_FILE: &str = "upload-status.json";

/// How the player asked, so a report says which surface needs fixing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    Menu,
    Console,
    Http,
}

impl Origin {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Menu => "ingame-menu",
            Self::Console => "console",
            Self::Http => "http",
        }
    }
}

/// One report waiting for the next frame boundary.
#[derive(Clone)]
pub(crate) struct Request {
    pub(crate) directory: PathBuf,
    pub(crate) description: String,
    pub(crate) note: String,
    pub(crate) origin: Origin,
}

static REQUESTS: Mutex<VecDeque<Request>> = Mutex::new(VecDeque::new());
static MESSAGES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Run `action` on the queue, surviving a poisoned lock: a report is evidence,
/// and losing it because another report panicked would hide the first defect.
fn with_queue<T>(action: impl FnOnce(&mut VecDeque<Request>) -> T) -> T {
    let mut queue = REQUESTS.lock().unwrap_or_else(|error| error.into_inner());
    action(&mut queue)
}

/// Reserve the bundle directory for one report.
///
/// The directory is created immediately so the answer to a request can name
/// where the evidence will be, and so two reports cannot pick the same name.
pub(crate) fn reserve(origin: Origin) -> Result<PathBuf> {
    let root =
        crate::install_home::root().context("no install directory is available for bug reports")?;
    let directory = root.join(BUNDLE_DIRECTORY);
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("create {}", directory.display()))?;
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = directory.join(format!(
        "{}-{}-{sequence}",
        crate::crash::utc_stamp(),
        origin.as_str()
    ));
    std::fs::create_dir(&path).with_context(|| format!("create {}", path.display()))?;
    Ok(path)
}

/// Queue one report for the next frame boundary.
///
/// A description is what the issue is titled with, so an empty one is refused
/// here rather than filed as an untitled issue.
pub(crate) fn enqueue(
    directory: PathBuf,
    description: &str,
    note: &str,
    origin: Origin,
) -> Result<()> {
    let description = description.trim();
    anyhow::ensure!(!description.is_empty(), "a bug report needs a description");
    with_queue(|queue| {
        queue.push_back(Request {
            directory,
            description: crate::bug_report::bound_text(description.to_owned(), 4_000),
            note: crate::bug_report::bound_text(note.trim().to_owned(), 4_000),
            origin,
        });
    });
    Ok(())
}

/// Drain every queued request. Called once per frame by the mission loop.
pub(crate) fn take_requests() -> Vec<Request> {
    with_queue(|queue| queue.drain(..).collect())
}

/// Whether an upload destination exists for this process.
pub(crate) fn endpoint_configured() -> bool {
    std::env::var(ENDPOINT_ENV)
        .ok()
        .is_some_and(|endpoint| !endpoint.trim().is_empty())
}

fn endpoint() -> Option<String> {
    std::env::var(ENDPOINT_ENV)
        .ok()
        .map(|endpoint| endpoint.trim().to_owned())
        .filter(|endpoint| !endpoint.is_empty())
}

/// Status lines for the player: the console scrollback and the report screen
/// drain these, so a failure is never silent.
pub(crate) fn message(text: &str) {
    tracing::info!("{text}");
    let mut messages = MESSAGES.lock().unwrap_or_else(|error| error.into_inner());
    if messages.len() >= 20 {
        messages.pop_front();
    }
    messages.push_back(text.to_owned());
}

pub(crate) fn take_messages() -> Vec<String> {
    let mut messages = MESSAGES.lock().unwrap_or_else(|error| error.into_inner());
    messages.drain(..).collect()
}

/// Write one report's bundle on a worker thread and return its directory.
///
/// `snapshot` is the game-state document gathered on the mission thread at the
/// frame boundary; the thread adds the process-wide evidence (run log, session
/// events, `/state`), copies the save directory and the replay, and then starts
/// the upload child. A failure leaves the partial bundle and is reported in the
/// console, never as a panic inside a report.
pub(crate) fn write(request: Request, snapshot: Value) -> PathBuf {
    let directory = request.directory.clone();
    let report_directory = directory.clone();
    let state = crate::observability::state_document_if_unlocked();
    let upload_endpoint = endpoint();
    let fallback_request = request.clone();
    let fallback_snapshot = snapshot.clone();
    let spawned = std::thread::Builder::new()
        .name("bug-report".into())
        .spawn(move || {
            match write_bundle(&report_directory, &request, snapshot, state) {
                Ok(()) => message(&format!(
                    "Bug report saved: {}",
                    report_directory.display()
                )),
                Err(error) => message(&format!(
                    "Bug report could not be written to {}: {error:#}",
                    report_directory.display()
                )),
            }
            if let Some(endpoint) = upload_endpoint {
                match crate::crash::spawn_upload_child(UPLOAD_ARGUMENT, &report_directory) {
                    Ok(()) => message(&format!(
                        "Bug report is uploading to {endpoint}; if that fails the bundle stays at {}",
                        report_directory.display()
                    )),
                    Err(error) => message(&format!(
                        "Bug report upload could not start ({error:#}); the bundle is at {}",
                        report_directory.display()
                    )),
                }
            } else {
                message(&format!(
                    "{ENDPOINT_ENV} is not set, so the bug report was not uploaded; the bundle is at {}",
                    report_directory.display()
                ));
            }
        });
    if let Err(error) = spawned {
        // No worker: write what we can on this thread rather than lose the
        // evidence. This is the only path that touches the frame.
        message(&format!(
            "Bug report worker unavailable ({error:#}); writing the bundle inline"
        ));
        match write_bundle(
            &directory,
            &fallback_request,
            fallback_snapshot,
            crate::observability::state_document_if_unlocked(),
        ) {
            Ok(()) => message(&format!("Bug report saved: {}", directory.display())),
            Err(error) => message(&format!(
                "Bug report could not be written to {}: {error:#}",
                directory.display()
            )),
        }
    }
    directory
}

/// Compose the one `cursor` section from the two raw facts the frame boundary
/// left in the snapshot, then drop those keys: the section is the single source
/// for what was under the cursor. A fact that was not captured is named rather
/// than omitted, the same rule every other section follows.
fn cursor_section(snapshot: &mut serde_json::Map<String, Value>) -> Value {
    let sample = snapshot.remove("cursor_sample");
    let query = snapshot.remove("cursor_element");
    let sampled = sample.as_ref().is_some_and(Value::is_object);
    let sample = match sample {
        Some(sample) if sampled => sample,
        Some(_) => json!({
            "available": false,
            "reason": "cursor_sample was not an object at the frame boundary"
        }),
        None => json!({
            "available": false,
            "reason": "no cursor_sample was captured at the frame boundary"
        }),
    };
    let mut section = json!({
        "sample": sample,
        "coordinates": "logical game-surface pixels, the space the HUD hit tests use",
        "element": Value::Null,
        "miss": {
            "reason": if sampled {
                "no cursor_element query was captured at the frame boundary"
            } else {
                "no cursor sample was captured, so no layer could be tested"
            }
        },
        "layers_tested": Vec::<Value>::new(),
        "active_modal": Value::Null,
        "terminal_page": {"active": false},
    });
    if let Some(query) = query.filter(Value::is_object) {
        if let Some(element) = query.get("element").filter(|value| !value.is_null()) {
            section["element"] = element.clone();
        }
        if let Some(miss) = query.get("miss").filter(|value| !value.is_null()) {
            section["miss"] = miss.clone();
        }
        if let Some(layers) = query.get("layers_tested") {
            section["layers_tested"] = layers.clone();
        }
        if let Some(modal) = query.get("active_modal") {
            section["active_modal"] = modal.clone();
        }
        if let Some(page) = query.get("terminal_page") {
            section["terminal_page"] = page.clone();
        }
    }
    section
}

/// The bundle writer, also called directly by tests.
fn write_bundle(
    directory: &Path,
    request: &Request,
    mut snapshot: Value,
    state: Option<Value>,
) -> Result<()> {
    std::fs::create_dir_all(directory)
        .with_context(|| format!("create {}", directory.display()))?;
    let mut notes = Vec::new();
    let mut files = json!({"saves": [], "replay": Vec::<Value>::new()});

    if let Some(state) = state.as_ref() {
        crate::crash::write_json(&directory.join("state.json"), state)?;
    }
    let run_log_tail = match crate::crash::evidence_path(&state, "trace_log") {
        None => {
            notes.push("no run log path in /state".to_owned());
            None
        }
        Some(path) => match crate::crash::tail(&path, RUN_LOG_TAIL_LINES) {
            Ok(tail) => Some(tail),
            Err(error) => {
                notes.push(format!("run log tail unavailable: {error:#}"));
                None
            }
        },
    };
    let events_tail = match crate::crash::evidence_path(&state, "event_log") {
        None => {
            notes.push("no session event log path in /state".to_owned());
            None
        }
        Some(path) => match crate::crash::tail(&path, EVENTS_TAIL_LINES) {
            Ok(tail) => Some(tail),
            Err(error) => {
                notes.push(format!("events tail unavailable: {error:#}"));
                None
            }
        },
    };

    let save_directory = snapshot
        .get("paths")
        .and_then(|paths| paths.get("save_directory"))
        .and_then(Value::as_str)
        .map(PathBuf::from);
    let saved = match save_directory.as_deref() {
        None => {
            notes.push(
                "no save directory is known for this profile, so no save was copied".to_owned(),
            );
            Vec::new()
        }
        Some(save_directory) => copy_saves(directory, save_directory, &mut notes)?,
    };
    files["saves"] = json!(saved);

    let replay_path = snapshot
        .get("paths")
        .and_then(|paths| paths.get("replay"))
        .and_then(Value::as_str)
        .map(PathBuf::from);
    let replay = match replay_path.as_deref() {
        None => {
            notes.push("no recording was active, so no replay was copied".to_owned());
            Vec::new()
        }
        Some(path) => copy_replay(directory, path, &mut notes)?,
    };
    files["replay"] = json!(replay);

    let object = snapshot
        .as_object_mut()
        .context("the game-state snapshot is not a JSON object")?;
    object.insert("origin".into(), json!(request.origin.as_str()));
    object.insert("description".into(), json!(request.description));
    object.insert("note".into(), json!(request.note));
    object.insert("captured_at".into(), json!(crate::crash::utc_stamp()));
    object.insert(
        "install_home".into(),
        json!(crate::install_home::root().map(|root| root.to_string_lossy().into_owned())),
    );
    object.insert("files".into(), files);
    let cursor = cursor_section(object);
    object.insert("cursor".into(), cursor);
    object.insert(
        "upload".into(),
        match endpoint() {
            Some(endpoint) => json!({"endpoint": endpoint, "attempted": false}),
            None => json!({
                "endpoint": null,
                "reason": format!(
                    "{ENDPOINT_ENV} is not set; nothing was uploaded and the bundle stays here"
                ),
            }),
        },
    );
    if let Some(tail) = run_log_tail {
        object.insert("run_log_tail".into(), json!(tail));
    }
    if let Some(tail) = events_tail {
        object.insert("events_tail".into(), json!(tail));
    }
    if let Some(state) = state {
        object.insert("state".into(), state);
    }
    let mut all_notes: Vec<Value> = object
        .remove("notes")
        .and_then(|notes| notes.as_array().cloned())
        .unwrap_or_default();
    all_notes.extend(notes.into_iter().map(Value::String));
    object.insert("notes".into(), Value::Array(all_notes));

    let bytes = serde_json::to_vec_pretty(&snapshot)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_BUNDLE_BYTES,
        "the bundle is {} bytes, over the {MAX_BUNDLE_BYTES}-byte limit",
        bytes.len()
    );
    crate::crash::write(&directory.join("bundle.json"), &bytes)?;
    Ok(())
}

/// Copy a profile's save directory into the bundle, bounded.
///
/// Files are named relative to the save directory, so a reader can rebuild the
/// layout; each entry records its size. Nothing is silently dropped: every skip
/// and every truncation is a note.
fn copy_saves(
    directory: &Path,
    save_directory: &Path,
    notes: &mut Vec<String>,
) -> Result<Vec<Value>> {
    let target = directory.join("saves");
    let mut copied = Vec::new();
    let mut budget = MAX_SAVE_BYTES;
    let mut entries: Vec<PathBuf> = std::fs::read_dir(save_directory)
        .with_context(|| format!("read {}", save_directory.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for source in entries {
        if !source.is_file() {
            notes.push(format!(
                "save entry {} is not a file; not copied",
                source.display()
            ));
            continue;
        }
        let name = source
            .file_name()
            .context("a save file without a name")?
            .to_owned();
        let length = std::fs::metadata(&source)?.len();
        if length > budget {
            notes.push(format!(
                "save file {name:?} is {length} bytes and the remaining copy budget is {budget}; not copied",
                name = name.to_string_lossy()
            ));
            continue;
        }
        std::fs::create_dir_all(&target)?;
        std::fs::copy(&source, target.join(&name))
            .with_context(|| format!("copy {}", source.display()))?;
        budget -= length;
        copied.push(json!({
            "file": name.to_string_lossy(),
            "bytes": length,
        }));
    }
    if copied.is_empty() {
        notes.push(format!(
            "{} contained no regular files; no save was copied",
            save_directory.display()
        ));
    }
    Ok(copied)
}

/// Copy the active recording, bounded, into the bundle.
///
/// A native recording is one JSONL file; a restart recording is a directory of
/// chunks. Both are handled, and every truncation is recorded rather than
/// passed off as the whole replay.
fn copy_replay(directory: &Path, replay: &Path, notes: &mut Vec<String>) -> Result<Vec<Value>> {
    let target = directory.join("replay");
    std::fs::create_dir_all(&target)?;
    let mut copied = Vec::new();
    let mut budget = MAX_REPLAY_BYTES;
    let sources: Vec<PathBuf> = if replay.is_dir() {
        let mut sources: Vec<PathBuf> = std::fs::read_dir(replay)
            .with_context(|| format!("read {}", replay.display()))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == "mission.json" || name.ends_with(".rhrec.jsonl"))
            })
            .collect();
        sources.sort();
        sources
    } else {
        vec![replay.to_owned()]
    };
    for source in sources {
        if !source.is_file() {
            notes.push(format!(
                "replay entry {} is not a file; not copied",
                source.display()
            ));
            continue;
        }
        let name = source
            .file_name()
            .context("a replay file without a name")?
            .to_owned();
        let length = std::fs::metadata(&source)?.len();
        let take = length.min(budget);
        if take == 0 {
            notes.push(format!(
                "replay file {name:?} left out: the {MAX_REPLAY_BYTES}-byte replay budget is spent",
                name = name.to_string_lossy()
            ));
            continue;
        }
        copy_tail(&source, &target.join(&name), take)?;
        budget -= take;
        let truncation = if take < length {
            notes.push(format!(
                "replay file {name:?} is {length} bytes; the last {take} bytes were copied",
                name = name.to_string_lossy()
            ));
            json!(format!("last {take} of {length} bytes copied"))
        } else {
            Value::Null
        };
        copied.push(json!({
            "file": name.to_string_lossy(),
            "bytes": take,
            "truncation": truncation,
        }));
    }
    if copied.is_empty() {
        notes.push(format!(
            "the recording at {} held no replay file; nothing was copied",
            replay.display()
        ));
    }
    Ok(copied)
}

/// Copy the last `bytes` of `source` to `target`, so a huge recording still
/// contributes its most recent — and most relevant — frames.
fn copy_tail(source: &Path, target: &Path, bytes: u64) -> Result<()> {
    use std::io::{Seek as _, SeekFrom};
    let mut file =
        std::fs::File::open(source).with_context(|| format!("open {}", source.display()))?;
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(bytes)))?;
    let mut out =
        std::fs::File::create(target).with_context(|| format!("create {}", target.display()))?;
    std::io::copy(&mut file, &mut out).with_context(|| format!("copy {}", source.display()))?;
    Ok(())
}

/// POST one bundle and return the process exit status.
///
/// Called only in the `--upload-bug-report` child, which the writer thread
/// starts detached so a game that quits first cannot cancel the upload. The
/// child writes [`UPLOAD_STATUS_FILE`] beside the bundle and exits non-zero on
/// any failure; the bundle is never removed by an upload attempt.
pub fn upload_bundle(directory: &Path) -> i32 {
    match upload(directory) {
        Ok(()) => 0,
        Err(error) => {
            let status = json!({
                "ok": false,
                "at": crate::crash::utc_stamp(),
                "error": format!("{error:#}"),
            });
            let _ = crate::crash::write_json(&directory.join(UPLOAD_STATUS_FILE), &status);
            #[allow(clippy::print_stderr)]
            {
                // The child has no console and no tracing subscriber.
                eprintln!("[robin] bug report upload failed: {error:#}");
            }
            1
        }
    }
}

fn upload(directory: &Path) -> Result<()> {
    let endpoint = endpoint().with_context(|| format!("{ENDPOINT_ENV} is not set"))?;
    let body = std::fs::read(directory.join("bundle.json"))
        .with_context(|| format!("read {}/bundle.json", directory.display()))?;
    // `reqwest` is built with `rustls-no-provider`; installing the provider
    // twice is harmless.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::blocking::Client::builder()
        .timeout(UPLOAD_TIMEOUT)
        .connect_timeout(UPLOAD_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut request = client
        .post(endpoint.clone())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body);
    if let Ok(token) = std::env::var(TOKEN_ENV) {
        let token = token.trim();
        if !token.is_empty() {
            request = request.bearer_auth(token);
        }
    }
    let response = request.send().with_context(|| format!("POST {endpoint}"))?;
    let status = response.status();
    let answer = response.text().unwrap_or_default();
    anyhow::ensure!(
        status.is_success(),
        "receiver answered HTTP {status}: {}",
        crate::bug_report::bound_text(answer, 500)
    );
    let receipt = json!({
        "ok": true,
        "at": crate::crash::utc_stamp(),
        "status": status.as_u16(),
        "answer": crate::bug_report::bound_text(answer, 2_000),
    });
    crate::crash::write_json(&directory.join(UPLOAD_STATUS_FILE), &receipt)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Environment variables are process-global; every test that sets one holds
    /// the install-home lock the rest of the crate already serializes on.
    fn with_install_home(home: &Path, clear_endpoint: bool) -> std::sync::MutexGuard<'static, ()> {
        let guard = crate::install_home::ENV_LOCK.lock().unwrap();
        // Safety: the lock above serializes every reader and writer.
        unsafe {
            std::env::set_var("ROBINHOOD_INSTALL_HOME", home);
            if clear_endpoint {
                std::env::remove_var(ENDPOINT_ENV);
            }
        }
        guard
    }

    fn request(directory: PathBuf) -> Request {
        Request {
            directory,
            description: "standing on the Sherwood sign, no return message".into(),
            note: "mission completed".into(),
            origin: Origin::Menu,
        }
    }

    /// The sections a report must carry are present, and the unavailable ones
    /// name why instead of disappearing.
    #[test]
    fn a_bundle_carries_the_game_state_the_saves_and_the_tails() {
        let home = tempfile::tempdir().unwrap();
        let _guard = with_install_home(home.path(), true);
        let saves = home.path().join("saves/Profile_001");
        std::fs::create_dir_all(&saves).unwrap();
        std::fs::write(saves.join("Savegame_002.json"), "{\"campaign\":1}").unwrap();
        std::fs::write(saves.join("quick.sav"), "quick").unwrap();

        let log = home.path().join("logs/robin.log");
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(&log, "line one\nline two\n").unwrap();
        let events = home.path().join("logs/events.jsonl");
        std::fs::write(&events, "{\"seq\":1}\n").unwrap();
        let state = json!({
            "frame": 133,
            "observability": {
                "trace_log": log.display().to_string(),
                "event_log": events.display().to_string(),
            },
        });
        let replay = home.path().join("replays/run.rhrec.jsonl");
        std::fs::create_dir_all(replay.parent().unwrap()).unwrap();
        std::fs::write(&replay, "{\"frame\":1}\n").unwrap();

        let snapshot = json!({
            "schema_version": 1,
            "fingerprint": "0123456789abcdef",
            "description": "ignored: the request owns it",
            "build": {"version": crate::crash::version(), "release": false},
            "session": {"kind": "single", "tick": 133, "mission_id": "Emb01"},
            "game": {"operation": "level_in_progress", "script": {"globals": {"mission_won": 0}}},
            "world": {"player_characters": [{"id": 7, "position": [1.0, 2.0]}]},
            "paths": {
                "save_directory": saves.display().to_string(),
                "replay": replay.display().to_string(),
            },
            "notes": ["seats are reported by the session block"],
        });
        let directory = home.path().join("bug-reports/one");
        write_bundle(
            &directory,
            &request(directory.clone()),
            snapshot,
            Some(state),
        )
        .unwrap();

        let bundle: Value =
            serde_json::from_slice(&std::fs::read(directory.join("bundle.json")).unwrap()).unwrap();
        assert_eq!(bundle["origin"], "ingame-menu");
        assert_eq!(bundle["session"]["tick"], 133);
        assert_eq!(bundle["session"]["kind"], "single");
        assert_eq!(bundle["game"]["script"]["globals"]["mission_won"], 0);
        assert_eq!(bundle["game"]["operation"], "level_in_progress");
        assert_eq!(bundle["world"]["player_characters"][0]["id"], 7);
        assert_eq!(
            bundle["description"],
            request(directory.clone()).description
        );
        let saves_written = bundle["files"]["saves"].as_array().unwrap();
        assert_eq!(saves_written.len(), 2, "every save file is listed");
        assert!(directory.join("saves/Savegame_002.json").exists());
        assert!(directory.join("saves/quick.sav").exists());
        let replay_written = bundle["files"]["replay"].as_array().unwrap();
        assert_eq!(replay_written.len(), 1);
        assert!(directory.join("replay/run.rhrec.jsonl").exists());
        assert!(directory.join("state.json").exists());
        assert!(
            bundle["run_log_tail"]
                .as_str()
                .unwrap()
                .contains("line two")
        );
        assert!(
            bundle["events_tail"]
                .as_str()
                .unwrap()
                .contains("\"seq\":1")
        );
        assert!(
            bundle["notes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|note| note.as_str().unwrap().contains("seats are reported")),
            "the gatherer's notes survive the writer's own"
        );
        assert!(
            bundle["upload"]["endpoint"].is_null(),
            "no endpoint means no upload claim"
        );
        assert!(
            bundle["upload"]["reason"]
                .as_str()
                .unwrap()
                .contains(ENDPOINT_ENV)
        );
    }

    /// A report with nothing to copy says so; a silent empty section would read
    /// as "there were no saves", which is a different fact.
    #[test]
    fn missing_saves_replay_and_logs_are_named_in_the_notes() {
        let home = tempfile::tempdir().unwrap();
        let _guard = with_install_home(home.path(), true);
        let snapshot = json!({
            "schema_version": 1,
            "fingerprint": "0123456789abcdef",
            "paths": {"save_directory": null, "replay": null},
            "notes": [],
        });
        let directory = home.path().join("bug-reports/two");
        write_bundle(&directory, &request(directory.clone()), snapshot, None).unwrap();
        let bundle: Value =
            serde_json::from_slice(&std::fs::read(directory.join("bundle.json")).unwrap()).unwrap();
        let notes: Vec<&str> = bundle["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|note| note.as_str().unwrap())
            .collect();
        for expected in [
            "no save directory is known",
            "no recording was active",
            "no run log path in /state",
            "no session event log path in /state",
        ] {
            assert!(
                notes.iter().any(|note| note.contains(expected)),
                "{expected:?} is one of {notes:?}"
            );
        }
    }

    /// The child is the only network actor. With no endpoint it must refuse,
    /// exit non-zero, and leave the bundle where it was.
    #[test]
    fn no_endpoint_keeps_the_bundle_and_the_child_fails() {
        let home = tempfile::tempdir().unwrap();
        let _guard = with_install_home(home.path(), true);
        let snapshot = json!({
            "schema_version": 1,
            "fingerprint": "0123456789abcdef",
            "paths": {"save_directory": null, "replay": null},
            "notes": [],
        });
        let directory = home.path().join("bug-reports/three");
        write_bundle(&directory, &request(directory.clone()), snapshot, None).unwrap();
        assert_ne!(upload_bundle(&directory), 0, "no endpoint is a failure");
        assert!(
            directory.join("bundle.json").exists(),
            "the bundle survives"
        );
        let status: Value =
            serde_json::from_slice(&std::fs::read(directory.join(UPLOAD_STATUS_FILE)).unwrap())
                .unwrap();
        assert_eq!(status["ok"], false);
        assert!(status["error"].as_str().unwrap().contains(ENDPOINT_ENV));
    }

    #[test]
    fn a_reserved_directory_is_unique_and_named_for_its_origin() {
        let home = tempfile::tempdir().unwrap();
        let _guard = with_install_home(home.path(), true);
        let first = reserve(Origin::Console).unwrap();
        let second = reserve(Origin::Console).unwrap();
        assert_ne!(first, second);
        assert!(first.is_dir() && second.is_dir());
        assert!(
            first
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("-console-")
        );
    }

    #[test]
    fn an_empty_description_is_refused_before_it_becomes_an_untitled_issue() {
        let home = tempfile::tempdir().unwrap();
        let _guard = with_install_home(home.path(), true);
        assert!(enqueue(home.path().join("x"), "   ", "", Origin::Http).is_err());
        assert!(
            enqueue(
                home.path().join("x"),
                "stuck on the sign",
                "note",
                Origin::Http
            )
            .is_ok()
        );
        let queued = take_requests();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].description, "stuck on the sign");
    }

    /// A synthetic frame with a known widget. The raw frame-boundary facts are
    /// composed by the writer into one `cursor` section, off the frame loop.
    #[test]
    fn a_bundle_names_the_widget_under_the_cursor() {
        let home = tempfile::tempdir().unwrap();
        let _guard = with_install_home(home.path(), true);
        let snapshot = json!({
            "schema_version": 1,
            "fingerprint": "0123456789abcdef",
            "paths": {"save_directory": null, "replay": null},
            "notes": [],
            "cursor_sample": {
                "x": 617.0, "y": 5.0,
                "surface_w": 640, "surface_h": 480, "scale": 2.0,
            },
            "cursor_element": {
                "element": {
                    "id": "zoom.up",
                    "name": "Zoom in",
                    "parent_layer": "zoom_hud",
                    "rect": {"x": 614, "y": 0, "w": 24, "h": 24},
                    "label": "Zoom in",
                    "enabled": true,
                },
                "miss": null,
                "layers_tested": ["touch_plan_hud", "zoom_hud"],
                "active_modal": {"kind": "dialogue"},
                "terminal_page": {"active": false},
            },
        });
        let directory = home.path().join("bug-reports/cursor-widget");
        write_bundle(&directory, &request(directory.clone()), snapshot, None).unwrap();
        let bundle: Value =
            serde_json::from_slice(&std::fs::read(directory.join("bundle.json")).unwrap()).unwrap();
        assert_eq!(bundle["cursor"]["element"]["name"], "Zoom in");
        assert_eq!(bundle["cursor"]["element"]["label"], "Zoom in");
        assert_eq!(bundle["cursor"]["element"]["parent_layer"], "zoom_hud");
        assert_eq!(bundle["cursor"]["element"]["rect"]["x"], 614);
        assert_eq!(bundle["cursor"]["sample"]["scale"], 2.0);
        assert_eq!(bundle["cursor"]["active_modal"]["kind"], "dialogue");
        assert_eq!(bundle["cursor"]["terminal_page"]["active"], false);
        assert!(
            bundle.get("cursor_sample").is_none() && bundle.get("cursor_element").is_none(),
            "the raw frame-boundary facts are replaced by the one cursor section"
        );
        assert!(
            bundle["cursor"]["coordinates"]
                .as_str()
                .is_some_and(|text| text.contains("logical")),
            "the coordinate space is named for the reader"
        );
    }

    /// Empty space is a positive fact: the miss record names every layer the
    /// query tested, so it cannot be read as "nobody looked".
    #[test]
    fn an_empty_space_cursor_miss_lists_every_layer_tested() {
        let home = tempfile::tempdir().unwrap();
        let _guard = with_install_home(home.path(), true);
        let snapshot = json!({
            "schema_version": 1,
            "fingerprint": "0123456789abcdef",
            "paths": {"save_directory": null, "replay": null},
            "notes": [],
            "cursor_sample": {
                "x": 320.0, "y": 240.0,
                "surface_w": 640, "surface_h": 480, "scale": 1.0,
            },
            "cursor_element": {
                "element": null,
                "miss": {"reason": "no UI element under the cursor"},
                "layers_tested": [
                    "touch_plan_hud", "zoom_hud", "corner_hud",
                    "stature_hud", "sherwood_hud", "ui_panel",
                ],
                "active_modal": null,
                "terminal_page": {"active": false},
            },
        });
        let directory = home.path().join("bug-reports/cursor-miss");
        write_bundle(&directory, &request(directory.clone()), snapshot, None).unwrap();
        let bundle: Value =
            serde_json::from_slice(&std::fs::read(directory.join("bundle.json")).unwrap()).unwrap();
        assert!(bundle["cursor"]["element"].is_null());
        assert_eq!(
            bundle["cursor"]["miss"]["reason"],
            "no UI element under the cursor"
        );
        let layers = bundle["cursor"]["layers_tested"]
            .as_array()
            .expect("the miss lists the layers it tested");
        for name in [
            "touch_plan_hud",
            "zoom_hud",
            "corner_hud",
            "stature_hud",
            "sherwood_hud",
            "ui_panel",
        ] {
            assert!(
                layers.iter().any(|layer| layer == name),
                "{name} is one of {layers:?}"
            );
        }
    }

    /// A snapshot with no cursor facts still writes a section that says why,
    /// instead of leaving the reader to guess at a missing key.
    #[test]
    fn a_missing_cursor_sample_is_named_instead_of_silently_dropped() {
        let home = tempfile::tempdir().unwrap();
        let _guard = with_install_home(home.path(), true);
        let snapshot = json!({
            "schema_version": 1,
            "fingerprint": "0123456789abcdef",
            "paths": {"save_directory": null, "replay": null},
            "notes": [],
        });
        let directory = home.path().join("bug-reports/cursor-absent");
        write_bundle(&directory, &request(directory.clone()), snapshot, None).unwrap();
        let bundle: Value =
            serde_json::from_slice(&std::fs::read(directory.join("bundle.json")).unwrap()).unwrap();
        assert_eq!(bundle["cursor"]["sample"]["available"], false);
        assert!(
            bundle["cursor"]["sample"]["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("cursor_sample")),
            "the absent raw fact is named"
        );
        assert!(bundle["cursor"]["element"].is_null());
        assert_eq!(bundle["cursor"]["layers_tested"], json!([]));
    }
}
