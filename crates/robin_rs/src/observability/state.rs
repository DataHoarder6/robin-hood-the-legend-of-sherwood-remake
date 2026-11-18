//! The last `/state` answer this process produced, and why it stopped
//! producing new ones.
//!
//! `/state` is written for the moment a session is broken: the mission is over,
//! its RPC session is gone, and the only useful answer is the state it last
//! reached plus the operation that ended it. The snapshot is published at every
//! RPC boundary, so the answer is at most one frame behind the live engine and
//! survives the mission that produced it.

use serde::Serialize;
use serde_json::Value;

/// Why this process stopped answering `/state` from a live mission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Terminal {
    /// The `GameCode` the mission ended on, when it got that far. A launch
    /// failure has no operation and reports `null`.
    pub(crate) operation: Option<String>,
    pub(crate) reason: String,
    pub(crate) at: String,
}

#[derive(Default)]
pub(super) struct StateSurface {
    snapshot: Option<Value>,
    terminal: Option<Terminal>,
}

impl StateSurface {
    /// A mission's first RPC boundary starts a new narrative: the previous
    /// mission's terminal block no longer describes this process.
    pub(super) fn mission_started(&mut self) {
        self.terminal = None;
    }

    pub(super) fn publish(&mut self, snapshot: Value) {
        self.snapshot = Some(snapshot);
    }

    /// Record the end of a mission. The operation survives from the last
    /// published snapshot when the mission died without reporting one.
    pub(super) fn record_terminal(&mut self, operation: Option<String>, reason: String) {
        let operation = operation.or_else(|| self.published_operation());
        self.terminal = Some(Terminal {
            operation,
            reason,
            at: super::events_utc_now(),
        });
    }

    fn published_operation(&self) -> Option<String> {
        self.snapshot
            .as_ref()?
            .get("multiplayer")?
            .get("final_operation")?
            .as_str()
            .map(str::to_owned)
    }

    /// The `/state` document: the last snapshot, the terminal block, and where
    /// this run's evidence lives. `None` before this process has attempted a
    /// mission, which is the one honest "nothing to report yet".
    pub(super) fn document(&self) -> Option<Value> {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return self.terminal.as_ref().map(|terminal| {
                let mut document = serde_json::json!({
                    "frame": null,
                    "map": null,
                    "replay": null,
                    "multiplayer": null,
                });
                document["terminal"] =
                    serde_json::to_value(terminal).expect("terminal block serializes");
                document
            });
        };
        let mut document = snapshot.clone();
        document["terminal"] = match self.terminal.as_ref() {
            Some(terminal) => serde_json::to_value(terminal).expect("terminal block serializes"),
            None => Value::Null,
        };
        Some(document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(frame: u32, operation: &str) -> Value {
        serde_json::json!({
            "frame": frame,
            "map": "Dem_Lei",
            "replay": null,
            "multiplayer": {
                "local_seat": 0,
                "expected_players": 2,
                "admission": "host_waiting_for_begin",
                "begin_sim_frame": null,
                "pending_transition": null,
                "last_compared_hash_frame": null,
                "desync_count": 0,
                "final_operation": operation,
                "connect_string": "endpoint",
            },
        })
    }

    #[test]
    fn a_live_mission_reports_its_snapshot_with_a_null_terminal() {
        let mut surface = StateSurface::default();
        assert!(surface.document().is_none());
        surface.publish(snapshot(120, "LevelInProgress"));
        let document = surface.document().unwrap();
        assert_eq!(document["frame"], 120);
        assert_eq!(document["terminal"], Value::Null);
    }

    #[test]
    fn a_terminal_operation_keeps_the_last_snapshot_and_names_its_operation() {
        let mut surface = StateSurface::default();
        surface.publish(snapshot(240, "LevelInProgress"));
        surface.record_terminal(Some("Quit".into()), "mission ended".into());
        let document = surface.document().unwrap();
        assert_eq!(document["frame"], 240);
        assert_eq!(
            document["multiplayer"]["admission"],
            "host_waiting_for_begin"
        );
        assert_eq!(document["terminal"]["operation"], "Quit");
        assert_eq!(document["terminal"]["reason"], "mission ended");
    }

    #[test]
    fn a_mission_that_never_published_still_answers_with_its_failure() {
        let mut surface = StateSurface::default();
        surface.record_terminal(None, "mission launch failed: no game data".into());
        let document = surface.document().unwrap();
        assert!(document["frame"].is_null());
        assert_eq!(
            document["terminal"]["reason"],
            "mission launch failed: no game data"
        );
    }

    #[test]
    fn a_dead_mission_names_the_operation_it_died_in() {
        let mut surface = StateSurface::default();
        surface.publish(snapshot(75, "LevelInProgress"));
        surface.record_terminal(None, "admission timeout".into());
        assert_eq!(
            surface.document().unwrap()["terminal"]["operation"],
            "LevelInProgress"
        );
    }

    #[test]
    fn a_new_mission_clears_the_previous_terminal_block() {
        let mut surface = StateSurface::default();
        surface.publish(snapshot(10, "LevelInProgress"));
        surface.record_terminal(Some("LevelFailed".into()), "mission ended".into());
        surface.mission_started();
        assert_eq!(surface.document().unwrap()["terminal"], Value::Null);
    }
}
