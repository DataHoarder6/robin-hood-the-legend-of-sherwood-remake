//! The run log: one file per process, kept where an agent can find it.
//!
//! A game started from a desktop launcher inherits no terminal, so stderr is
//! gone by the time anyone asks what happened. Every run therefore also writes
//! its trace to `<install home>/logs/robin-<YYYYMMDDTHHMMSS>.log`.
//!
//! Retention is the newest [`RETAINED_RUN_LOGS`] files, pruned at startup. The
//! oldest run is expendable; an install is never expendable to its disk.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// How many run logs one install keeps. Small on purpose: a run log is evidence
/// for the last few failures, not an archive.
pub(crate) const RETAINED_RUN_LOGS: usize = 10;

const FILE_PREFIX: &str = "robin-";
const FILE_SUFFIX: &str = ".log";

/// Process-lifetime owner of the open run-log file. The path is the durable
/// fact; the writer is only how it is appended to.
#[derive(Default)]
pub(super) struct RunLog {
    path: Option<PathBuf>,
    writer: Option<TraceWriter>,
    /// Why the run log could not be opened, reported once the subscriber can
    /// carry the message (`install` runs before tracing exists).
    failure: Option<String>,
}

/// A `tracing_subscriber` sink that appends to this run's log file.
#[derive(Clone)]
pub(crate) struct TraceWriter(Arc<Mutex<std::fs::File>>);

impl RunLog {
    pub(super) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(super) fn writer(&self) -> Option<TraceWriter> {
        self.writer.clone()
    }

    pub(super) fn take_failure(&mut self) -> Option<String> {
        self.failure.take()
    }

    /// Record why this process has no run log; reported once tracing exists.
    pub(super) fn deny(&mut self, reason: &str) {
        self.failure = Some(reason.to_owned());
    }
}

/// Open this run's log file under `directory`.
///
/// Retention makes room first, keeping [`RETAINED_RUN_LOGS`] - 1 older files,
/// and only then is this run's file created — so the file this process is about
/// to append to can never be one of the pruned ones.
pub(super) fn open(directory: &Path) -> std::io::Result<RunLog> {
    open_at(directory, &current_stamp())
}

fn open_at(directory: &Path, stamp: &str) -> std::io::Result<RunLog> {
    std::fs::create_dir_all(directory)?;
    // One slot is this run's own file.
    prune(directory, RETAINED_RUN_LOGS - 1)?;
    let path = create_run_file(directory, stamp)?;
    let file = std::fs::OpenOptions::new().append(true).open(&path)?;
    Ok(RunLog {
        path: Some(path),
        writer: Some(TraceWriter(Arc::new(Mutex::new(file)))),
        failure: None,
    })
}

fn current_stamp() -> String {
    jiff::Zoned::now().strftime("%Y%m%dT%H%M%S").to_string()
}

/// `<install home>/logs/robin-<YYYYMMDDTHHMMSS>.log`, with a `-<pid>` suffix
/// only when two processes on one install start inside the same second.
fn create_run_file(directory: &Path, stamp: &str) -> std::io::Result<PathBuf> {
    let name = format!("{FILE_PREFIX}{stamp}{FILE_SUFFIX}");
    let path = directory.join(name);
    match std::fs::OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(&path)
    {
        Ok(_) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let unique = directory.join(format!(
                "{FILE_PREFIX}{stamp}-{}{FILE_SUFFIX}",
                std::process::id()
            ));
            std::fs::OpenOptions::new()
                .create_new(true)
                .append(true)
                .open(&unique)?;
            Ok(unique)
        }
        Err(error) => Err(error),
    }
}

/// Keep the newest `retained` run logs in `directory`.
///
/// Age comes from the filesystem, not from the name: a `-<pid>` collision sorts
/// differently from the plain name, so byte order is not age order.
fn prune(directory: &Path, retained: usize) -> std::io::Result<()> {
    let mut logs = Vec::new();
    for path in run_logs(directory)? {
        logs.push((std::fs::metadata(&path)?.modified()?, path));
    }
    logs.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let excess = logs.len().saturating_sub(retained);
    for (_, path) in logs.into_iter().take(excess) {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            // The file is evidence, not a reason to refuse to start.
            Err(error) => tracing::warn!(
                path = %path.display(),
                %error,
                "run log: could not prune an older run log"
            ),
        }
    }
    Ok(())
}

fn run_logs(directory: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut logs = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|name| name.to_str());
        if let Some(name) = name
            && name.starts_with(FILE_PREFIX)
            && name.ends_with(FILE_SUFFIX)
        {
            logs.push(path);
        }
    }
    Ok(logs)
}

impl std::io::Write for TraceWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let mut file = self.0.lock().unwrap_or_else(|error| error.into_inner());
        file.write_all(buffer)?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TraceWriter {
    type Writer = TraceWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::prelude::*;

    fn names(directory: &Path) -> Vec<String> {
        let mut names: Vec<String> = run_logs(directory)
            .unwrap()
            .into_iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_run_creates_its_log_file_and_the_startup_line_lands_in_it() {
        let home = tempfile::tempdir().unwrap();
        let log = open(home.path()).unwrap();
        let path = log.path().unwrap().to_path_buf();
        assert!(path.exists(), "{}", path.display());
        let name = path.file_name().unwrap().to_string_lossy();
        let stamp = &name[FILE_PREFIX.len()..name.len() - FILE_SUFFIX.len()];
        assert!(
            stamp.len() == 15 && stamp.as_bytes()[8] == b'T',
            "unexpected run log name: {name}"
        );

        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(log.writer().unwrap()),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("startup line: run log opened");
        });

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("startup line: run log opened"),
            "{written}"
        );
    }

    #[test]
    fn startup_retains_the_newest_run_logs_and_prunes_the_rest() {
        let home = tempfile::tempdir().unwrap();
        for day in 1..=RETAINED_RUN_LOGS + 2 {
            std::fs::write(
                home.path().join(format!("robin-202601{day:02}T000000.log")),
                "",
            )
            .unwrap();
        }
        // Not a run log: retention must not touch another file in the directory.
        std::fs::write(home.path().join("events.jsonl"), "").unwrap();
        std::fs::write(home.path().join("notes.txt"), "").unwrap();

        let log = open(home.path()).unwrap();
        let kept = names(home.path());
        assert_eq!(kept.len(), RETAINED_RUN_LOGS, "{kept:?}");
        assert!(
            kept.contains(
                &log.path()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            )
        );
        assert!(!kept.contains(&format!("robin-20260101T000000{FILE_SUFFIX}")));
        assert!(!kept.contains(&format!("robin-20260102T000000{FILE_SUFFIX}")));
        assert!(!kept.contains(&format!("robin-20260103T000000{FILE_SUFFIX}")));
        // Retention owns `robin-*.log` only.
        assert!(home.path().join("events.jsonl").exists());
        assert!(home.path().join("notes.txt").exists());
    }

    /// Two processes starting inside one second make the collision name
    /// `robin-<stamp>-<pid>.log`, which sorts before `robin-<stamp>.log`
    /// (`-` is `0x2D`, `.` is `0x2E`). Retention used to run after the file was
    /// created and order by name, so the run's own log was the one deleted and
    /// the append then failed `NotFound` — losing both the run log and the
    /// event file for that process.
    #[test]
    fn a_same_second_collision_keeps_the_run_file_it_just_created() {
        let home = tempfile::tempdir().unwrap();
        let stamp = "20261118T101500";
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        // Nine processes from an earlier second, all named so they sort after
        // this run's collision name.
        for index in 0..RETAINED_RUN_LOGS - 1 {
            let path = home
                .path()
                .join(format!("{FILE_PREFIX}{stamp}-999999{index}{FILE_SUFFIX}"));
            let file = std::fs::File::create(&path).unwrap();
            file.set_modified(old).unwrap();
        }
        // Another process in this same second already took the plain name.
        std::fs::File::create(
            home.path()
                .join(format!("{FILE_PREFIX}{stamp}{FILE_SUFFIX}")),
        )
        .unwrap();

        let log = open_at(home.path(), stamp).unwrap();
        let path = log.path().unwrap().to_path_buf();
        assert!(path.exists(), "{}", path.display());
        let mut writer = log.writer().unwrap();
        std::io::Write::write_all(&mut writer, b"this run survives retention\n").unwrap();
        std::io::Write::flush(&mut writer).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("this run survives retention"), "{written}");
        let kept = names(home.path());
        assert_eq!(kept.len(), RETAINED_RUN_LOGS, "{kept:?}");
        assert!(
            kept.contains(&path.file_name().unwrap().to_string_lossy().into_owned()),
            "{kept:?}"
        );
    }
}
