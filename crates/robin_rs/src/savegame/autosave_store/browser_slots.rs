//! Browser QuickSave and Continue slots on page-global localStorage.
//!
//! The browser build has no desktop manual-save filesystem: `wasm32-unknown-
//! unknown` reports every `std::fs` operation as unsupported. Autosaves already
//! persist through localStorage with a compressed, checksummed envelope; these
//! two special slots reuse that backend instead of failing the quick-save key.

use super::{browser_key, decode_browser_blob, encode_browser_blob};
use crate::save_file::GameSaveFile;
use anyhow::{Context, Result};

/// The smallest storage surface a browser save slot needs. Native tests use an
/// in-memory implementation; the browser uses page-global localStorage.
pub(crate) trait QuickSlotStore {
    fn read(&self, key: &str) -> Result<Option<String>>;
    fn write(&self, key: &str, value: &str) -> Result<()>;
}

/// localStorage key of one durable QuickSave-family slot.
pub(crate) fn slot_key(save_directory: &str, filename: &str) -> String {
    browser_key(save_directory, &format!("slot.{filename}"))
}

/// Publish one immutable save payload to a browser slot.
pub(crate) fn write_payload<S: QuickSlotStore + ?Sized>(
    store: &S,
    save_directory: &str,
    filename: &str,
    save: &GameSaveFile,
) -> Result<()> {
    let _ = (store, save_directory, filename, save);
    // TODO(81): mirror the current browser refusal until the failing test is
    // red at this seam.
    anyhow::bail!("browser manual-save persistence is unavailable; use durable autosaves")
}

/// Decode one immutable save payload from a browser slot.
pub(crate) fn read_payload<S: QuickSlotStore + ?Sized>(
    store: &S,
    save_directory: &str,
    filename: &str,
) -> Result<GameSaveFile> {
    let key = slot_key(save_directory, filename);
    let encoded = store
        .read(&key)?
        .with_context(|| format!("browser save slot {filename:?} is missing"))?;
    decode_browser_blob(&encoded)
        .with_context(|| format!("decoding browser save slot {filename:?}"))
}

/// Does a browser slot hold a payload? Storage errors are not absence.
pub(crate) fn payload_exists<S: QuickSlotStore + ?Sized>(
    store: &S,
    save_directory: &str,
    filename: &str,
) -> Result<bool> {
    store
        .read(&slot_key(save_directory, filename))
        .map(|value| value.is_some())
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct BrowserSlotStore(crate::blob_store::BrowserLocalStorage);

#[cfg(target_arch = "wasm32")]
impl BrowserSlotStore {
    pub(crate) fn open() -> Result<Self> {
        crate::blob_store::BrowserLocalStorage::open()
            .map(Self)
            .map_err(|error| anyhow::anyhow!("browser localStorage is unavailable: {error}"))
    }
}

#[cfg(target_arch = "wasm32")]
impl QuickSlotStore for BrowserSlotStore {
    fn read(&self, key: &str) -> Result<Option<String>> {
        use crate::blob_store::BlobStore as _;
        self.0.read_text(key).map_err(|error| {
            anyhow::anyhow!(
                "reading browser save slot failed: {error}; retry or free browser storage"
            )
        })
    }

    fn write(&self, key: &str, value: &str) -> Result<()> {
        use crate::blob_store::BlobStore as _;
        self.0.write_text(key, value).map_err(|error| {
            anyhow::anyhow!("browser storage limit reached writing a save: {error}; free browser storage and retry")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{QuickSlotStore, payload_exists, read_payload, slot_key, write_payload};
    use crate::save_file::GameSaveFile;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct MemoryStore {
        entries: RefCell<BTreeMap<String, String>>,
    }

    impl QuickSlotStore for MemoryStore {
        fn read(&self, key: &str) -> anyhow::Result<Option<String>> {
            Ok(self.entries.borrow().get(key).cloned())
        }

        fn write(&self, key: &str, value: &str) -> anyhow::Result<()> {
            self.entries
                .borrow_mut()
                .insert(key.to_owned(), value.to_owned());
            Ok(())
        }
    }

    struct QuotaStore;

    impl QuickSlotStore for QuotaStore {
        fn read(&self, _key: &str) -> anyhow::Result<Option<String>> {
            Ok(None)
        }

        fn write(&self, _key: &str, _value: &str) -> anyhow::Result<()> {
            anyhow::bail!("QuotaExceededError: Failed to execute 'setItem' on 'Storage'")
        }
    }

    fn fixture() -> GameSaveFile {
        use robin_engine::campaign::Campaign;
        use robin_engine::engine::{Engine, LevelAssets, SimConfig};
        use robin_engine::mission::Mission;
        use robin_engine::profiles::{MissionProfile, ProfileManager};

        let mut profiles = ProfileManager::default();
        profiles.missions.push(MissionProfile {
            id: 17,
            mission_filename: "Mission_17".into(),
            proto_level_filename: "Map_17".into(),
            mission_name: "Mission 17".into(),
            ..MissionProfile::default()
        });
        let mut campaign = Campaign::default();
        campaign.missions.push(Mission {
            profile_idx: Some(0),
            ..Mission::default()
        });
        campaign.current_mission_idx = Some(0);
        campaign.snapshot_preselected_with_simulation(7, SimConfig::default());
        let mut assets = LevelAssets::new();
        assets.profile_manager = std::sync::Arc::new(profiles);
        let engine = Engine::new_for_test(800.0, 600.0, campaign, &mut assets).unwrap();
        let host = crate::host::Host::scratch(800.0, 600.0);
        GameSaveFile::capture(&engine, &host, 17, "quick".into())
    }

    #[test]
    fn browser_quick_save_writes_and_reads_quick_and_continue_slots() {
        let save = fixture();
        let store = MemoryStore::default();
        for filename in ["QuickSave", "Continue"] {
            write_payload(&store, "browser-save", filename, &save).unwrap();
            assert!(payload_exists(&store, "browser-save", filename).unwrap());
            let read = read_payload(&store, "browser-save", filename).unwrap();
            assert_eq!(
                serde_json::to_value(&read).unwrap(),
                serde_json::to_value(&save).unwrap()
            );
        }
        assert!(slot_key("browser-save", "QuickSave").contains("browser-save"));
    }

    #[test]
    fn browser_storage_limit_reports_an_actionable_error() {
        let save = fixture();
        let error = write_payload(&QuotaStore, "browser-save", "QuickSave", &save)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("browser storage limit reached"),
            "unexpected message: {error}"
        );
        assert!(
            error.contains("free browser storage and retry"),
            "the player must be told what to do: {error}"
        );
    }
}
