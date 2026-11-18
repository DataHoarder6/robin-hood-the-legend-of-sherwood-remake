//! Pinned, human-readable evidence for a multiplayer session that ended
//! without progress.
//!
//! The replay file is the record of what happened; this bundle is the note that
//! says which session to open and why it stopped, written to disk at the moment
//! the session fails so a week-old report still has an address.

use serde::Serialize;
use std::path::PathBuf;

/// Facts a later reader needs to find the replay and understand the stop.
#[derive(Debug, Serialize)]
pub(crate) struct SessionBundle {
    /// The exact failure text shown to the player.
    pub(crate) reason: String,
    /// Admission phase that was waiting, when the failure came from one.
    pub(crate) admission: Option<String>,
    pub(crate) seat: Option<u8>,
    /// The session identity, also the bundle's file stem.
    pub(crate) session_id: Option<String>,
    /// Newest replay at the time of failure; the mission that just stopped.
    pub(crate) replay: Option<PathBuf>,
    /// When `replay` was last written (RFC 3339 with the local offset), so a
    /// reader can tell the replay belongs to this failure.
    pub(crate) replay_modified: Option<String>,
    pub(crate) written_at: String,
}

/// The newest replay and the moment it was last written.
#[derive(Debug)]
pub(crate) struct ReplayEvidence {
    pub(crate) path: PathBuf,
    /// RFC 3339 with the local offset.
    pub(crate) modified: String,
}

/// The most recently modified replay, which is the mission that just stopped.
pub(crate) fn newest_replay() -> Option<ReplayEvidence> {
    let directory = crate::install_home::replay_directory()?;
    let (modified, path) = std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter_map(|path| {
            let modified = path.metadata().and_then(|meta| meta.modified()).ok()?;
            Some((modified, path))
        })
        .max_by_key(|(modified, _)| *modified)?;
    Some(ReplayEvidence {
        path,
        modified: modified_stamp(modified)?,
    })
}

/// RFC 3339 with the local offset, the shape a reader compares against
/// `written_at`.
fn modified_stamp(modified: std::time::SystemTime) -> Option<String> {
    let timestamp = jiff::Timestamp::try_from(modified).ok()?;
    Some(timestamp.to_zoned(jiff::tz::TimeZone::system()).to_string())
}

/// Write the bundle as pretty JSON and return its path.
///
/// The file is named for the session it describes, so a second failure in one
/// session overwrites its own note instead of accumulating guesses; without a
/// session identity the write time names it. Diagnostics must never mask the
/// failure they describe: every error here is reported to the log and the
/// session keeps failing for its original reason.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_session_bundle(
    mut bundle: SessionBundle,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let dir = crate::install_home::session_diagnostics_directory()
        .ok_or("no data directory for diagnostics")?;
    std::fs::create_dir_all(&dir)?;
    // `%:z` → `+HH:MM`; strip the inner colon so the stamp is path-safe.
    let stamp = jiff::Zoned::now()
        .strftime("%Y-%m-%dT%H-%M-%S%:z-%3f")
        .to_string()
        .replace(':', "-");
    bundle.written_at = stamp.clone();
    let stem = match bundle.session_id.as_deref() {
        Some(session_id) => format!("session-{session_id}"),
        None => format!("session-{stamp}"),
    };
    let path = dir.join(format!("{stem}.json"));
    std::fs::write(&path, serde_json::to_vec_pretty(&bundle)?)?;
    Ok(path)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn write_session_bundle(
    _bundle: SessionBundle,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    Err("browser sessions have no durable diagnostic directory".into())
}

/// Write a bundle for a session that is failing and log where the evidence went.
///
/// Never returns an error: the session keeps failing for its original reason
/// and a diagnostics failure is reported, not raised.
pub(crate) fn record_session_failure(
    reason: &str,
    admission: Option<&str>,
    seat: Option<u8>,
    session_id: Option<String>,
) {
    let newest = newest_replay();
    let bundle = SessionBundle {
        reason: reason.to_owned(),
        admission: admission.map(str::to_owned),
        seat,
        session_id,
        replay: newest.as_ref().map(|evidence| evidence.path.clone()),
        replay_modified: newest.map(|evidence| evidence.modified),
        written_at: String::new(),
    };
    match write_session_bundle(bundle) {
        Ok(path) => tracing::error!(
            path = %path.display(),
            %reason,
            "multiplayer: session failed; pinned diagnostics written"
        ),
        Err(error) => tracing::warn!(
            %error,
            %reason,
            "multiplayer: session failed; could not write diagnostics"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_serializes_every_field_a_reader_needs() {
        let bundle = SessionBundle {
            reason: "no admission progress for 180000 ms in peer waiting for BeginSim".into(),
            admission: Some("peer waiting for BeginSim".into()),
            seat: Some(1),
            session_id: Some("7f3c".into()),
            replay: Some(PathBuf::from("/tmp/example.mission")),
            replay_modified: Some("2026-11-17T22:00:01+02:00".into()),
            written_at: "2026-11-17T22-00-00+02-00-000".into(),
        };
        let encoded = serde_json::to_value(&bundle).unwrap();
        for key in [
            "reason",
            "admission",
            "seat",
            "session_id",
            "replay",
            "replay_modified",
            "written_at",
        ] {
            assert!(encoded.get(key).is_some(), "missing {key}");
        }
        assert!(
            encoded["reason"].as_str().unwrap().contains("BeginSim"),
            "the reason must name the phase the player was stuck in"
        );
    }

    /// The bundle is named for its session and says when the replay it points
    /// at was written, so a reader can tell it belongs to this failure.
    #[test]
    fn bundle_is_named_for_its_session_and_carries_the_replay_write_time() {
        let _guard = crate::install_home::ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        // Safety: every test touching these variables holds ENV_LOCK.
        unsafe { std::env::set_var("ROBINHOOD_INSTALL_HOME", home.path()) };
        let replays = home.path().join("replays");
        std::fs::create_dir_all(&replays).unwrap();
        let replay = replays.join("2026-11-17T22-00-00.mission");
        std::fs::write(&replay, b"replay").unwrap();
        let written = std::fs::metadata(&replay).unwrap().modified().unwrap();

        let session = "7f3c4d2e".repeat(8);
        record_session_failure("transport closed", None, Some(1), Some(session.clone()));

        let path = home
            .path()
            .join("diagnostics")
            .join(format!("session-{session}.json"));
        let encoded: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(encoded["session_id"], serde_json::json!(session));
        assert_eq!(encoded["replay"], serde_json::json!(replay));
        let modified = encoded["replay_modified"]
            .as_str()
            .expect("the bundle carries the replay's modification time");
        let parsed: jiff::Timestamp = modified.parse().expect("RFC 3339");
        let expected = jiff::Timestamp::try_from(written).unwrap();
        assert_eq!(parsed, expected);

        // Without a session identity the write time names the bundle.
        std::fs::remove_file(&path).unwrap();
        record_session_failure("transport closed before a session", None, None, None);
        let names: Vec<String> = std::fs::read_dir(home.path().join("diagnostics"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 1, "one failure, one bundle: {names:?}");
        assert!(
            names[0].ends_with(".json") && names[0].contains('T'),
            "the write-time fallback names the bundle: {names:?}"
        );

        unsafe { std::env::remove_var("ROBINHOOD_INSTALL_HOME") };
    }
}
