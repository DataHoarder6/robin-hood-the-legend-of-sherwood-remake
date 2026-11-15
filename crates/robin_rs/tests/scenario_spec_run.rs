#![cfg(all(not(target_arch = "wasm32"), unix))]

//! L2 scenario spec (see `docs/TESTING.md`, "Scenario specs (L2)"): the
//! committed fixture drives a real mission headless through `--spec`, and the
//! run must end on its own frame budget with every command in its scheduled
//! frame.
//!
//! The asserted surfaces are the process exit status and the mission recording
//! the engine already writes. The runner's own INFO report is not asserted: a
//! headless process can exit before its final stderr line reaches a redirected
//! stream (the pre-existing `headless replay finished` line is lost the same
//! way), so the report stays a human-facing nicety.
//!
//! Opt-in, because it needs a built `robin`, licensed game data and an X
//! display for the GL backend:
//!
//! ```text
//! ROBINHOOD_DATA_DIR=/path/to/robin-hood/game DISPLAY=:11 WGPU_BACKEND=gl \
//!   cargo test -p robin_rs --test scenario_spec_run -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Kept in step with `tests/fixtures/scenario_spec_s01.json`.
const SPEC: &str = include_str!("fixtures/scenario_spec_s01.json");
const SPEC_FRAMES: u32 = 60;
/// `(frame, command)` in spec order.
const SPEC_COMMANDS: [(u32, &str); 2] = [(10, "CrouchDown"), (20, "StandUp")];
const MISSION: &str = "S01_Not_VL";
/// `GameCode::LevelInterrupted`, the spec's "budget completed" result.
const SPEC_COMPLETED_EXIT: i32 = 3;

fn data_dir() -> PathBuf {
    let path = std::env::var_os("ROBINHOOD_DATA_DIR").expect(
        "ROBINHOOD_DATA_DIR must point at an install with licensed game data (e.g. \
         /home/admin/Games/robin-hood/game)",
    );
    PathBuf::from(path)
}

fn run_spec(spec_path: &Path, record_dir: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_robin"));
    command
        .args(["--headless", "--no-sound", "--mission", MISSION])
        .arg("--spec")
        .arg(spec_path)
        .env("ROBINHOOD_DATA_DIR", data_dir());
    if let Some(dir) = record_dir {
        command.arg("--record").arg(dir);
    }
    command.output().expect("run robin headless with --spec")
}

fn combined_log(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// `(frame, [command names])` for every recorded frame, oldest first.
///
/// The recording is the engine's own command journal, so it shows exactly
/// which frame each staged command reached.
fn recorded_frames(record_dir: &Path) -> Vec<(u32, Vec<String>)> {
    let chunk = std::fs::read_dir(record_dir)
        .expect("recording directory")
        .map(|entry| entry.expect("recording entry").path())
        .find(|path| path.to_string_lossy().ends_with(".rhrec.jsonl"))
        .expect("recording chunk");
    let text = std::fs::read_to_string(&chunk).expect("read recording chunk");
    // Rows are the recorder's per-frame envelope: `f` is the frame ordinal and
    // `i.input.commands` is exactly the command stage that frame executed.
    let mut frames: Vec<(u32, Vec<String>)> = Vec::new();
    for line in text.lines() {
        let row: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|error| panic!("{line}: {error}"));
        let Some(input) = row
            .get("i")
            .and_then(|input| input.get("input"))
            .and_then(|input| input.get("commands"))
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        let frame = row
            .get("f")
            .and_then(serde_json::Value::as_u64)
            .expect("recorded input row carries its frame") as u32;
        let commands = input
            .iter()
            .map(|command| {
                command
                    .get("command")
                    .expect("recorded command")
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| command.to_string())
            })
            .collect();
        frames.push((frame, commands));
    }
    frames.sort_by_key(|(frame, _)| *frame);
    assert!(
        !frames.is_empty(),
        "the recorder wrote no frame rows to {}",
        chunk.display()
    );
    frames
}

#[test]
#[ignore = "needs licensed game data, an X display and the built robin binary"]
fn spec_commands_are_staged_at_their_frames_and_the_run_ends_after_its_budget() {
    let directory = tempfile::tempdir().expect("temp dir");
    let spec_path = directory.path().join("s01-scenario.json");
    std::fs::write(&spec_path, SPEC).expect("write scenario spec");
    let record_dir = directory.path().join("recording");

    let output = run_spec(&spec_path, Some(&record_dir));
    let log = combined_log(&output);
    assert_eq!(
        output.status.code(),
        Some(SPEC_COMPLETED_EXIT),
        "spec run did not report completion: {log}"
    );

    let frames = recorded_frames(&record_dir);
    let last = frames.last().expect("recorded frames").0;
    assert_eq!(
        last + 1,
        SPEC_FRAMES,
        "the run must stop on its frame budget, recorded frames: {last}"
    );
    let mut expected = SPEC_COMMANDS.iter();
    let mut applied = 0;
    for (frame, commands) in &frames {
        for command in commands {
            let (want_frame, want_command) = expected
                .next()
                .unwrap_or_else(|| panic!("extra command {command:?} at frame {frame}"));
            assert_eq!(
                (*frame, command.as_str()),
                (*want_frame, *want_command),
                "commands must reach exactly their scheduled frame, in file order"
            );
            applied += 1;
        }
    }
    assert_eq!(applied, SPEC_COMMANDS.len(), "not every command was staged");
}

#[test]
#[ignore = "needs licensed game data, an X display and the built robin binary"]
fn a_spec_command_that_never_runs_is_rejected_before_the_mission_loads() {
    let directory = tempfile::tempdir().expect("temp dir");
    let spec_path = directory.path().join("broken-scenario.json");
    std::fs::write(
        &spec_path,
        r#"{"frames": 30, "commands": [{"frame": 30, "command": "CrouchDown"}]}"#,
    )
    .expect("write scenario spec");

    let output = run_spec(&spec_path, None);
    let log = combined_log(&output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a broken spec must fail: {log}"
    );
    assert!(
        log.contains("would never run"),
        "the error must name the unreachable command: {log}"
    );
    assert!(
        !log.contains("scenario spec complete"),
        "a rejected spec must not report a completed run: {log}"
    );
}
