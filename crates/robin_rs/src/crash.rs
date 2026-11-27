//! Crash bundles: one durable record per panic, uploaded by a child process.
//!
//! A crash is worthless unless it leaves enough behind to be fixed, and a
//! player will never copy a log by hand. A panic therefore writes a bundle
//! under `<install home>/crashes/<utc>-<fingerprint>/` **before** any upload,
//! so an offline crash still leaves evidence, and then hands that directory to
//! a detached child (`--upload-crash <dir>`) which POSTs it and exits. The
//! dying process never waits on the network.
//!
//! The hook chains to the previously installed hook, so the panic message on
//! stderr and the existing diagnostic queue are unchanged. Nothing here may
//! panic: the whole bundle write runs under [`std::panic::catch_unwind`], and
//! every failure path falls back to a marker file rather than a second panic
//! (which aborts the process).
//!
//! The endpoint, the receiver and the dedupe rule are documented in
//! `docs/CRASHES.md`.
//!
//! TODO: fatal signals (SIGSEGV/SIGABRT) arrive on an alternate stack when the
//! allocator may already be unusable, so this module's `Path`/JSON code cannot
//! run there; the binary's signal handler still writes only a fixed message.
//! Capturing a bundle on a signal needs an async-signal-safe writer with a
//! preallocated buffer and `write(2)`.

use anyhow::{Context as _, Result};
use serde_json::{Value, json};
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// Directory under the install home holding one child per crash.
const BUNDLE_DIRECTORY: &str = "crashes";
/// Hidden argument that selects the upload child: `<exe> --upload-crash <dir>`.
const UPLOAD_ARGUMENT: &str = "--upload-crash";
/// Environment variable naming the receiver. Unset means local-only.
const ENDPOINT_ENV: &str = "ROBIN_CRASH_ENDPOINT";
/// Bearer token the receiver demands. Sent only when set, so a self-hosted
/// receiver that requires no authentication keeps working.
const TOKEN_ENV: &str = "ROBIN_CRASH_TOKEN";
/// How much of a log file the tail is read from. A run log is evidence, not an
/// archive; the hook must not read a hundred megabytes inside a panic.
const TAIL_BYTES: u64 = 512 * 1024;
const RUN_LOG_TAIL_LINES: usize = 200;
const EVENTS_TAIL_LINES: usize = 50;
/// The upload child's whole budget: a crash report that arrives late is worth
/// less than one that blocks a shutdown.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(5);

/// What [`install`] resolved once, at startup. The hook reads it through
/// [`installed`], so a panic never has to touch the environment.
struct Install {
    install_home: PathBuf,
    endpoint: Option<String>,
}

static INSTALL: Mutex<Option<Install>> = Mutex::new(None);
static HOOK: std::sync::Once = std::sync::Once::new();

/// Install the crash hook for this process.
///
/// `install_home` is the per-install root resolved once at startup
/// (`install_home::root`); bundles land in its `crashes/` child. Calling this
/// again updates the context and leaves the hook chain alone, so one panic can
/// never write two bundles.
pub fn install(install_home: &Path) {
    // TODO: the owner asked for `crash_report.endpoint` in the profile config as
    // a second source. The profile domain (`robin_engine::player_profile`) has
    // no such setting, so the environment is the only source; add the setting
    // there when it exists instead of inventing one here.
    let endpoint = std::env::var(ENDPOINT_ENV)
        .ok()
        .map(|endpoint| endpoint.trim().to_owned())
        .filter(|endpoint| !endpoint.is_empty());
    {
        let mut install = INSTALL.lock().unwrap_or_else(|error| error.into_inner());
        *install = Some(Install {
            install_home: install_home.to_owned(),
            endpoint,
        });
    }
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            capture(info);
            // stderr output, the diagnostic queue and the exit status stay the
            // previous hook's business.
            previous(info);
        }));
    });
}

fn installed() -> Option<Install> {
    let install = INSTALL.lock().unwrap_or_else(|error| error.into_inner());
    install.as_ref().map(|install| Install {
        install_home: install.install_home.clone(),
        endpoint: install.endpoint.clone(),
    })
}

/// Version line of this binary: the package version plus the commit the build
/// script stamped in, the same string `--version` prints.
pub(crate) fn version() -> &'static str {
    crate::version::cli_version()
}

/// The panic-hook body: bundle first, upload second, nothing may unwind.
///
/// Nothing here logs: a panic can happen while this thread holds the run log's
/// writer, and a second lock on it would turn a crash into a hang. Whatever
/// could not be gathered is named in `bundle.json`'s `notes` instead.
fn capture(info: &std::panic::PanicHookInfo<'_>) {
    let Some(install) = installed() else {
        return;
    };
    let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        write_bundle(&install, info)
    }));
    let directory = match written {
        Ok(Ok(directory)) => directory,
        Ok(Err(error)) => {
            fallback_marker(&install, info, &format!("{error:#}"));
            return;
        }
        Err(_) => {
            fallback_marker(&install, info, "the bundle writer panicked");
            return;
        }
    };
    if install.endpoint.is_some() {
        spawn_uploader(&directory);
    }
}

/// Last resort when the bundle itself could not be written: one line naming
/// the panic, beside the bundles.
fn fallback_marker(install: &Install, info: &std::panic::PanicHookInfo<'_>, reason: &str) {
    let directory = install.install_home.join(BUNDLE_DIRECTORY);
    let _ = std::fs::create_dir_all(&directory);
    let text = format!(
        "{} {} {}: {} ({reason})\n",
        utc_stamp(),
        location(info).unwrap_or_else(|| "<unknown location>".into()),
        version(),
        panic_message(info),
    );
    let _ = std::fs::write(directory.join(format!("{}-marker.txt", utc_stamp())), text);
}

/// `file:line:column` of the panic, the panic hook's own location format.
fn location(info: &std::panic::PanicHookInfo<'_>) -> Option<String> {
    info.location().map(|location| {
        format!(
            "{}:{}:{}",
            location.file(),
            location.line(),
            location.column()
        )
    })
}

fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(message) = info.payload().downcast_ref::<&str>() {
        return (*message).to_owned();
    }
    if let Some(message) = info.payload().downcast_ref::<String>() {
        return message.clone();
    }
    "<panic payload is not a string>".to_owned()
}

/// Everything the bundle carries besides the panic itself, gathered before
/// anything is written so `bundle.json` can say what is missing and why.
struct Evidence {
    state: Option<Value>,
    run_log_tail: Option<String>,
    events_tail: Option<String>,
    replay: Option<PathBuf>,
    notes: Vec<String>,
}

fn gather() -> Evidence {
    let mut notes = Vec::new();
    // The hook runs inside a panic: a surface lock the panicking thread holds
    // must not turn a crash into a hang.
    let state = crate::observability::state_document_if_unlocked();
    if state.is_none() {
        notes.push("no /state document: none published yet, or its lock is held".into());
    }
    let run_log_tail = match evidence_path(&state, "trace_log") {
        None => {
            notes.push("no run log path in /state".into());
            None
        }
        Some(path) => match tail(&path, RUN_LOG_TAIL_LINES) {
            Ok(tail) => Some(tail),
            Err(error) => {
                notes.push(format!("run log tail unavailable: {error:#}"));
                None
            }
        },
    };
    let events_tail = match evidence_path(&state, "event_log") {
        None => {
            notes.push("no session event log path in /state".into());
            None
        }
        Some(path) => match tail(&path, EVENTS_TAIL_LINES) {
            Ok(tail) => Some(tail),
            Err(error) => {
                notes.push(format!("events tail unavailable: {error:#}"));
                None
            }
        },
    };
    Evidence {
        state,
        run_log_tail,
        events_tail,
        replay: crate::bug_report::replay_path(),
        notes,
    }
}

/// The evidence file the `/state` document names under `observability`, if it
/// names one. `/state` writes `null` when a path does not exist.
pub(crate) fn evidence_path(document: &Option<Value>, key: &str) -> Option<PathBuf> {
    let path = document.as_ref()?.get("observability")?.get(key)?;
    path.as_str().map(PathBuf::from)
}

/// The last `lines` whole lines of `path`, read from a bounded tail.
///
/// Whole-file reads are pointless here: the last lines are exactly where the
/// crash is, and the hook must stay fast and allocation-bounded.
pub(crate) fn tail(path: &Path, lines: usize) -> Result<String> {
    let mut file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let length = file.metadata()?.len();
    let start = length.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut bytes)?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        // The read may begin mid-line; drop that partial line so the tail is
        // whole log lines.
        if let Some(newline) = text.find('\n') {
            text.drain(..=newline);
        }
    }
    let mut tail: Vec<&str> = text.lines().rev().take(lines).collect();
    tail.reverse();
    let mut joined = tail.join("\n");
    joined.push('\n');
    Ok(joined)
}

/// Write the bundle and return its directory.
fn write_bundle(install: &Install, info: &std::panic::PanicHookInfo<'_>) -> Result<PathBuf> {
    let message = panic_message(info);
    let location = location(info);
    let fingerprint = fingerprint_of(location.as_deref(), version());
    let captured_at = utc_stamp();
    let directory = install
        .install_home
        .join(BUNDLE_DIRECTORY)
        .join(format!("{captured_at}-{fingerprint}"));
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("create {}", directory.display()))?;

    let evidence = gather();
    let document = json!({
        "schema_version": 1,
        "fingerprint": fingerprint,
        "message": message,
        "location": location,
        "backtrace": std::backtrace::Backtrace::force_capture().to_string(),
        "version": version(),
        "engine_commit": crate::replay_format::engine_source_commit(),
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "install_home": install.install_home.to_string_lossy(),
        "replay_path": evidence.replay.as_ref().map(|path| path.display().to_string()),
        "captured_at": captured_at,
        "notes": evidence.notes,
    });
    write_json(&directory.join("bundle.json"), &document)?;
    if let Some(state) = &evidence.state {
        write_json(&directory.join("state.json"), state)?;
    }
    if let Some(tail) = &evidence.run_log_tail {
        write(&directory.join("run-log-tail.txt"), tail.as_bytes())?;
    }
    if let Some(tail) = &evidence.events_tail {
        write(&directory.join("events-tail.jsonl"), tail.as_bytes())?;
    }
    if let Some(replay) = &evidence.replay {
        write(
            &directory.join("replay-path.txt"),
            format!("{}\n", replay.display()).as_bytes(),
        )?;
    }
    Ok(directory)
}

pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).with_context(|| format!("write {}", path.display()))
}

pub(crate) fn write_json(path: &Path, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    write(path, &bytes)
}

/// Stable hash of (panic location, version), 16 hex characters.
///
/// One fingerprint is one GitHub issue: a crash loop reports an occurrence
/// instead of filing the same issue again. The version participates because a
/// panic at the same line after a change is a different failure.
pub(crate) fn fingerprint_of(location: Option<&str>, version: &str) -> String {
    use sha2::{Digest as _, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(location.unwrap_or("<unknown location>").as_bytes());
    hasher.update([0]);
    hasher.update(version.as_bytes());
    let digest = hex::encode(hasher.finalize());
    digest[..16].to_owned()
}

/// `YYYYMMDDTHHMMSSZ`: sorts as time, survives every filesystem.
pub(crate) fn utc_stamp() -> String {
    jiff::Timestamp::now()
        .strftime("%Y%m%dT%H%M%SZ")
        .to_string()
}

/// Start the upload child and return immediately.
///
/// The child is the same executable with `--upload-crash`, so a crashed
/// process needs no second binary and the receiver contract stays in one
/// place. It gets no stdin and no output, and its own process group, so a
/// terminal signal sent to the game's process group cannot kill the upload.
fn spawn_uploader(directory: &Path) {
    if let Err(error) = spawn_upload_child(UPLOAD_ARGUMENT, directory) {
        // The process is already dying; stderr is the only channel left.
        report_failure("cannot start the crash uploader", error);
    }
}

/// Start this executable as an upload child for one bundle directory.
///
/// One child mode per bundle kind: `argument` names the hidden CLI flag the
/// binary dispatches on. In a test the child is recorded beside the bundle
/// instead, because a test's bundle lives in a child process and no test may
/// reach the network.
pub(crate) fn spawn_upload_child(argument: &str, directory: &Path) -> Result<()> {
    #[cfg(test)]
    {
        let _ = std::fs::write(
            directory.join(SPAWN_MARKER),
            format!("{argument} {}\n", directory.display()),
        );
        Ok(())
    }
    #[cfg(not(test))]
    detached(argument, directory)
}

#[cfg(test)]
const SPAWN_MARKER: &str = "upload-requested.txt";

#[cfg(not(test))]
fn detached(argument: &str, directory: &Path) -> Result<()> {
    let executable = std::env::current_exe().context("locate this executable")?;
    let mut command = std::process::Command::new(executable);
    command
        .arg(argument)
        .arg(directory)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command.spawn().context("start the upload child")?;
    Ok(())
}

#[allow(clippy::print_stderr)]
fn report_failure(what: &str, error: anyhow::Error) {
    // The crash path runs before or outside tracing; stderr may be nowhere.
    eprintln!("[robin] {what}: {error:#}");
}

/// POST one bundle and return the process exit status.
///
/// Called only in the `--upload-crash` child. Non-zero means the bundle stayed
/// on disk for a later attempt; nothing here panics, so a broken receiver
/// cannot produce a second crash report *about* the crash reporter.
pub fn upload_bundle(directory: &Path) -> i32 {
    match upload(directory) {
        Ok(()) => 0,
        Err(error) => {
            report_failure("crash upload failed", error);
            1
        }
    }
}

fn upload(directory: &Path) -> Result<()> {
    let endpoint = std::env::var(ENDPOINT_ENV)
        .ok()
        .map(|endpoint| endpoint.trim().to_owned())
        .filter(|endpoint| !endpoint.is_empty())
        .with_context(|| format!("{ENDPOINT_ENV} is not set"))?;
    let payload = payload(directory)?;
    // `reqwest` is built with `rustls-no-provider`; the process selects one
    // before the first request. Installing it twice is harmless.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::blocking::Client::builder()
        .timeout(UPLOAD_TIMEOUT)
        .connect_timeout(UPLOAD_TIMEOUT)
        // A signed-in-clear body must never be forwarded to another origin.
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut request = client.post(endpoint.clone()).json(&payload);
    if let Ok(token) = std::env::var(TOKEN_ENV) {
        let token = token.trim();
        if !token.is_empty() {
            request = request.bearer_auth(token);
        }
    }
    let response = request.send().with_context(|| format!("POST {endpoint}"))?;
    let status = response.status();
    anyhow::ensure!(status.is_success(), "receiver answered HTTP {status}");
    Ok(())
}

/// The bundle as one JSON document: the panic facts plus the evidence files
/// the receiver's issue body quotes.
fn payload(directory: &Path) -> Result<Value> {
    let bundle = std::fs::read(directory.join("bundle.json"))
        .with_context(|| format!("read {}/bundle.json", directory.display()))?;
    let mut document: Value = serde_json::from_slice(&bundle).context("decode bundle.json")?;
    let object = document
        .as_object_mut()
        .context("bundle.json is not a JSON object")?;
    object.insert(
        "state".into(),
        read_optional_json(&directory.join("state.json"))?,
    );
    object.insert(
        "run_log_tail".into(),
        read_optional(&directory.join("run-log-tail.txt"))?,
    );
    object.insert(
        "events_tail".into(),
        read_optional(&directory.join("events-tail.jsonl"))?,
    );
    Ok(document)
}

fn read_optional(path: &Path) -> Result<Value> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Value::String(String::from_utf8_lossy(&bytes).into_owned())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn read_optional_json(path: &Path) -> Result<Value> {
    match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).with_context(|| format!("decode {}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panic here would install a process-wide hook for every other test in
    /// the lib binary, so the hook is exercised in a copy of the test binary:
    /// the same shape the binary's own crash tests use.
    const CHILD_ENV: &str = "ROBIN_CRASH_BUNDLE_TEST_CHILD";
    const CHILD_HOME_ENV: &str = "ROBIN_CRASH_BUNDLE_TEST_HOME";

    /// Run one child that panics with the hook installed and return the install
    /// home it used.
    fn panic_child(endpoint: Option<&str>) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "crash::tests::bundle_child_panic", "--nocapture"])
            .env(CHILD_ENV, "1")
            .env(CHILD_HOME_ENV, home.path());
        match endpoint {
            Some(endpoint) => command.env(ENDPOINT_ENV, endpoint),
            None => command.env_remove(ENDPOINT_ENV),
        };
        let output = command.output().unwrap();
        assert!(
            !output.status.success(),
            "the child is expected to die of the panic under test"
        );
        home
    }

    #[test]
    fn bundle_child_panic() {
        if std::env::var_os(CHILD_ENV).is_none() {
            return;
        }
        let home = PathBuf::from(std::env::var_os(CHILD_HOME_ENV).unwrap());
        install(&home);
        // A spawned thread, not the test thread: a panic outside the main
        // thread is what a game session's worker actually looks like.
        let joined = std::thread::spawn(|| panic!("crash bundle test panic")).join();
        // The hook has already written the bundle; exit without panicking a
        // second time, which would write a second one.
        std::process::exit(if joined.is_err() { 101 } else { 0 });
    }

    fn only_bundle(home: &Path) -> PathBuf {
        let directory = home.join(BUNDLE_DIRECTORY);
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        assert_eq!(entries.len(), 1, "{entries:?}");
        entries.remove(0)
    }

    #[test]
    fn a_panicking_thread_writes_a_bundle_that_names_the_panic() {
        let home = panic_child(None);
        let bundle = only_bundle(home.path());
        let document: Value =
            serde_json::from_slice(&std::fs::read(bundle.join("bundle.json")).unwrap()).unwrap();
        assert_eq!(document["message"], "crash bundle test panic");
        let location = document["location"].as_str().expect("a panicking line");
        assert_eq!(
            location.split(':').next(),
            Some(file!()),
            "the bundle names the source file of the panicking line"
        );
        assert_eq!(location.split(':').count(), 3, "{location}");
        assert_eq!(document["version"], version());
        assert_eq!(document["platform"], std::env::consts::OS);
        assert_eq!(document["arch"], std::env::consts::ARCH);
        assert_eq!(
            document["fingerprint"],
            fingerprint_of(Some(location), version())
        );
        assert!(
            document["backtrace"]
                .as_str()
                .unwrap()
                .contains("bundle_child_panic")
        );
        assert!(
            !bundle.join(SPAWN_MARKER).exists(),
            "a crash without an endpoint must not request an upload"
        );
        assert!(
            bundle.join("bundle.json").exists(),
            "the bundle is written before any upload"
        );
    }

    #[test]
    fn an_endpoint_requests_the_upload_child() {
        let home = panic_child(Some("https://receiver.invalid/crash"));
        let bundle = only_bundle(home.path());
        assert_eq!(
            std::fs::read_to_string(bundle.join(SPAWN_MARKER)).unwrap(),
            format!("{UPLOAD_ARGUMENT} {}\n", bundle.display()),
            "the child is asked to upload exactly this bundle"
        );
    }

    #[test]
    fn fingerprint_is_stable_for_one_location_and_version() {
        let first = fingerprint_of(Some("crates/a.rs:1:2"), "0.1.0+0123456789ab");
        assert_eq!(
            first,
            fingerprint_of(Some("crates/a.rs:1:2"), "0.1.0+0123456789ab")
        );
        assert_ne!(
            first,
            fingerprint_of(Some("crates/a.rs:1:3"), "0.1.0+0123456789ab"),
            "a different location is a different crash"
        );
        assert_ne!(
            first,
            fingerprint_of(Some("crates/a.rs:1:2"), "0.1.0+ba9876543210"),
            "the same line in another build is a different crash"
        );
        assert_eq!(first.len(), 16);
    }

    #[test]
    fn a_tail_keeps_whole_lines_and_the_requested_count() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("run.log");
        let text: String = (0..500).map(|line| format!("line {line}\n")).collect();
        std::fs::write(&path, text).unwrap();
        assert_eq!(tail(&path, 3).unwrap(), "line 497\nline 498\nline 499\n");

        // A file larger than the read window starts mid-line; the partial line
        // is dropped so every reported line is a whole log line.
        let path = home.path().join("large.log");
        let text: String = (0..80_000).map(|line| format!("line {line}\n")).collect();
        assert!(text.len() as u64 > TAIL_BYTES);
        std::fs::write(&path, text).unwrap();
        let tail = tail(&path, 2).unwrap();
        assert_eq!(tail, "line 79998\nline 79999\n");
    }
}
