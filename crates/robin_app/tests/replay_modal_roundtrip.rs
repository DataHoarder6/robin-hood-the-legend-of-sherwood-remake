#![cfg(all(not(target_arch = "wasm32"), unix))]

//! A recording of a mission that opens a modal before the frame's modal drain
//! runs must replay on the binary that made it.
//!
//! Sherwood HQ's `PostInitialize` queues its entry popup after the frame's own
//! modal drain, so the next frame's headless driver is the one that dismisses
//! it. That dismissal is a recorded fact: the replay reopens the popup from the
//! same recorded frame and must consume the acknowledgement there. When the
//! driver instead discarded it, playback reopened the popup with nothing to
//! consume and the *next* recorded dismissal (a tutorial popup 34, opened by
//! `CrouchDown`) was orphaned:
//!
//! ```text
//! replay desync: 1 recorded modal dismissal(s) were unused in their host frame
//! ```
//!
//! The two runs below are the same binary: the first records, the second
//! replays the recorded chunk. The assertion is the existing per-25-frame hash
//! chain: every sampled hash the recorder wrote must be verified by playback,
//! with no desync line. A modal regression fails here rather than in a player's
//! session.
//!
//! Opt-in, because it needs a built `robin` and licensed game data:
//!
//! ```text
//! ROBINHOOD_DATA_DIR=/path/to/robin-hood/game \
//!   cargo test -p robin_rs --test replay_modal_roundtrip -- --ignored --nocapture
//! ```

robin_build_identity::define!();

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SPEC: &str = include_str!("fixtures/scenario_spec_sherwood_hq.json");
/// `GameCode::LevelInterrupted`, the spec's "budget completed" result.
const SPEC_COMPLETED_EXIT: i32 = 3;
/// Mission id whose entry queues the popup from `PostInitialize`.
const MISSION: &str = "Sherwood";
/// The recording's own debug lines, not a second comparer.
const HASH_LINE: &str = "Replay hash OK @ frame ";

fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("ROBINHOOD_DATA_DIR").expect(
        "ROBINHOOD_DATA_DIR must point at an install with licensed game data (e.g. \
             /home/admin/Games/robin-hood/game)",
    ))
}

fn robin(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_robin"))
        .args(["--headless", "--no-sound"])
        .args(arguments)
        .env("ROBINHOOD_DATA_DIR", data_dir())
        .env("RUST_LOG", "debug")
        .output()
        .expect("run robin headless")
}

fn combined_log(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The recording chunk the run wrote, wherever the recorder split it.
fn recording_chunk(directory: &Path) -> PathBuf {
    std::fs::read_dir(directory)
        .expect("recording directory")
        .map(|entry| entry.expect("recording entry").path())
        .find(|path| path.to_string_lossy().ends_with(".rhrec.jsonl"))
        .expect("recording chunk")
}

/// `(frame, hash)` for every cadence sample the recorder wrote.
fn recorded_hashes(chunk: &Path) -> Vec<(u32, String)> {
    let mut samples = Vec::new();
    for line in std::fs::read_to_string(chunk)
        .expect("read recording chunk")
        .lines()
    {
        let row: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|error| panic!("{line}: {error}"));
        let Some(hash) = row.get("h").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        samples.push((
            row.get("f")
                .and_then(serde_json::Value::as_u64)
                .expect("hash row carries its frame") as u32,
            format!("{hash:016x}"),
        ));
    }
    samples.sort_unstable();
    samples
}

/// `(frame, hash)` for every hash playback re-derived and verified.
fn replayed_hashes(log: &str) -> Vec<(u32, String)> {
    let mut samples = Vec::new();
    for line in log.lines() {
        let Some(rest) = line.split(HASH_LINE).nth(1) else {
            continue;
        };
        let (frame, hash) = rest.split_once(": ").expect("hash line carries a value");
        samples.push((
            frame.parse().expect("hash line carries its frame"),
            hash.trim().to_owned(),
        ));
    }
    samples.sort_unstable();
    samples
}

#[test]
#[ignore = "needs licensed game data and the built robin binary"]
fn sherwood_hq_entry_modal_recording_replays_to_identical_hashes() {
    let directory = tempfile::tempdir().expect("temp dir");
    let spec_path = directory.path().join("sherwood-hq.json");
    std::fs::write(&spec_path, SPEC).expect("write scenario spec");
    let record_dir = directory.path().join("recording");

    let recorded = robin(&[
        "--mission",
        MISSION,
        "--spec",
        spec_path.to_str().expect("UTF-8 spec path"),
        "--record",
        record_dir.to_str().expect("UTF-8 record path"),
    ]);
    let log = combined_log(&recorded);
    assert_eq!(
        recorded.status.code(),
        Some(SPEC_COMPLETED_EXIT),
        "the recording run did not complete its spec budget: {log}"
    );
    assert!(
        !log.contains("replay desync"),
        "the recording run reported a modal desync: {log}"
    );
    let chunk = recording_chunk(&record_dir);
    let recorded_chain = recorded_hashes(&chunk);
    assert!(
        recorded_chain.len() > 1,
        "the recorder wrote no cadence hashes to {}",
        chunk.display()
    );

    let replayed = robin(&["--replay", record_dir.to_str().expect("UTF-8 replay path")]);
    let log = combined_log(&replayed);
    assert!(
        !log.contains("replay desync"),
        "replaying the recording desynced: {log}"
    );
    assert!(
        replayed.status.success(),
        "replaying the recording failed ({}): {log}",
        replayed.status
    );
    assert_eq!(
        replayed_hashes(&log),
        recorded_chain,
        "playback must verify exactly the hashes the recording captured"
    );
}
