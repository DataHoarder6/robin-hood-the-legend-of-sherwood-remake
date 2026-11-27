//! The build's source identity as a link-time contract.
//!
//! Libraries read the commit through [`commit`] and [`short_commit`]; the value
//! is supplied by the final artifact that links them, never compiled into a
//! library. A library that bakes the commit in (an `env!` fed by a build script
//! watching `.git/HEAD`) is rebuilt on every commit, and Cargo then rebuilds
//! every crate that depends on it — on the gate that was the 11 MB `robin_rs`
//! in every configuration of every landing
//! (`docs/slop/build-time-root-causes.md`).
//!
//! Exactly one crate in each final artifact defines the symbol:
//!
//! - binaries, examples and cdylibs: `robin_build_identity::define!();` at the
//!   crate root, stamping the commit `robin_build_identity`'s build script
//!   discovered;
//! - test harnesses of a library that must not depend on
//!   `robin_build_identity` (unit tests and integration tests of
//!   `robin_replay_format` and `robin_rs`): [`define_test_identity!`], a fixed
//!   commit nobody built.
//!
//! A final artifact without a definition fails to link with an undefined
//! `robin_build_identity_commit_v1`; one with two fails with a duplicate symbol.
//! Neither is ever papered over.

unsafe extern "Rust" {
    /// The full lowercase hex commit (40 or 64 characters), or the literal
    /// `unknown` for a developer build from a source archive.
    safe fn robin_build_identity_commit_v1() -> &'static str;
}

/// The full commit of the final artifact this code is linked into.
pub fn commit() -> &'static str {
    robin_build_identity_commit_v1()
}

/// The 12-character prefix the replay format, `--version` and multiplayer
/// artifacts carry.
pub fn short_commit() -> &'static str {
    let commit = commit();
    commit.get(..12).unwrap_or(commit)
}

/// The commit a test harness reports: well formed, so version validation
/// accepts it, and recognisable, so it can never be mistaken for a real build.
pub const TEST_COMMIT: &str = "7e57c0de7e57c0de7e57c0de7e57c0de7e57c0de";

/// Define the identity of a test harness as [`TEST_COMMIT`].
#[macro_export]
macro_rules! define_test_identity {
    () => {
        #[unsafe(no_mangle)]
        fn robin_build_identity_commit_v1() -> &'static str {
            $crate::TEST_COMMIT
        }
    };
}
