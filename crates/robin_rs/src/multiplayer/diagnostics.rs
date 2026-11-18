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
    pub(crate) session_id: Option<String>,
    /// Newest replay at the time of failure; the mission that just stopped.
    pub(crate) replay: Option<PathBuf>,
    pub(crate) written_at: String,
}

#[cfg(not(target_arch = "wasm32"))]
fn data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("robin_hood"))
}

pub(crate) fn diagnostics_dir() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("diagnostics"))
}

pub(crate) fn replays_dir() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("replays"))
}

/// The most recently modified replay, which is the mission that just stopped.
pub(crate) fn newest_replay() -> Option<PathBuf> {
    let dir = replays_dir()?;
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .max_by_key(|path| path.metadata().and_then(|meta| meta.modified()).ok())
}

/// Write the bundle as pretty JSON and return its path.
///
/// Diagnostics must never mask the failure they describe: every error here is
/// reported to the log and the session keeps failing for its original reason.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_session_bundle(
    mut bundle: SessionBundle,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let dir = diagnostics_dir().ok_or("no data directory for diagnostics")?;
    std::fs::create_dir_all(&dir)?;
    // `%:z` → `+HH:MM`; strip the inner colon so the stamp is path-safe.
    let stamp = jiff::Zoned::now()
        .strftime("%Y-%m-%dT%H-%M-%S%:z-%3f")
        .to_string()
        .replace(':', "-");
    bundle.written_at = stamp.clone();
    let path = dir.join(format!("session-{stamp}.json"));
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
    let bundle = SessionBundle {
        reason: reason.to_owned(),
        admission: admission.map(str::to_owned),
        seat,
        session_id,
        replay: newest_replay(),
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
            session_id: Some("MultiplayerSessionId([7; 32])".into()),
            replay: Some(PathBuf::from("/tmp/example.mission")),
            written_at: "2026-11-17T22-00-00+02-00-000".into(),
        };
        let encoded = serde_json::to_value(&bundle).unwrap();
        for key in [
            "reason",
            "admission",
            "seat",
            "session_id",
            "replay",
            "written_at",
        ] {
            assert!(encoded.get(key).is_some(), "missing {key}");
        }
        assert!(
            encoded["reason"].as_str().unwrap().contains("BeginSim"),
            "the reason must name the phase the player was stuck in"
        );
    }
}
