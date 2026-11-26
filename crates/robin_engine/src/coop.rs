//! Deterministic cooperative session rules shared by local and network players.
use serde::{Deserialize, Serialize};

pub const MAX_PLAYERS: usize = 5;

/// Allocate the host's next cooperative-pause decision nonce.
///
/// The simulation frame is frozen while a pause holds, so a nonce derived
/// from the frame alone is reused by every toggle at that boundary and a
/// guest's duplicate guard discards the genuine resume. The last id committed
/// in the hashed simulation gate and the ids already staged for this frame
/// make the sequence monotonic while staying a pure function of deterministic
/// state — no clock or entropy enters the command stream.
pub fn next_coop_pause_decision_id(
    last_committed: u64,
    frame: u32,
    staged_ids: impl IntoIterator<Item = u64>,
) -> u64 {
    // Keep the original frame-derived seed so the first toggle still matches
    // existing replays, then advance past every id already committed or
    // staged at this boundary.
    u64::from(frame)
        .saturating_add(1)
        .max(last_committed.saturating_add(1))
        .max(
            staged_ids
                .into_iter()
                .map(|id| id.saturating_add(1))
                .max()
                .unwrap_or(0),
        )
}

/// Stable character codes shared by lobby rules and campaign construction.
pub const TEAM_CHARACTERS: &[(u8, &str, &str)] = &[
    (b'R', "Robin Hood", "Robin des bois"),
    (b'J', "Little John", "Petit Jean"),
    (b'T', "Friar Tuck", "Frere Tuck"),
    (b'S', "Stuteley", "Stutely"),
    (b'W', "Will Scarlet", "Will Ecarlate"),
    (b'M', "Marian", "Lady Marianne"),
    (b'A', "Mustached Merry", "Paysan A"),
    (b'B', "Healing Merry", "Paysan B"),
    (b'C', "Strong Merry", "Paysan C"),
];

pub fn team_character_name(code: u8) -> &'static str {
    TEAM_CHARACTERS
        .iter()
        .find(|entry| entry.0 == code)
        .expect("validated team character")
        .1
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum CharacterControl {
    #[default]
    Shared,
    Exclusive,
    Assigned,
}

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
pub struct CoopRules {
    /// Preserve story progression and the recruited roster between missions.
    #[serde(default)]
    pub campaign: bool,
    pub players: u8,
    pub control: CharacterControl,
    /// Exact party in display order. Trailing zeroes are empty slots;
    /// all zeroes retains automatic mission-party construction.
    #[serde(default)]
    pub team: [u8; MAX_PLAYERS],
    /// Roster index to copy for each missing player slot.
    pub duplicate_choices: [u8; MAX_PLAYERS],
    pub assignments: [u8; MAX_PLAYERS],
    /// Additional enemy health per duplicate, in percent; zero disables scaling.
    pub enemy_health_per_duplicate: u16,
}

impl Default for CoopRules {
    fn default() -> Self {
        Self {
            campaign: false,
            players: 1,
            control: CharacterControl::Shared,
            team: [0; MAX_PLAYERS],
            duplicate_choices: [0; MAX_PLAYERS],
            assignments: [0, 1, 2, 3, 4],
            enemy_health_per_duplicate: 25,
        }
    }
}

impl CoopRules {
    pub fn team_len(&self) -> usize {
        self.team.iter().take_while(|&&code| code != 0).count()
    }

    pub fn team_string(&self) -> Option<String> {
        (self.team_len() > 0).then(|| {
            self.team[..self.team_len()]
                .iter()
                .map(|&c| char::from(c))
                .collect()
        })
    }

    pub fn duplicate_count(&self) -> usize {
        self.team[..self.team_len()]
            .iter()
            .enumerate()
            .filter(|(i, code)| self.team[..*i].contains(code))
            .count()
    }

    /// The rules a campaign lobby carries and a campaign mission launches with.
    ///
    /// A campaign owns its roster: the party comes from the host's campaign
    /// slot, so the lobby publishes the seat count and no explicit team, and
    /// every seat shares that party. A save's own rules hold the roster it
    /// played with (`campaign` plus a team), which is exactly what
    /// [`Self::validate`] refuses for a campaign; this is the one place that
    /// converts those words into the campaign's.
    pub fn campaign_lobby(self) -> Self {
        Self {
            campaign: true,
            team: [0; MAX_PLAYERS],
            control: CharacterControl::Shared,
            ..self
        }
    }

    pub fn validate(self) -> Result<(), String> {
        if !(1..=MAX_PLAYERS as u8).contains(&self.players) {
            return Err("co-op requires one to five players".into());
        }
        let count = self.team_len();
        if self.campaign && count > 0 {
            return Err("campaign co-op uses the persistent campaign roster".into());
        }
        if self.team[count..].iter().any(|&code| code != 0)
            || self.team[..count]
                .iter()
                .any(|code| !TEAM_CHARACTERS.iter().any(|entry| entry.0 == *code))
        {
            return Err(
                "invalid co-op team: use one to five characters with trailing empty slots".into(),
            );
        }
        if count > 0 && self.control != CharacterControl::Shared && count < self.players as usize {
            return Err(
                "exclusive or assigned control requires at least one character per player".into(),
            );
        }
        if self.enemy_health_per_duplicate > 200 {
            return Err("co-op health scaling exceeds 200% per duplicate".into());
        }
        if self
            .duplicate_choices
            .iter()
            .any(|&choice| choice >= MAX_PLAYERS as u8)
        {
            return Err("invalid co-op character choice".into());
        }
        let mut assignments = self.assignments;
        assignments.sort_unstable();
        if assignments != [0, 1, 2, 3, 4] {
            return Err("co-op assignments must be a permutation of player slots".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_coop_pause_toggles_at_a_frozen_frame_get_distinct_nonces() {
        let frozen_frame = 8_994;
        let first = next_coop_pause_decision_id(0, frozen_frame, []);
        let second = next_coop_pause_decision_id(first, frozen_frame, []);
        let third = next_coop_pause_decision_id(second, frozen_frame, []);
        assert_ne!(
            first, second,
            "a resume at the frozen frame must not reuse the pause's id"
        );
        assert_ne!(second, third, "every genuine toggle needs its own id");
        assert!(second > first && third > second, "nonces must be monotonic");
    }

    #[test]
    fn coop_pause_nonce_accounts_for_decisions_staged_in_the_same_frame() {
        let next = next_coop_pause_decision_id(7, 2, [9, 11]);
        assert_eq!(
            next, 12,
            "a second press before the frame drains must not reuse the first's id"
        );
    }

    #[test]
    fn exact_team_allows_repeats_and_is_independent_of_shared_player_count() {
        let rules = CoopRules {
            players: 5,
            team: [b'R', b'R', 0, 0, 0],
            ..Default::default()
        };
        assert!(rules.validate().is_ok());
        assert_eq!(rules.team_string().as_deref(), Some("RR"));
        assert_eq!(rules.duplicate_count(), 1);
        for control in [CharacterControl::Exclusive, CharacterControl::Assigned] {
            assert!(CoopRules { control, ..rules }.validate().is_err());
        }
        let full = CoopRules {
            team: *b"RRTTM",
            ..rules
        };
        assert!(full.validate().is_ok());
        assert_eq!(full.duplicate_count(), 2);
        assert_eq!(
            bitcode::decode::<CoopRules>(&bitcode::encode(&full)).unwrap(),
            full
        );
        assert_eq!(
            serde_json::from_str::<CoopRules>(&serde_json::to_string(&full).unwrap()).unwrap(),
            full
        );
        assert!(
            CoopRules {
                team: [b'R', 0, b'T', 0, 0],
                ..rules
            }
            .validate()
            .is_err()
        );
        assert!(
            CoopRules {
                team: [b'?', 0, 0, 0, 0],
                ..rules
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn cooperative_rules_validate_player_count_and_unique_assignments() {
        let mut rules = CoopRules::default();
        for players in 1..=5 {
            rules.players = players;
            assert!(rules.validate().is_ok());
        }
        rules.players = 6;
        assert!(rules.validate().is_err());
        rules.players = 2;
        rules.assignments = [0, 0, 2, 3, 4];
        assert!(rules.validate().is_err());
    }

    /// A campaign save plays with the roster it recorded, so its own rules are
    /// `campaign` plus a team — the one shape a campaign lobby may not carry.
    /// The lobby words are derived from them, and those validate.
    #[test]
    fn campaign_lobby_rules_drop_the_saved_party_and_keep_the_seats() {
        let saved = CoopRules {
            campaign: true,
            players: 2,
            control: CharacterControl::Assigned,
            team: [b'R', b'J', 0, 0, 0],
            enemy_health_per_duplicate: 50,
            ..Default::default()
        };
        assert!(saved.validate().is_err());
        let lobby = saved.campaign_lobby();
        assert!(lobby.validate().is_ok());
        assert!(lobby.campaign);
        assert_eq!(lobby.team_len(), 0);
        assert_eq!(lobby.players, 2);
        assert_eq!(lobby.control, CharacterControl::Shared);
        assert_eq!(lobby.enemy_health_per_duplicate, 50);
    }
}
