//! One resolver for every per-install state path.
//!
//! Saves, the transport identity, replays, attempt links, diagnostic reports
//! and `datadir.txt` all live under one install root, so two processes on one
//! OS account can be separated without touching the account's data directory.
//!
//! `ROBINHOOD_INSTALL_HOME`, when set, *is* that root. It parents every path
//! below. `ROBINHOOD_SAVE_DIR` keeps its narrower meaning and overrides the
//! save root alone; `ROBINHOOD_MODS_DIR` belongs to the mod pack and is not
//! install state.
//!
//! No other module may call `dirs::data_dir()`: the source-scoped test at the
//! bottom of this file fails the crate when one does.

use std::path::PathBuf;

/// Parent of every per-install path, or `None` when the platform exposes no
/// data directory and no override was given.
fn install_root() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("ROBINHOOD_INSTALL_HOME") {
        return Some(PathBuf::from(dir));
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(data_dir) = dirs::data_dir() {
        return Some(data_dir.join("robin_hood"));
    }
    None
}

/// Root holding `profiles.json`, `keyconfigs.json`, `language.json` and one
/// `Profile_NNN/` directory per player profile.
///
/// `ROBINHOOD_SAVE_DIR` overrides this directory, and only this one.
pub fn save_root() -> PathBuf {
    if let Ok(dir) = std::env::var("ROBINHOOD_SAVE_DIR") {
        return PathBuf::from(dir);
    }
    install_root()
        .map(|root| root.join("saves"))
        .unwrap_or_else(|| PathBuf::from("Data/Savegame/default"))
}

/// Durable Ed25519 seed shared by the leaderboard signer and the multiplayer
/// transport. `None` when no install root exists.
pub fn identity_key_path() -> Option<PathBuf> {
    install_root().map(|root| root.join("multiplayer_identity.key"))
}

/// Directory holding recorded `*.mission` replays.
pub fn replay_directory() -> PathBuf {
    install_root()
        .map(|root| root.join("replays"))
        .unwrap_or_else(|| PathBuf::from("Data/Replays"))
}

/// Directory holding one attempt link per completed mission attempt.
pub fn attempt_directory() -> PathBuf {
    install_root()
        .map(|root| root.join("replays").join("attempts"))
        .unwrap_or_else(|| PathBuf::from("Data/Replays/attempts"))
}

/// Directory holding this install's run logs: one `robin-<stamp>.log` per
/// process and the append-only `events.jsonl` session narrative. `None` when no
/// install root exists.
pub fn log_directory() -> Option<PathBuf> {
    install_root().map(|root| root.join("logs"))
}

/// Queue directory for captured diagnostics. `None` when no install root
/// exists.
pub fn report_directory() -> Option<PathBuf> {
    install_root().map(|root| root.join("reports"))
}

/// Directory for the pinned bundle a multiplayer session writes when it fails.
/// `None` when no install root exists.
pub fn session_diagnostics_directory() -> Option<PathBuf> {
    install_root().map(|root| root.join("diagnostics"))
}

/// Remembered game-installation choice. `None` when no install root exists.
pub fn datadir_file() -> Option<PathBuf> {
    install_root().map(|root| root.join("datadir.txt"))
}

#[cfg(test)]
pub(crate) use test_env::ENV_LOCK;

/// Serializes every test that reads or writes these environment variables:
/// `std::env` is process-global, so parallel tests would otherwise observe
/// each other's overrides.
#[cfg(test)]
mod test_env {
    pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn guard() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn set_both(install: Option<&Path>, save: Option<&Path>) {
        // Safety: every test touching these variables holds ENV_LOCK.
        unsafe {
            match install {
                Some(dir) => std::env::set_var("ROBINHOOD_INSTALL_HOME", dir),
                None => std::env::remove_var("ROBINHOOD_INSTALL_HOME"),
            }
            match save {
                Some(dir) => std::env::set_var("ROBINHOOD_SAVE_DIR", dir),
                None => std::env::remove_var("ROBINHOOD_SAVE_DIR"),
            }
        }
    }

    #[test]
    fn install_home_parents_every_per_install_path() {
        let _guard = guard();
        let home = tempfile::tempdir().unwrap();
        set_both(Some(home.path()), None);

        assert_eq!(save_root(), home.path().join("saves"));
        assert_eq!(replay_directory(), home.path().join("replays"));
        assert_eq!(attempt_directory(), home.path().join("replays/attempts"));
        assert_eq!(log_directory(), Some(home.path().join("logs")));
        assert_eq!(report_directory(), Some(home.path().join("reports")));
        assert_eq!(
            session_diagnostics_directory(),
            Some(home.path().join("diagnostics"))
        );
        assert_eq!(datadir_file(), Some(home.path().join("datadir.txt")));
        assert_eq!(
            identity_key_path(),
            Some(home.path().join("multiplayer_identity.key"))
        );

        set_both(None, None);
    }

    #[test]
    fn save_dir_overrides_only_the_save_root() {
        let _guard = guard();
        let home = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        set_both(Some(home.path()), Some(saves.path()));

        assert_eq!(save_root(), saves.path());
        assert_eq!(replay_directory(), home.path().join("replays"));
        assert_eq!(attempt_directory(), home.path().join("replays/attempts"));
        assert_eq!(log_directory(), Some(home.path().join("logs")));
        assert_eq!(report_directory(), Some(home.path().join("reports")));
        assert_eq!(
            session_diagnostics_directory(),
            Some(home.path().join("diagnostics"))
        );
        assert_eq!(datadir_file(), Some(home.path().join("datadir.txt")));
        assert_eq!(
            identity_key_path(),
            Some(home.path().join("multiplayer_identity.key"))
        );

        set_both(None, None);
    }

    #[test]
    fn unset_resolves_to_the_platform_data_directory() {
        let _guard = guard();
        set_both(None, None);
        let Some(data_dir) = dirs::data_dir() else {
            assert_eq!(save_root(), PathBuf::from("Data/Savegame/default"));
            assert_eq!(replay_directory(), PathBuf::from("Data/Replays"));
            assert_eq!(attempt_directory(), PathBuf::from("Data/Replays/attempts"));
            assert_eq!(log_directory(), None);
            assert_eq!(report_directory(), None);
            assert_eq!(session_diagnostics_directory(), None);
            assert_eq!(datadir_file(), None);
            assert_eq!(identity_key_path(), None);
            return;
        };
        let base = data_dir.join("robin_hood");
        assert_eq!(save_root(), base.join("saves"));
        assert_eq!(replay_directory(), base.join("replays"));
        assert_eq!(attempt_directory(), base.join("replays/attempts"));
        assert_eq!(log_directory(), Some(base.join("logs")));
        assert_eq!(report_directory(), Some(base.join("reports")));
        assert_eq!(
            session_diagnostics_directory(),
            Some(base.join("diagnostics"))
        );
        assert_eq!(datadir_file(), Some(base.join("datadir.txt")));
        assert_eq!(
            identity_key_path(),
            Some(base.join("multiplayer_identity.key"))
        );
    }

    /// The override is only as good as its exclusivity: a module that resolves
    /// the platform data directory itself silently splits one install in two.
    #[test]
    fn only_the_resolver_reads_the_platform_data_directory() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let offenders = offenders_in(&root, "install_home.rs");
        assert!(
            offenders.is_empty(),
            "resolve per-install paths through crate::install_home instead of dirs::data_dir: {offenders:?}"
        );
    }

    /// The guard above is only useful if it actually reports a violation.
    #[test]
    fn a_module_that_resolves_the_data_directory_is_reported() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("clean.rs"), "pub fn x() {}").unwrap();
        std::fs::write(
            root.path().join("leaky.rs"),
            "pub fn x() -> Option<std::path::PathBuf> {\n    dirs :: data_dir ()\n}\n",
        )
        .unwrap();
        assert_eq!(
            offenders_in(root.path(), "install_home.rs"),
            vec![PathBuf::from("leaky.rs")]
        );
        assert!(offenders_in(root.path(), "leaky.rs").is_empty());
    }

    /// Files under `root` that mention the platform data directory, except
    /// `resolver` itself.
    fn offenders_in(root: &Path, resolver: &str) -> Vec<PathBuf> {
        let mut offenders = Vec::new();
        visit(root, &mut |path| {
            if path.file_name().is_some_and(|name| name == resolver) {
                return;
            }
            let source = std::fs::read_to_string(path).unwrap();
            // Whitespace-insensitive: rustfmt may split `dirs::data_dir()`, and a
            // `use` of it is the same violation as one qualified call.
            let compact = source.split_whitespace().collect::<String>();
            if compact.contains("dirs::data_dir") {
                offenders.push(path.strip_prefix(root).unwrap().to_owned());
            }
        });
        offenders.sort();
        offenders
    }

    fn visit(directory: &Path, visit_file: &mut impl FnMut(&Path)) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, visit_file);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                visit_file(&path);
            }
        }
    }
}
