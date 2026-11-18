//! In-game menus: pause, dialogue, debriefing, mission state, options,
//! graphics, sounds, shortcuts.
//!
//! Used during gameplay (as opposed to the main menu, which lives in
//! [`crate::main_menu`]).  Contains:
//!
//! - [`resources`] — per-mission resource cache: DEFAULT.RES button sprites,
//!   window backgrounds, parchments, native bitmap fonts and the menu text
//!   string table.
//! - [`layout`] — virtual 640x480 coordinate helpers, button/widget layout,
//!   text rendering and wrapping / pagination helpers.
//! - [`pause`] — the in-game pause menu, including campaign history access.
//! - [`briefings`] — renders the primary/secondary objective list in the
//!   left half of the pause window.
//! - [`yesno`] — the 400x200 modal confirmation.
//! - [`mission_state`] — the mission start/won/lost/aborted transient popup with
//!   zoom transition and Yes/No confirm.
//! - [`debriefing`] — the post-mission parchment with justified text,
//!   pagination and Restart / Load / OK buttons.
//! - [`dialogue`] — character dialogues with per-sentence audio, mouth
//!   animation and auto-advance timer.
//! - [`options`] — the in-mission options hub.
//! - [`perks_panel`] — the Sherwood permanent-perk purchase panel.
//! - [`graphics`] — resolution + visual toggles.
//! - [`sounds`] — stereo/EAX radios and volume sliders.
//! - [`shortcuts`] — keyboard binding editor with click-to-rebind mode.

pub mod blazon_set;
pub mod briefings;
pub mod buy_blazons;
pub mod debriefing;
pub mod dialogue;
pub mod gameplay;
pub mod graphics;
pub mod language;
pub mod layout;
pub mod leaderboard;
pub mod leaderboard_settings;
pub mod mission_description;
pub mod mission_state;
pub mod modal_net;
pub mod multiplayer_privacy;
pub mod options;
pub mod pause;
pub mod perks_panel;
pub mod popup_scroll;
pub mod resources;
pub mod save_load;
mod save_picker;
pub mod shortcuts;
pub mod sounds;
pub mod spellforge_content;
pub mod trading;
pub mod widget_bridge;
pub mod yesno;

pub use buy_blazons::BuyBlazonsOutcome;
pub use debriefing::{DebriefingContent, DebriefingModalState, DebriefingOutcome, show_debriefing};
pub(crate) use dialogue::show_dialogue_batch;
pub use dialogue::{
    BatchDialogue, DIALOGUE_PORTRAIT_IDS, DialogueAudio, DialogueIo, DialogueModalState,
    DialogueSentence, show_dialogue,
};
pub use layout::{MENU_H, MENU_W, MenuButton, MenuTransform};
pub use leaderboard::MissionEndLeaderboardScreen;
pub use mission_state::MissionStatePopupState;
pub use modal_net::ModalNet;
pub use options::{OptionsOutcome, OptionsTargets, show_options};
pub use pause::{PauseMenu, PauseMenuOutcome};
pub use perks_panel::{PerksPanelModalState, PerksPanelOutcome, campaign_rows};
pub(crate) use popup_scroll::show_popup_scroll;
pub use popup_scroll::{PopupScrollItem, PopupScrollModalState};
pub use resources::{IngameMenuResources, MenuSurface};
pub use save_load::{
    SaveLoadMode, SaveLoadOutcome, SavePickerConfig, SavePickerModalState, show_load_picker,
};
pub use trading::{TradingModalState, TradingOutcome};
pub use yesno::{YesNoModalState, show_yesno};

/// Terminal result from a one-frame menu state.
///
/// `ExitRequested` is deliberately distinct from cancellation: closing the
/// native window must propagate out of nested pause-side screens instead of
/// quietly returning to gameplay.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModalScreenOutcome<T> {
    Accepted(T),
    Cancelled,
    ExitRequested,
}

use robin_engine::game_operation::GameCode;

use resources::{MT_TTL_MISSION_ABORTED, MT_TTL_MISSION_LOST, MT_TTL_MISSION_WON};

/// Which original mission-end page a mission presents.
///
/// The retail menu table carries one title per outcome — "Mission won",
/// "Mission lost" and "Mission aborted" at ids 24–26 — and an abandoned
/// mission is the third, not a lost one. The level's win/lose debriefing
/// tables are unchanged: an aborted mission reads the lose table, whose
/// script-selected entry (`ChooseVictoryDefeatText`) carries the retreat
/// text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MissionEndKind {
    Won,
    #[default]
    Lost,
    Aborted,
}

impl MissionEndKind {
    pub fn from_exit_code(code: GameCode) -> Option<Self> {
        match code {
            GameCode::LevelSucceeded => Some(Self::Won),
            GameCode::LevelFailed => Some(Self::Lost),
            GameCode::LevelInterrupted => Some(Self::Aborted),
            GameCode::LevelInProgress
            | GameCode::LevelNext
            | GameCode::LevelRestart
            | GameCode::Quit
            | GameCode::LevelLoad
            | GameCode::LevelSave => None,
        }
    }

    /// A surface that only distinguishes victory from defeat (scripted
    /// debriefings, pseudo-mission reports) keeps its two-way meaning.
    pub fn from_won(won: bool) -> Self {
        if won { Self::Won } else { Self::Lost }
    }

    pub fn is_won(self) -> bool {
        self == Self::Won
    }

    /// Retail menu-text id for the page title.
    pub fn title_id(self) -> usize {
        match self {
            Self::Won => MT_TTL_MISSION_WON,
            Self::Lost => MT_TTL_MISSION_LOST,
            Self::Aborted => MT_TTL_MISSION_ABORTED,
        }
    }

    /// Line shown in the mission-state confirmation before the debriefing.
    pub fn popup_message(self) -> &'static str {
        match self {
            Self::Won => "Mission Won",
            Self::Lost => "Mission Lost",
            Self::Aborted => "Mission Abandoned",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mission_end_kind_comes_from_the_exit_code() {
        assert_eq!(
            MissionEndKind::from_exit_code(GameCode::LevelSucceeded),
            Some(MissionEndKind::Won)
        );
        assert_eq!(
            MissionEndKind::from_exit_code(GameCode::LevelFailed),
            Some(MissionEndKind::Lost)
        );
        assert_eq!(
            MissionEndKind::from_exit_code(GameCode::LevelInterrupted),
            Some(MissionEndKind::Aborted)
        );
        assert_eq!(
            MissionEndKind::from_exit_code(GameCode::LevelInProgress),
            None
        );
    }

    #[test]
    fn mission_end_titles_are_the_three_retail_ids() {
        assert_eq!(MissionEndKind::Won.title_id(), 24);
        assert_eq!(MissionEndKind::Lost.title_id(), 25);
        assert_eq!(MissionEndKind::Aborted.title_id(), 26);
        assert!(!MissionEndKind::Aborted.is_won());
    }
}
