//! Per-profile multiplayer publication and privacy preferences.
//!
//! These options affect host-side network exposure only. They are deliberately
//! separate from `SimConfig`, so changing them cannot alter deterministic game
//! rules, snapshots, or replay behavior.

use serde::{Deserialize, Serialize};

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MultiplayerConfig {
    /// Advertise a signed browser invitation for newly hosted games. Native
    /// iroh may still choose a relay as a transport when this is disabled;
    /// this preference controls publication, not packet routing.
    #[serde(default = "crate::serde_defaults::enabled")]
    pub publish_browser_join_links: bool,

    /// Show teammates' cursors and broadcast this player's cursor in co-op.
    /// Presentation-only: never a simulation, replay, or snapshot input.
    #[serde(default)]
    pub show_teammate_cursors: bool,
}

impl Default for MultiplayerConfig {
    fn default() -> Self {
        Self {
            publish_browser_join_links: true,
            show_teammate_cursors: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_defaults_on_for_fresh_and_partial_profiles() {
        assert!(MultiplayerConfig::default().publish_browser_join_links);
        let partial: MultiplayerConfig = serde_json::from_str("{}").unwrap();
        assert!(partial.publish_browser_join_links);
    }

    #[test]
    fn teammate_cursors_default_off_for_fresh_and_partial_profiles() {
        assert!(!MultiplayerConfig::default().show_teammate_cursors);
        let partial: MultiplayerConfig = serde_json::from_str("{}").unwrap();
        assert!(!partial.show_teammate_cursors);
    }

    #[test]
    fn disabled_preference_roundtrips_without_becoming_default() {
        let config = MultiplayerConfig {
            publish_browser_join_links: false,
            show_teammate_cursors: false,
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            !serde_json::from_str::<MultiplayerConfig>(&json)
                .unwrap()
                .publish_browser_join_links
        );
    }
}
