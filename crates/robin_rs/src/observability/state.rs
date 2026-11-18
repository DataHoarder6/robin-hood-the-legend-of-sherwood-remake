//! The last `/state` answer this process produced, and why it stopped
//! producing new ones.
//!
//! `/state` is written for the moment a session is broken: the mission is over,
//! its RPC session is gone, and the only useful answer is the state it last
//! reached plus the operation that ended it. The snapshot is published at every
//! RPC boundary, so the answer is at most one frame behind the live engine and
//! survives the mission that produced it.
//!
//! One generation per mission attempt is the single owner of that narrative:
//! a snapshot describes only the attempt that published it, an attempt records
//! one terminal, and a new attempt starts without either.

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

/// One attempt's snapshot, stamped with the generation that published it so it
/// can never describe a later attempt.
struct Published {
    generation: u64,
    document: Value,
}

#[derive(Default)]
pub(super) struct StateSurface {
    /// Advanced by [`StateSurface::mission_started`]. Generation zero is the
    /// window before this process has attempted a mission.
    generation: u64,
    snapshot: Option<Published>,
    terminal: Option<Terminal>,
    /// The generation whose terminal is already recorded. One outcome records
    /// once, whichever writer reaches the surface first.
    terminal_generation: Option<u64>,
}

impl StateSurface {
    /// A mission attempt starts: the previous attempt's terminal block and
    /// snapshot no longer describe this process.
    pub(super) fn mission_started(&mut self) {
        self.generation += 1;
        self.terminal = None;
        self.terminal_generation = None;
    }

    pub(super) fn publish(&mut self, snapshot: Value) {
        self.snapshot = Some(Published {
            generation: self.generation,
            document: snapshot,
        });
    }

    /// Record the end of this attempt. Returns whether it recorded: a second
    /// writer for the same generation changes nothing and emits nothing.
    ///
    /// The operation survives from this generation's last published snapshot
    /// when the attempt died without reporting one; a snapshot from an earlier
    /// mission is never inherited.
    pub(super) fn record_terminal(&mut self, operation: Option<String>, reason: String) -> bool {
        if self.terminal_generation == Some(self.generation) {
            return false;
        }
        let operation = operation.or_else(|| self.published_operation());
        self.terminal = Some(Terminal {
            operation,
            reason,
            at: super::events_utc_now(),
        });
        self.terminal_generation = Some(self.generation);
        true
    }

    fn published_operation(&self) -> Option<String> {
        self.published()?
            .get("multiplayer")?
            .get("final_operation")?
            .as_str()
            .map(str::to_owned)
    }

    fn published(&self) -> Option<&Value> {
        self.snapshot
            .as_ref()
            .filter(|snapshot| snapshot.generation == self.generation)
            .map(|snapshot| &snapshot.document)
    }

    /// The `/state` document: this attempt's snapshot, the terminal block, and
    /// where this run's evidence lives. `None` only before the process has
    /// attempted a mission, which is the one honest "nothing to report yet".
    pub(super) fn document(&self) -> Option<Value> {
        let published = self.published();
        if published.is_none() && self.terminal.is_none() && self.generation == 0 {
            return None;
        }
        let mut document = match published {
            Some(snapshot) => snapshot.clone(),
            // An attempt that has not reached its first boundary yet: the
            // previous mission's frame and map describe nothing this process is
            // doing, so they are reported as absent rather than as current.
            None => serde_json::json!({
                "frame": null,
                "map": null,
                "replay": null,
                "multiplayer": null,
            }),
        };
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
        assert!(surface.record_terminal(Some("Quit".into()), "mission ended".into()));
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

    #[test]
    fn one_outcome_records_once_and_a_later_attempt_inherits_nothing() {
        let mut surface = StateSurface::default();
        surface.mission_started();
        surface.publish(snapshot(240, "LevelInProgress"));
        assert!(surface.record_terminal(Some("Quit".into()), "mission ended".into()));
        assert!(
            !surface.record_terminal(None, "launch failed: drain".into()),
            "the attempt already recorded its outcome"
        );
        assert_eq!(
            surface.document().unwrap()["terminal"]["reason"],
            "mission ended"
        );

        surface.mission_started();
        assert!(surface.record_terminal(None, "launch failed: no game data".into()));
        let document = surface.document().unwrap();
        assert!(document["frame"].is_null(), "{document}");
        assert!(document["multiplayer"].is_null(), "{document}");
        assert!(document["terminal"]["operation"].is_null(), "{document}");
    }
}
