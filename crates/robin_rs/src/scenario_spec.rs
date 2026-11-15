//! L2 scenario specs: a mission driven by a JSON script of player commands.
//!
//! A mod ships a spec; the fork provides the runner (`--headless --spec <PATH>`).
//! See `docs/TESTING.md`, "Scenario specs (L2)". The runner stages each command
//! into the frame the spec names, exactly like live input and the `/command`
//! RPC do, and exits when the frame budget completes. Assertions on engine
//! state are not part of this layer yet: a spec's outcome is pinned by the
//! state-hash surface (L3) instead.
//!
//! ```json
//! {
//!   "frames": 600,
//!   "commands": [
//!     { "frame": 30, "command": { "StopPc": { "pc_id": { "Pc": 252 } } } }
//!   ]
//! }
//! ```

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use robin_engine::player_command::PlayerCommand;
use serde::{Deserialize, Serialize};

/// One headless run of a mission: a frame budget plus timed player commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionSpec {
    /// Simulated frames to run. Valid command frames are `0 .. frames`.
    pub frames: u32,
    /// Commands in file order; `frame` must not decrease.
    #[serde(default)]
    pub commands: Vec<SpecCommand>,
}

/// One command the spec issues at one simulated frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecCommand {
    /// Zero-based simulated frame, counted from mission start.
    pub frame: u32,
    /// Externally tagged [`PlayerCommand`] JSON, the same shape `POST /command`
    /// accepts (`{"SelectPc":{"pc_id":{"Pc":1},"append":false}}`).
    pub command: PlayerCommand,
}

impl MissionSpec {
    /// Read and validate a spec file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read scenario spec {}", path.display()))?;
        let spec: Self = serde_json::from_str(&text)
            .with_context(|| format!("parse scenario spec {} as JSON", path.display()))?;
        spec.validate()
            .with_context(|| format!("invalid scenario spec {}", path.display()))?;
        Ok(spec)
    }

    /// Reject specs the runner cannot honour.
    fn validate(&self) -> Result<()> {
        if self.frames == 0 {
            bail!("`frames` must be at least 1");
        }
        let mut previous = None;
        for (index, entry) in self.commands.iter().enumerate() {
            if entry.frame >= self.frames {
                bail!(
                    "command {index} is scheduled for frame {} but `frames` is {}; \
                     the command would never run",
                    entry.frame,
                    self.frames
                );
            }
            if let Some(previous) = previous
                && entry.frame < previous
            {
                bail!(
                    "command {index} is scheduled for frame {} after frame {previous}; \
                     commands must be listed in non-decreasing frame order",
                    entry.frame
                );
            }
            previous = Some(entry.frame);
        }
        Ok(())
    }

    /// Take ownership as a runner cursor.
    pub(crate) fn into_run(self) -> ScenarioRun {
        ScenarioRun {
            spec: self,
            next_command: 0,
            frames_run: 0,
            commands_applied: 0,
            completed: false,
        }
    }
}

/// Per-run cursor over a [`MissionSpec`]: which frame is being simulated and
/// which commands are due.
///
/// Frame accounting counts *simulated* frames only; a paused tick does not
/// consume a spec frame and does not leave a due command behind.
pub(crate) struct ScenarioRun {
    spec: MissionSpec,
    next_command: usize,
    frames_run: u32,
    commands_applied: u32,
    completed: bool,
}

impl ScenarioRun {
    /// Zero-based index of the frame about to be simulated.
    pub(crate) fn current_frame(&self) -> u32 {
        self.frames_run
    }

    /// Pop the next command scheduled for the current frame.
    ///
    /// Every command of a frame is returned in file order before the frame
    /// advances; `None` means the current frame has no further commands.
    pub(crate) fn take_due(&mut self) -> Option<PlayerCommand> {
        let entry = self.spec.commands.get(self.next_command)?;
        if entry.frame != self.frames_run {
            return None;
        }
        self.next_command += 1;
        self.commands_applied += 1;
        Some(entry.command.clone())
    }

    /// Account for one finished frame; returns `true` on the frame that
    /// completes the frame budget, exactly once.
    pub(crate) fn finish_frame(&mut self, advanced: bool) -> bool {
        if !advanced || self.completed {
            return self.completed;
        }
        self.frames_run += 1;
        if self.frames_run < self.spec.frames {
            return false;
        }
        assert_eq!(
            self.next_command,
            self.spec.commands.len(),
            "scenario spec ran all {} frames without applying every command",
            self.spec.frames
        );
        self.completed = true;
        tracing::info!(
            "scenario spec complete: {} frames, {} commands applied",
            self.frames_run,
            self.commands_applied
        );
        true
    }

    pub(crate) fn is_completed(&self) -> bool {
        self.completed
    }

    pub(crate) fn total_frames(&self) -> u32 {
        self.spec.frames
    }
}

#[cfg(test)]
mod tests {
    use super::MissionSpec;
    use robin_engine::element::{EntityId, PcId};
    use robin_engine::player_command::PlayerCommand;

    fn load(body: &str) -> anyhow::Result<MissionSpec> {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("spec.json");
        std::fs::write(&path, body).expect("write spec");
        MissionSpec::load(&path)
    }

    #[test]
    fn a_spec_parses_commands_with_the_rpc_entity_shape() {
        let spec = load(
            r#"{
              "frames": 600,
              "commands": [
                { "frame": 30, "command": { "StopPc": { "pc_id": { "Pc": 252 } } } },
                { "frame": 30, "command": { "StopPc": { "pc_id": { "Pc": 7 } } } }
              ]
            }"#,
        )
        .expect("valid spec");

        assert_eq!(spec.frames, 600);
        assert_eq!(spec.commands.len(), 2);
        assert_eq!(spec.commands[0].frame, 30);
        assert!(matches!(
            &spec.commands[0].command,
            PlayerCommand::StopPc { pc_id } if *pc_id == EntityId::Pc(PcId(252))
        ));
    }

    #[test]
    fn an_empty_command_list_is_legal() {
        let spec = load(r#"{"frames": 5}"#).expect("valid spec");
        assert_eq!(spec.frames, 5);
        assert!(spec.commands.is_empty());
    }

    #[test]
    fn a_zero_frame_budget_is_rejected() {
        let error = load(r#"{"frames": 0}"#).expect_err("frames must be positive");
        assert!(
            format!("{error:#}").contains("`frames` must be at least 1"),
            "{error:#}"
        );
    }

    #[test]
    fn a_command_past_the_frame_budget_is_rejected() {
        let error = load(
            r#"{"frames": 10, "commands": [
                 { "frame": 10, "command": "CrouchDown" }
               ]}"#,
        )
        .expect_err("command at `frames` never runs");
        assert!(
            format!("{error:#}").contains("would never run"),
            "{error:#}"
        );
    }

    #[test]
    fn commands_out_of_frame_order_are_rejected() {
        let error = load(
            r#"{"frames": 60, "commands": [
                 { "frame": 30, "command": "CrouchDown" },
                 { "frame": 12, "command": "CrouchDown" }
               ]}"#,
        )
        .expect_err("frame order is part of the format");
        assert!(
            format!("{error:#}").contains("non-decreasing frame order"),
            "{error:#}"
        );
    }

    #[test]
    fn unknown_fields_and_bad_json_are_rejected() {
        let unknown = load(r#"{"frames": 5, "assertions": []}"#).expect_err("unknown field");
        assert!(
            format!("{unknown:#}").contains("unknown field"),
            "{unknown:#}"
        );

        let malformed = load(r#"{"frames": 5,"#).expect_err("truncated json");
        assert!(
            format!("{malformed:#}").contains("as JSON"),
            "{malformed:#}"
        );
    }

    #[test]
    fn the_runner_stages_each_command_on_its_own_frame_in_file_order() {
        let spec = load(
            r#"{"frames": 3, "commands": [
                 { "frame": 1, "command": "CrouchDown" },
                 { "frame": 1, "command": "StandUp" },
                 { "frame": 2, "command": "CrouchDown" }
               ]}"#,
        )
        .expect("valid spec");
        let mut run = spec.into_run();

        assert_eq!(run.current_frame(), 0);
        assert!(run.take_due().is_none(), "frame 0 has no commands");
        assert!(!run.finish_frame(true));

        assert_eq!(run.current_frame(), 1);
        assert!(matches!(run.take_due(), Some(PlayerCommand::CrouchDown)));
        assert!(matches!(run.take_due(), Some(PlayerCommand::StandUp)));
        assert!(run.take_due().is_none());
        assert!(!run.finish_frame(false), "a paused tick consumes no frame");
        assert_eq!(run.current_frame(), 1);
        assert!(!run.finish_frame(true));

        assert_eq!(run.current_frame(), 2);
        assert!(matches!(run.take_due(), Some(PlayerCommand::CrouchDown)));
        assert!(run.take_due().is_none());
        assert!(run.finish_frame(true), "frame 3 completes the budget");
        assert!(run.is_completed());
        assert_eq!(run.total_frames(), 3);
    }
}
