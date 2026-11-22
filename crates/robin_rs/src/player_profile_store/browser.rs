//! Browser profile archive: one versioned envelope in page-global localStorage.

use super::{PlayerProfileStore, decode_browser_profile_archive, encode_browser_profile_archive};
use crate::blob_store::{BlobStore as _, BrowserLocalStorage};
use robin_engine::player_profile::PlayerProfileManager;

const BROWSER_PROFILE_STORE_KEY: &str = "robin-hood-player-profiles-v1";

impl PlayerProfileStore {
    pub fn for_directory(directory: &str) -> Self {
        Self::Browser {
            directory: directory.into(),
        }
    }

    pub(crate) fn directory(&self) -> std::io::Result<&str> {
        match self {
            Self::Browser { directory } => Ok(directory),
            Self::Unavailable { reason } => Err(std::io::Error::other(reason.clone())),
        }
    }

    /// `Ok(None)` only when no archive has been published yet.
    pub(super) fn load_existing(
        &self,
        directory: &str,
    ) -> std::io::Result<Option<PlayerProfileManager>> {
        match self {
            Self::Browser { .. } => BrowserLocalStorage::open()
                .and_then(|storage| storage.read_text(BROWSER_PROFILE_STORE_KEY))
                .map_err(|error| error.into_io("read browser player profiles"))?
                .map(|serialized| decode_browser_profile_archive(&serialized, directory))
                .transpose(),
            Self::Unavailable { .. } => unreachable!("directory checked authority"),
        }
    }

    pub(super) fn publish(&self, manager: &PlayerProfileManager) -> std::io::Result<()> {
        match self {
            Self::Browser { .. } => {
                let serialized = encode_browser_profile_archive(manager)?;
                BrowserLocalStorage::open()
                    .and_then(|storage| storage.write_text(BROWSER_PROFILE_STORE_KEY, &serialized))
                    .map_err(|error| error.into_io("persist browser player profiles"))
            }
            Self::Unavailable { .. } => unreachable!("directory checked authority"),
        }
    }

    /// Browser saves have their own store; the legacy profile archive did not
    /// perform filesystem directory deletion.
    pub(super) fn move_profile_saves(
        &self,
        _profile_id: u32,
        _restore: bool,
    ) -> std::io::Result<()> {
        self.directory()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::BROWSER_PROFILE_BYTE_LIMIT;
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn durable_profile_reload_and_corruption_are_distinguished() {
        let storage = BrowserLocalStorage::open().unwrap();
        storage.remove(BROWSER_PROFILE_STORE_KEY).unwrap();

        let mut first = PlayerProfileStore::for_directory("browser-save")
            .load()
            .unwrap();
        assert!(first.default_profiles);
        first.default_profiles = false;
        first.profiles[0].name = "Durable Robin".to_owned();
        PlayerProfileStore::for_directory("browser-save")
            .save(&first)
            .unwrap();

        let reloaded = PlayerProfileStore::for_directory("ignored-after-load")
            .load()
            .unwrap();
        assert!(!reloaded.default_profiles);
        assert_eq!(reloaded.get_active().unwrap().name, "Durable Robin");
        assert_eq!(reloaded.save_directory, "ignored-after-load");

        storage
            .write_text(
                BROWSER_PROFILE_STORE_KEY,
                r#"{"schema_version":999,"manager":{}}"#,
            )
            .unwrap();
        assert!(
            PlayerProfileStore::for_directory("browser-save")
                .load()
                .is_err()
        );
        assert!(
            decode_browser_profile_archive(
                &"x".repeat(BROWSER_PROFILE_BYTE_LIMIT + 1),
                "browser-save"
            )
            .unwrap_err()
            .to_string()
            .contains("limit")
        );
        storage.remove(BROWSER_PROFILE_STORE_KEY).unwrap();
    }

    /// Issue #76: a reload must retain the player's identity/config and every
    /// campaign attempt, and a rejected archive must be reported explicitly
    /// rather than silently reset. The probe is the non-sensitive surface the
    /// browser check asserts; no name or save payload leaves this process.
    #[wasm_bindgen_test]
    fn campaign_persistence_probe_distinguishes_retained_progress_from_rejection() {
        let storage = BrowserLocalStorage::open().unwrap();
        storage.remove(BROWSER_PROFILE_STORE_KEY).unwrap();
        crate::campaign_persistence::clear_failure();

        let mut manager = PlayerProfileStore::for_directory("browser-save")
            .load()
            .unwrap();
        manager.default_profiles = false;
        manager.profiles[0].name = "Campaign Pilot".to_owned();
        PlayerProfileStore::for_directory("browser-save")
            .save(&manager)
            .unwrap();

        // Retain a full campaign attempt without trimming history: the probe
        // must report it as durable campaign progress, not zero.
        let encoded = storage
            .read_text(BROWSER_PROFILE_STORE_KEY)
            .unwrap()
            .unwrap();
        let mut document: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        document["manager"]["profiles"][0]["campaign_history"]["attempts"] =
            serde_json::json!([{}]);
        storage
            .write_text(BROWSER_PROFILE_STORE_KEY, &document.to_string())
            .unwrap();

        let probe = crate::campaign_persistence::probe();
        assert!(probe["profile"]["errorKind"].is_null());
        assert_eq!(probe["profile"]["attemptCount"].as_u64(), Some(1));
        assert!(probe["profile"]["identityDigest"].as_str().is_some());
        assert!(probe["profile"]["configDigest"].as_str().is_some());

        // A corrupt archive is reported; it is never treated as a first launch.
        storage
            .write_text(
                BROWSER_PROFILE_STORE_KEY,
                r#"{"schema_version":999,"manager":{}}"#,
            )
            .unwrap();
        let probe = crate::campaign_persistence::probe();
        assert_eq!(probe["profile"]["errorKind"].as_str(), Some("corrupt"));

        // An oversized archive is reported with its own kind.
        storage
            .write_text(
                BROWSER_PROFILE_STORE_KEY,
                &"x".repeat(BROWSER_PROFILE_BYTE_LIMIT + 1),
            )
            .unwrap();
        let probe = crate::campaign_persistence::probe();
        assert_eq!(probe["profile"]["errorKind"].as_str(), Some("oversize"));

        // Explicit failure reporting is observable through the probe.
        crate::campaign_persistence::report_failure("oversize", "test");
        let probe = crate::campaign_persistence::probe();
        assert_eq!(probe["lastFailure"]["kind"].as_str(), Some("oversize"));
        crate::campaign_persistence::clear_failure();
        storage.remove(BROWSER_PROFILE_STORE_KEY).unwrap();
    }
}
