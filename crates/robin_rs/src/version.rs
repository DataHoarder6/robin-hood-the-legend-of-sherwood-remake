//! User-visible application version shared by startup and menu UI.

/// The version embedded in this binary.
///
/// Native release builds receive the exact version passed to Velopack from
/// the release workflow. Local and other builds fall back to Cargo's package
/// version.
pub const PACKAGE_VERSION: &str = match option_env!("ROBIN_PACKAGE_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// Identity `--version` prints: the package version plus the engine commit the
/// build script stamped into every binary.
///
/// `PACKAGE_VERSION` is the same for every build of a release tag, so on its
/// own it cannot say which commit produced the binary. `ROBIN_GIT_HASH` is the
/// 12-hex prefix the replay format and multiplayer tickets already use.
pub const CLI_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+", env!("ROBIN_GIT_HASH"));

/// Format the embedded version for display in the game UI.
pub fn version_label() -> String {
    format!("v{PACKAGE_VERSION}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_contains_embedded_package_version() {
        assert_eq!(version_label(), format!("v{PACKAGE_VERSION}"));
    }
}
