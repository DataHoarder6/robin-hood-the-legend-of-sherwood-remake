//! The commit a final artifact was built from.
//!
//! Only final artifacts depend on this crate: its build script reruns on every
//! commit, so any library depending on it would be rebuilt on every commit too.
//! Libraries read the value through `robin_build_info`; see that crate for the
//! contract.

/// The full commit `build.rs` discovered or the packager supplied.
pub const COMMIT: &str = env!("ROBIN_GIT_COMMIT");

/// Define this final artifact's identity as [`COMMIT`]. Call it once, at the
/// root of a binary, example or cdylib.
#[macro_export]
macro_rules! define {
    () => {
        #[unsafe(no_mangle)]
        fn robin_build_identity_commit_v1() -> &'static str {
            $crate::COMMIT
        }
    };
}
