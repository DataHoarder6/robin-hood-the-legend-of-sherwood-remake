//! Exercise build-script provenance in subprocesses so Cargo/environment state
//! is never changed in the test runner or concurrent tests.
#[path = "../build.rs"]
mod build_script;

/// A commit nobody built. `ROBIN_GIT_COMMIT` is an *output* of the build
/// script, and Cargo exports this package's build-script `cargo:rustc-env`
/// values into the environment of the binaries it runs, so the inherited copy
/// is normally present here. The probe overwrites it with this stamp to pin
/// that an inherited output can never stand in for a packager's commit: an
/// override read from it would surface as this hash in the assertions below.
const UNEARNED_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn provenance_probe() {
    if std::env::var_os("ROBIN_TEST_PROVENANCE_PROBE").is_some() {
        build_script::main();
    }
}

fn probe(
    manifest: &std::path::Path,
    strict: bool,
    git_ceiling: Option<&std::path::Path>,
    package_commit: Option<&str>,
) -> std::process::Output {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "provenance_probe", "--nocapture"])
        .env("ROBIN_TEST_PROVENANCE_PROBE", "1")
        .env("CARGO_MANIFEST_DIR", manifest)
        .env("GIT_DIR", "/not-the-build-checkout")
        .env("ROBIN_GIT_COMMIT", UNEARNED_COMMIT)
        .env_remove("GIT_CEILING_DIRECTORIES")
        .env_remove("ROBIN_PACKAGE_COMMIT")
        .env_remove("ROBIN_REQUIRE_BUILD_IDENTITY");
    if let Some(ceiling) = git_ceiling {
        command.env("GIT_CEILING_DIRECTORIES", ceiling);
    }
    if strict {
        command.env("ROBIN_REQUIRE_BUILD_IDENTITY", "1");
    }
    if let Some(commit) = package_commit {
        command.env("ROBIN_PACKAGE_COMMIT", commit);
    }
    command.output().unwrap()
}

#[test]
fn source_archives_warn_for_development_and_fail_for_release() {
    let archive = tempfile::tempdir().unwrap();
    let manifest = archive.path().join("crates/client");
    std::fs::create_dir_all(&manifest).unwrap();
    std::fs::write(archive.path().join("Cargo.lock"), "lock bytes").unwrap();
    // TMPDIR may live inside a checkout. Git must treat this fixture as an
    // independent source archive instead of discovering the containing repo.
    let developer = probe(&manifest, false, Some(archive.path()), None);
    assert!(developer.status.success());
    let stdout = String::from_utf8(developer.stdout).unwrap();
    assert!(stdout.contains("cargo:warning=Incomplete developer build provenance"));
    assert!(stdout.contains("cargo:rustc-env=ROBIN_GIT_COMMIT=unknown"));
    assert!(
        !probe(&manifest, true, Some(archive.path()), None)
            .status
            .success()
    );
}

#[test]
fn packager_supplied_commit_stamps_a_source_archive() {
    let archive = tempfile::tempdir().unwrap();
    let manifest = archive.path().join("crates/client");
    std::fs::create_dir_all(&manifest).unwrap();
    std::fs::write(archive.path().join("Cargo.lock"), "lock bytes").unwrap();
    // The Nix package has no `.git` and hands the revision it resolved to the
    // build script; that commit is the identity, so a release build is complete.
    let output = probe(&manifest, true, Some(archive.path()), Some(UNEARNED_COMMIT));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(&format!(
        "cargo:rustc-env=ROBIN_GIT_COMMIT={UNEARNED_COMMIT}"
    )));
    assert!(!stdout.contains("cargo:warning="));
    // A malformed packager commit is refused, never repaired into a guess.
    assert!(
        !probe(&manifest, false, Some(archive.path()), Some("not-a-commit"))
            .status
            .success()
    );
}

#[test]
fn checkout_or_linked_worktree_emits_current_git_identity_and_watches_head() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = probe(manifest, true, None, None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(&format!(
        "cargo:rustc-env=ROBIN_GIT_COMMIT={}",
        head_of(manifest)
    )));
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("cargo:rerun-if-changed=") && line.ends_with("/HEAD"))
    );
}

/// Discovery must resolve a linked worktree's `gitdir:` pointer, not only a
/// plain checkout's `.git` directory, and report that worktree's own HEAD.
#[test]
fn linked_worktree_owns_its_head_and_commit() {
    let root = tempfile::tempdir().unwrap();
    let primary = root.path().join("primary");
    std::fs::create_dir_all(primary.join("crates/client")).unwrap();
    git(&primary, &["init", "--quiet", "--initial-branch=main"]);
    std::fs::write(
        primary.join("crates/client/Cargo.toml"),
        "[package]\nname = \"client\"\n",
    )
    .unwrap();
    git(&primary, &["add", "--all"]);
    git(&primary, &["commit", "--quiet", "--message=fixture"]);
    let commit = head_of(&primary);
    let linked = root.path().join("linked");
    git(
        &primary,
        &["worktree", "add", "--quiet", linked.to_str().unwrap()],
    );

    let output = probe(&linked.join("crates/client"), true, None, None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(&format!("cargo:rustc-env=ROBIN_GIT_COMMIT={commit}")));
    assert!(stdout.lines().any(|line| {
        line.starts_with("cargo:rerun-if-changed=") && line.ends_with("/worktrees/linked/HEAD")
    }));
}

fn head_of(repository: &std::path::Path) -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repository)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn git(repository: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Build Provenance",
            "-c",
            "user.email=provenance@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(repository)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed in {repository:?}");
}
