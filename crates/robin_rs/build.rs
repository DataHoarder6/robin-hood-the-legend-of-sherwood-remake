//! Build script: emit the target, profile and feature set the bug and session
//! reports name (`ROBIN_BUILD_TARGET`, `ROBIN_BUILD_PROFILE`,
//! `ROBIN_BUILD_FEATURES`).
//!
//! The source commit is deliberately not stamped here: a build script watching
//! `.git` would rebuild this crate on every commit. The final artifact supplies
//! it at link time (`robin_build_info`).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").expect("Cargo must provide TARGET to build scripts");
    let profile = std::env::var("PROFILE").expect("Cargo must provide PROFILE to build scripts");
    let mut features = std::env::vars()
        .filter_map(|(key, value)| {
            (value == "1")
                .then(|| key.strip_prefix("CARGO_FEATURE_").map(str::to_owned))
                .flatten()
        })
        .map(|feature| feature.to_ascii_lowercase().replace('_', "-"))
        .collect::<Vec<_>>();
    features.sort();
    features.dedup();
    println!("cargo:rustc-env=ROBIN_BUILD_TARGET={target}");
    println!("cargo:rustc-env=ROBIN_BUILD_PROFILE={profile}");
    println!(
        "cargo:rustc-env=ROBIN_BUILD_FEATURES={}",
        features.join(",")
    );
}
