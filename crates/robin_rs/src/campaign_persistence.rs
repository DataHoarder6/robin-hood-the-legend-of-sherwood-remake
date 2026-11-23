//! Non-sensitive campaign-persistence diagnostics for the browser reload check.
//!
//! The `campaign-persistence` RPC reports only digests, counts, storage kinds
//! and byte sizes. It never returns a profile name, a save payload, a credential
//! or an origin password, so the evidence a browser check records stays safe.

use std::sync::Mutex;

/// An explicit persistence failure that must be reported, never swallowed as
/// an in-memory-only retention (`application.rs::RetainOnFailure`). Only the
/// stable kind and stage are retained — never the underlying save bytes.
static LAST_FAILURE: Mutex<Option<(&'static str, &'static str)>> = Mutex::new(None);

/// Record and log a rejected profile/campaign save with a stable, machine-readable
/// message. This is the "reported explicitly" half of criterion 4: the caller
/// may keep an in-memory copy to stay playable, but the rejection is never silent
/// and the previous durable archive is left intact.
pub(crate) fn report_failure(kind: &'static str, stage: &'static str) {
    tracing::error!(
        "campaign-persistence: rejected {kind} at {stage}; progress is retained in memory for this session and the durable archive was not reset"
    );
    if let Ok(mut guard) = LAST_FAILURE.lock() {
        *guard = Some((kind, stage));
    }
}

pub(crate) fn clear_failure() {
    if let Ok(mut guard) = LAST_FAILURE.lock() {
        *guard = None;
    }
}

#[cfg(target_arch = "wasm32")]
fn last_failure() -> serde_json::Value {
    let failure = LAST_FAILURE.lock().ok().and_then(|guard| *guard);
    match failure {
        Some((kind, stage)) => serde_json::json!({ "kind": kind, "stage": stage }),
        None => serde_json::Value::Null,
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use crate::blob_store::{BlobStore as _, BrowserLocalStorage};
    use crate::savegame::autosave_store::{AutosaveManifest, decode_browser_blob};
    use serde_json::json;
    use sha2::{Digest as _, Sha256};

    const PROFILE_KEY: &str = "robin-hood-player-profiles-v1";
    const BYTE_LIMIT: usize = 4 * 1024 * 1024;
    const AUTOSAVE_PREFIX: &str = "robinhood.autosave.v1.";

    fn digest(text: &str) -> String {
        let hash = Sha256::digest(text.as_bytes());
        hex::encode(&hash[..8])
    }

    fn rejected(bytes: usize, kind: &'static str) -> serde_json::Value {
        json!({
            "present": true,
            "archiveBytes": bytes,
            "byteLimit": BYTE_LIMIT,
            "schemaVersion": null,
            "identityDigest": null,
            "configDigest": null,
            "attemptCount": 0,
            "errorKind": kind,
        })
    }

    fn profile_snapshot(storage: &BrowserLocalStorage) -> serde_json::Value {
        let Some(text) = storage.read_text(PROFILE_KEY).unwrap_or(None) else {
            return json!({
                "present": false,
                "archiveBytes": 0,
                "byteLimit": BYTE_LIMIT,
                "schemaVersion": null,
                "identityDigest": null,
                "configDigest": null,
                "attemptCount": 0,
                "errorKind": "missing",
            });
        };
        let bytes = text.len();
        if bytes > BYTE_LIMIT {
            return rejected(bytes, "oversize");
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return rejected(bytes, "corrupt");
        };
        if value.get("schema_version").and_then(|v| v.as_u64()) != Some(1) {
            return rejected(bytes, "corrupt");
        }
        let manager = value.get("manager");
        let profiles = manager
            .and_then(|m| m.get("profiles"))
            .and_then(|p| p.as_array());
        let active = manager
            .and_then(|m| m.get("active_index"))
            .and_then(|v| v.as_u64())
            .and_then(|index| profiles.and_then(|p| p.get(index as usize)));
        let Some(active) = active else {
            return rejected(bytes, "corrupt");
        };
        let identity = active.get("name").and_then(|v| v.as_str()).map(digest);
        // `active` is already the unwrapped `&Value`, so the digest is computed
        // directly from it. Config digests intentionally cover only settings,
        // never the player's name or any save data. This path is always taken
        // for a well-formed archive, so configDigest stays a non-null string.
        let mut canonical = String::new();
        for key in [
            "difficulty",
            "graphic_config",
            "gameplay_config",
            "sound_config",
            "multiplayer_config",
        ] {
            if let Some(field) = active.get(key) {
                canonical.push_str(&field.to_string());
            }
        }
        let config = digest(&canonical);
        let attempts = active
            .get("campaign_history")
            .and_then(|history| history.get("attempts"))
            .and_then(|attempts| attempts.as_array())
            .map_or(0, Vec::len);
        json!({
            "present": true,
            "archiveBytes": bytes,
            "byteLimit": BYTE_LIMIT,
            "schemaVersion": 1,
            "identityDigest": identity,
            "configDigest": config,
            "attemptCount": attempts,
            "errorKind": null,
        })
    }

    fn resume_snapshot(storage: &BrowserLocalStorage) -> serde_json::Value {
        let keys = storage.keys().unwrap_or_default();
        let mut best: Option<(u64, u32, &'static str)> = None;
        for key in keys {
            if !key.starts_with(AUTOSAVE_PREFIX) || !key.ends_with(".manifest") {
                continue;
            }
            let Some(encoded) = storage.read_text(&key).unwrap_or(None) else {
                continue;
            };
            let Ok(manifest) = decode_browser_blob::<AutosaveManifest>(&encoded) else {
                // A corrupt manifest is explicit; it must not be mistaken for
                // "no campaign" and silently reset.
                return json!({ "available": false, "missionId": null, "slotKind": null, "errorKind": "corrupt-manifest" });
            };
            for save in manifest.saves {
                if !crate::save_file::is_supported_save_version(save.version) {
                    continue;
                }
                let slot_kind = if save.is_continue() {
                    "continue"
                } else if save.is_restart() {
                    "restart"
                } else if save.is_autosave() {
                    "autosave"
                } else {
                    continue;
                };
                let timestamp = save.timestamp.parse::<u64>().unwrap_or(0);
                let candidate = (timestamp, save.mission_id, slot_kind);
                if best.as_ref().is_none_or(|current| candidate.0 > current.0) {
                    best = Some(candidate);
                }
            }
        }
        match best {
            Some((_, mission_id, slot_kind)) => json!({
                "available": true,
                "missionId": mission_id,
                "slotKind": slot_kind,
                "errorKind": null,
            }),
            None => {
                json!({ "available": false, "missionId": null, "slotKind": null, "errorKind": null })
            }
        }
    }

    /// Read-only probe of the durable browser persistence surface. Safe to call
    /// at the main menu: it answers straight from localStorage, never from the
    /// mission queue.
    pub(crate) fn probe() -> serde_json::Value {
        let Ok(storage) = BrowserLocalStorage::open() else {
            return json!({
                "storage": "unavailable",
                "profile": rejected(0, "unavailable"),
                "resume": { "available": false, "missionId": null, "slotKind": null, "errorKind": "unavailable" },
                "lastFailure": last_failure(),
            });
        };
        json!({
            "storage": "localStorage",
            "profile": profile_snapshot(&storage),
            "resume": resume_snapshot(&storage),
            "lastFailure": last_failure(),
        })
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) use browser::probe;
