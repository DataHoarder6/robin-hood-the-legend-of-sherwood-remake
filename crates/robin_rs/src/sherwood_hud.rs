//! Sherwood-mission HUD buttons (DisplayCampaignMap, GoToExit,
//! StartMission, QuitMission, SherwoodTrading, PerksPanel).
//!
//! Only the logical layout + hit-test + enabled-state machine is
//! implemented here; visual polish (proper sprite backgrounds, tooltips,
//! blink animations) is a separate pass driven by the widget module.
//!
//! The buttons drive the Sherwood flow defined in `run_mission`:
//!
//! - `DisplayCampaignMap` re-raises the campaign-map overlay (the
//!   same modal shown on Sherwood entry).  Enabled while the player
//!   hasn't yet committed to a next mission.
//! - `GoToExit` centers the camera on the current mission's exit
//!   gate.  Enabled once a mission has been committed via the
//!   overlay.
//! - `StartMission` re-triggers the confirm dialog for the already
//!   selected next mission — without having to re-open the map.
//!   Enabled once a mission has been committed.
//! - `QuitMission` opens the `REALLY_RETURN_TO_MAP` confirm dialog
//!   to bail back to the campaign-map overlay.  Always enabled in
//!   Sherwood once the player has picked a mission (so they can
//!   back out).

use crate::gfx_types::{Point, Rect as ScreenRect};

use crate::hud_sprite::HudButton;
use crate::ingame_menu::layout::{BTN_STATE_HOVER, BTN_STATE_NORMAL, button_sprite_state};
use crate::renderer::Renderer;
use robin_engine::resource_ids::{
    RHID_CLOVER, RHID_CONVERT_MONEY_TO_BLAZONS, RHID_DISPLAY_CAMPAIGN_MAP, RHID_FLOATING_CANCEL,
    RHID_FLOATING_OK, RHID_GO_TO_EXIT,
};

/// Logical Sherwood button id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SherwoodButton {
    /// Re-raise the campaign map.
    DisplayCampaignMap,
    /// Centre the camera on the next mission's exit gate.
    GoToExit,
    /// Confirm the committed mission selection.
    StartMission,
    /// Return to the campaign map.
    QuitMission,
    /// Open the post-port Sherwood inventory-trading panel.
    SherwoodTrading,
    /// Open the permanent-perk purchase panel.
    PerksPanel,
}

/// A press that landed on a rail control, with the multi-click count the
/// window handler derived from press timing.
///
/// The count arrives in `GameEvent::MouseDown`'s fourth field, produced by
/// `window::AppHandler::handle_mouse_input` from
/// `window::DOUBLE_CLICK_INTERVAL_MS`; there is no second timing
/// mechanism here. Every HUD command whose original has a walk/run pair picks
/// its run variant from [`Self::is_double`], the same rule the world path
/// applies to the identical count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SherwoodHudClick {
    pub button: SherwoodButton,
    /// 1 for a single press; 2 when the window recognised a double click.
    pub count: u8,
}

impl SherwoodHudClick {
    pub fn new(button: SherwoodButton, count: u8) -> Self {
        Self { button, count }
    }

    /// Mirrors `InputState::press_left_pointer`'s rule: everything from the
    /// second press of the pair is the double-click press.
    pub fn is_double(self) -> bool {
        self.count >= 2
    }
}

/// Which controls the rail offers.
///
/// Presence is a property of the situation, derived from campaign state every
/// frame — not a hand-maintained mirror of it. Clickability stays in
/// [`SherwoodButtonEnable`]: an offered control whose gate is closed is drawn
/// disabled and still owns its pixels. One exception: the Sherwood
/// `StartMission` gate is the mission-requirements (or blazon-conversion)
/// check, and the user must not be offered a start they cannot perform — so a
/// closed requirements gate removes that button from the rail entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SherwoodRail {
    /// Sherwood, no next mission committed.
    PreCommit,
    /// Sherwood, next mission committed and its start gate is open.
    PostCommit,
    /// Sherwood, next mission committed but its requirements are unmet:
    /// `StartMission` is not rendered at all.
    PostCommitStartNotReady,
    /// A mission being played. `can_quit` is false in the first two campaign
    /// missions, which have no route back to Sherwood: the red abandon control
    /// would be a false choice, so it is removed from the rail entirely.
    InMission { can_quit: bool },
}

impl SherwoodRail {
    /// The rail while a mission is being played.
    ///
    /// `H01_Lin_VL` and `H02_Not_EC` cannot return to Sherwood, so they do not
    /// offer `QuitMission`. Everywhere else the abandon control stays.
    pub fn in_mission(mission_name: Option<&str>) -> Self {
        Self::InMission {
            can_quit: !matches!(mission_name, Some("H01_Lin_VL" | "H02_Not_EC")),
        }
    }

    /// Derive the rail from campaign state. `has_next_mission` is
    /// `campaign.next_mission_idx.is_some()`; `start_ready` is the live
    /// requirements (or blazon-conversion) gate for the committed mission;
    /// `mission_name` is the running mission's filename, used only to decide
    /// whether an abandon route exists.
    pub fn from_campaign(
        is_sherwood: bool,
        has_next_mission: bool,
        start_ready: bool,
        mission_name: Option<&str>,
    ) -> Self {
        match (is_sherwood, has_next_mission) {
            (false, _) => Self::in_mission(mission_name),
            (true, false) => Self::PreCommit,
            (true, true) if start_ready => Self::PostCommit,
            (true, true) => Self::PostCommitStartNotReady,
        }
    }

    /// Whether the rail offers `button` in this situation.
    pub fn present(self, button: SherwoodButton) -> bool {
        use SherwoodButton::*;
        matches!(
            (self, button),
            (
                Self::InMission { can_quit: true },
                StartMission | QuitMission
            ) | (Self::InMission { can_quit: false }, StartMission)
                | (Self::PreCommit, DisplayCampaignMap | PerksPanel)
                | (
                    Self::PostCommit,
                    GoToExit | StartMission | QuitMission | PerksPanel
                )
                | (
                    Self::PostCommitStartNotReady,
                    GoToExit | QuitMission | PerksPanel
                )
                | (
                    Self::PreCommit | Self::PostCommit | Self::PostCommitStartNotReady,
                    SherwoodTrading
                )
        )
    }

    /// The offered buttons, in draw and hit-test order.
    pub fn visible(self) -> impl Iterator<Item = SherwoodButton> {
        SherwoodButton::ALL
            .into_iter()
            .filter(move |button| self.present(*button))
    }
}

/// Which Sherwood buttons are interactable this frame.
///
/// The enable toggles are derived from initialization (GoToExit
/// default-off), commit transitions (DisplayCampaignMap disabled after
/// commit), and per-frame mission-team refreshes (Start gated on
/// requirements).
#[derive(Debug, Clone, Copy)]
pub struct SherwoodButtonEnable {
    pub display_campaign_map: bool,
    pub go_to_exit: bool,
    pub start_mission: bool,
    pub quit_mission: bool,
    pub sherwood_trading: bool,
    pub perks_panel: bool,
}

impl SherwoodButtonEnable {
    /// Whether `button` can be clicked this frame.
    ///
    /// This is the click gate, not presence: an offered button whose gate is
    /// closed is drawn disabled (greyed) and still owns its pixels. The Sherwood
    /// StartMission requirements gate is applied earlier, to presence itself
    /// ([`SherwoodRail::from_campaign`]).
    pub fn is_enabled(self, button: SherwoodButton) -> bool {
        match button {
            SherwoodButton::DisplayCampaignMap => self.display_campaign_map,
            SherwoodButton::GoToExit => self.go_to_exit,
            SherwoodButton::StartMission => self.start_mission,
            SherwoodButton::QuitMission => self.quit_mission,
            SherwoodButton::SherwoodTrading => self.sherwood_trading,
            SherwoodButton::PerksPanel => self.perks_panel,
        }
    }

    /// The rail while a mission is being played: finish it once the win
    /// condition is met, and abandon it at any time. Abandoning is always
    /// allowed, so its gate is open from the first frame.
    pub fn in_mission(can_finish: bool) -> Self {
        Self {
            display_campaign_map: false,
            go_to_exit: false,
            start_mission: can_finish,
            quit_mission: true,
            sherwood_trading: false,
            perks_panel: false,
        }
    }

    /// Default state at Sherwood entry, before the player has
    /// committed to a next mission via the map overlay.  GoToExit is
    /// disabled, Start/Quit disabled until `DisplayCampaignMap`
    /// completes.
    pub fn pre_commit() -> Self {
        Self {
            display_campaign_map: true,
            go_to_exit: false,
            start_mission: false,
            quit_mission: false,
            sherwood_trading: false,
            perks_panel: true,
        }
    }

    /// State right after the player commits a mission via the overlay,
    /// before the live requirements gate has been applied to Start.
    /// DisplayCampaignMap is disabled, GoToExit and Quit are enabled.
    /// `start_mission` is left `false` here — the caller should
    /// immediately follow up with [`Self::apply_update_mission_team`]
    /// to reflect the live requirements / blazon-conversion gates.
    pub fn post_commit() -> Self {
        Self {
            display_campaign_map: false,
            go_to_exit: true,
            start_mission: false,
            quit_mission: true,
            sherwood_trading: false,
            perks_panel: true,
        }
    }

    /// Refresh the `start_mission` gate based on the live campaign
    /// state:
    ///
    /// - In men-to-blazon conversion mode, enable Start only when the
    ///   campaign has enough peasants to convert.
    /// - Otherwise, enable Start when the selected mission's
    ///   requirements are fulfilled by the current mission team
    ///   (`Campaign::mission_requirements_met`). The same check feeds
    ///   [`SherwoodRail::from_campaign`], so an unmet gate hides the button
    ///   rather than merely greying it.
    ///
    /// Once post-commit, `QuitMission` is always live so the player can
    /// back out.
    ///
    /// `start_disabled_temp` / `quit_disabled_temp` are the transient
    /// disable flags driven by the hourglass tick at
    /// `Game::perform_hourglass_inner`. They close the click gate for that
    /// frame: the button greys out and stops answering clicks, but stays on
    /// the rail. Presence is [`SherwoodRail`]'s job, not this mask's.
    pub fn apply_update_mission_team(
        &mut self,
        men_to_blazon: bool,
        can_convert_merry_men: bool,
        requirements_met: bool,
        start_disabled_temp: bool,
        quit_disabled_temp: bool,
    ) {
        let base_start = if men_to_blazon {
            can_convert_merry_men
        } else {
            requirements_met
        };
        self.start_mission = base_start && !start_disabled_temp;
        self.quit_mission = !quit_disabled_temp;
    }

    /// Refresh the `go_to_exit` gate from the live selection.  Runs every
    /// frame, equivalent to the predicate:
    ///
    /// ```text
    /// go_to_exit = selected_count != 0 && !selected_pc_in_mission_team
    /// ```
    ///
    /// The button dispatches start-up message 1000, which the Sherwood
    /// script answers with `SendToDeploymentZone` for every selected PC
    /// (`GetSelectedPC(i)`), so the gate must track the *selection* and
    /// nothing else.  An earlier version compared the whole portrait bar
    /// (`engine.pc_ids().len()`) against the next mission's
    /// `number_of_beam_mes`; in Sherwood the portrait bar holds every hero
    /// and recruit in the gang, so any gang larger than the target
    /// mission's beam slots left the button dark forever and the player had
    /// to walk the men to the exit by hand.  Capacity is a mission-entry
    /// concern — the beam-in limit and the men-to-blazon conversion are
    /// enforced there — not a reason to refuse to send the selection.
    ///
    /// `selected_pc_in_mission_team` comes from
    /// [`EngineInner::are_selected_pc_in_mission_team`]: when every selected
    /// PC is already committed to the upcoming mission the button stays
    /// dark, because there is nothing left to send.
    ///
    /// Callers with no committed next mission (`post_commit` hasn't run)
    /// keep `go_to_exit = false` from [`Self::pre_commit`]; this helper
    /// leaves the flag alone in that case so pre-commit state is preserved.
    pub fn apply_update_go_to_exit(
        &mut self,
        has_next_mission: bool,
        selected_count: usize,
        selected_pc_in_mission_team: bool,
    ) {
        if !has_next_mission {
            // No refresh when the player hasn't picked a target yet.
            return;
        }
        self.go_to_exit = selected_count != 0 && !selected_pc_in_mission_team;
    }
}

/// Horizontal offset of the tall column from the right screen edge — the lower
/// panel's slot column in the original layout.
const TALL_COLUMN_X_OFFSET: i32 = 45;
/// Panel coordinates of the column: first slot, and the pitch between slots.
const COLUMN_TOP: i32 = 105;
const COLUMN_PITCH: i32 = 45;
/// Clearance between a sprite's bottom edge and the next slot: the shipped
/// 41 px tall buttons plus this reproduce the panel's 45 px pitch exactly.
const RAIL_GAP: i32 = 4;

/// Vertical tops of the tall column: start mission, quit mission, trading,
/// perks.
///
/// The panel anchors hold whatever the artwork is, and a sprite taller than its
/// slot pushes the following ones down instead of overlapping them. Pure so
/// tests can cover the artwork cases: absent frames (fallback sizes), a wide
/// slot taller than the panel, and buttons taller than the pitch.
fn column_tops(wide_h: u16, tall: [u16; 3]) -> [i32; 4] {
    let mut tops = [COLUMN_TOP.max(wide_h as i32 + RAIL_GAP); 4];
    for (index, height) in tall.into_iter().enumerate() {
        tops[index + 1] = tops[index] + COLUMN_PITCH.max(height as i32 + RAIL_GAP);
    }
    tops
}

/// Screen-space bounding boxes for the Sherwood HUD buttons.
#[derive(Debug, Clone, Copy)]
pub struct SherwoodHudLayout {
    pub display_campaign_map: ScreenRect,
    pub go_to_exit: ScreenRect,
    pub start_mission: ScreenRect,
    pub quit_mission: ScreenRect,
    pub sherwood_trading: ScreenRect,
    pub perks_panel: ScreenRect,
}

impl SherwoodHudLayout {
    /// Derive button rects from the current screen resolution and the loaded
    /// DEFAULT.RES artwork. A button whose frames are absent falls back to a
    /// development size so the rail stays hit-testable; [`Self::for_sizes`]
    /// names that fallback so a test can prove the rail with the real art.
    pub fn for_resolution(screen_w: u32, screen_h: u32, sprites: &SherwoodButtonSprites) -> Self {
        Self::for_sizes(screen_w, screen_h, |button| sprites.size(button))
    }

    /// The layout for an explicit artwork table.
    ///
    /// Production passes the loaded DEFAULT.RES sizes; a caller with no
    /// renderer passes the sizes the artwork ships. Keeping the geometry a
    /// function of sizes alone is what lets a test prove the rail places no
    /// two visible controls over each other with the real artwork rather than
    /// with the development fallbacks.
    pub fn for_sizes(
        screen_w: u32,
        screen_h: u32,
        size: impl Fn(SherwoodButton) -> Option<(u16, u16)>,
    ) -> Self {
        // Fallback sizes for missing sprites (keeps the hit-rect
        // usable during development against incomplete DEFAULT.RES).
        const FALLBACK_WIDE_W: u16 = 80;
        const FALLBACK_WIDE_H: u16 = 32;
        const FALLBACK_TALL_W: u16 = 40;
        const FALLBACK_TALL_H: u16 = 40;

        let sw = screen_w as i32;

        // DisplayCampaignMap / GoToExit share the top-right corner of
        // the screen at `(width - 100, 0)` (no viewport-origin offset)
        // — they sit on the parchment-scroll decoration, not the lower
        // panel.  Sherwood missions show DisplayCampaignMap; non-Sherwood
        // would show Sight instead at the same coordinates.
        let wide_x = sw - 100;
        let wide_y = 0;
        let tall_x = sw - TALL_COLUMN_X_OFFSET;

        let (dcm_w, dcm_h) =
            size(SherwoodButton::DisplayCampaignMap).unwrap_or((FALLBACK_WIDE_W, FALLBACK_WIDE_H));
        let (gte_w, gte_h) =
            size(SherwoodButton::GoToExit).unwrap_or((FALLBACK_WIDE_W, FALLBACK_WIDE_H));
        let (sm_w, sm_h) =
            size(SherwoodButton::StartMission).unwrap_or((FALLBACK_TALL_W, FALLBACK_TALL_H));
        let (qm_w, qm_h) =
            size(SherwoodButton::QuitMission).unwrap_or((FALLBACK_TALL_W, FALLBACK_TALL_H));
        let (trade_w, trade_h) =
            size(SherwoodButton::SherwoodTrading).unwrap_or((FALLBACK_TALL_W, FALLBACK_TALL_H));
        let (perks_w, perks_h) =
            size(SherwoodButton::PerksPanel).unwrap_or((FALLBACK_TALL_W, FALLBACK_TALL_H));

        // The parchment scroll hosts the wide slot at the top-right; the tall
        // column lives on the lower panel, whose first slot is at 105 with a
        // 45 px pitch in the original layout. Those anchors are the artwork's
        // coordinates — deriving the column from the wide sprite put the
        // drafting checkmark at y=37 whenever that sprite's frames were absent.
        // The anchors still yield to the sprite: a wide slot taller than the
        // panel pushes the column down instead of overlapping it.
        let [start_y, quit_y, trading_y, perks_y] =
            column_tops(dcm_h.max(gte_h), [sm_h, qm_h, trade_h]);
        // The column is anchored to drawn artwork; artwork that does not fit
        // is a broken layout, not an off-position one.
        assert!(
            perks_y + perks_h as i32 <= screen_h as i32,
            "Sherwood rail does not fit {screen_w}x{screen_h}: column ends at {}",
            perks_y + perks_h as i32
        );

        Self {
            display_campaign_map: ScreenRect::new(wide_x, wide_y, dcm_w as u32, dcm_h as u32),
            go_to_exit: ScreenRect::new(wide_x, wide_y, gte_w as u32, gte_h as u32),
            start_mission: ScreenRect::new(tall_x, start_y, sm_w as u32, sm_h as u32),
            quit_mission: ScreenRect::new(tall_x, quit_y, qm_w as u32, qm_h as u32),
            sherwood_trading: ScreenRect::new(tall_x, trading_y, trade_w as u32, trade_h as u32),
            perks_panel: ScreenRect::new(tall_x, perks_y, perks_w as u32, perks_h as u32),
        }
    }

    /// Screen rect of `button`. The single source for both drawing and
    /// hit-testing.
    pub fn rect(&self, button: SherwoodButton) -> ScreenRect {
        match button {
            SherwoodButton::DisplayCampaignMap => self.display_campaign_map,
            SherwoodButton::GoToExit => self.go_to_exit,
            SherwoodButton::StartMission => self.start_mission,
            SherwoodButton::QuitMission => self.quit_mission,
            SherwoodButton::SherwoodTrading => self.sherwood_trading,
            SherwoodButton::PerksPanel => self.perks_panel,
        }
    }

    /// Hit-test a screen-space click: the first offered, enabled button whose
    /// rect contains the point. Callers consume the click whenever
    /// [`Self::claims_point`] is true, so a greyed control never acts on the
    /// world behind it and never answers a click either.
    pub fn hit_test(
        &self,
        x: i32,
        y: i32,
        rail: SherwoodRail,
        enable: SherwoodButtonEnable,
    ) -> Option<SherwoodButton> {
        let pt = Point::new(x, y);
        rail.visible()
            .find(|button| enable.is_enabled(*button) && self.rect(*button).contains_point(pt))
    }

    /// Hit-test a hover: the first offered button under the point, gate open or
    /// not. Disabled controls still explain themselves through their tooltip.
    pub fn hover_test(&self, x: i32, y: i32, rail: SherwoodRail) -> Option<SherwoodButton> {
        let pt = Point::new(x, y);
        rail.visible()
            .find(|button| self.rect(*button).contains_point(pt))
    }

    /// The rect of every offered button, whether or not its gate is open.
    /// Input capture uses this: a disabled control still owns its pixels.
    pub fn occupied(&self, rail: SherwoodRail) -> Vec<ScreenRect> {
        rail.visible().map(|button| self.rect(button)).collect()
    }

    /// Whether the rail owns the point at all — the click belongs to the HUD
    /// and must not reach the world, whatever the button's gate says.
    pub fn claims_point(&self, rail: SherwoodRail, x: i32, y: i32) -> bool {
        let pt = Point::new(x, y);
        self.occupied(rail)
            .into_iter()
            .any(|rect| rect.contains_point(pt))
    }
}

pub type SherwoodButtonSprites = crate::hud_sprite::ButtonSprites<SherwoodButton, 6>;
pub type SherwoodHoverState = crate::hud_sprite::HoverState<SherwoodButton>;

impl crate::hud_sprite::HudButton<6> for SherwoodButton {
    const ALL: [Self; 6] = [
        Self::DisplayCampaignMap,
        Self::GoToExit,
        Self::StartMission,
        Self::QuitMission,
        Self::SherwoodTrading,
        Self::PerksPanel,
    ];
    fn index(self) -> usize {
        self as usize
    }
    fn resource(self) -> (i32, &'static str) {
        match self {
            Self::DisplayCampaignMap => (RHID_DISPLAY_CAMPAIGN_MAP, "DisplayCampaignMap"),
            Self::GoToExit => (RHID_GO_TO_EXIT, "GoToExit"),
            Self::StartMission => (RHID_FLOATING_OK, "StartMission"),
            Self::QuitMission => (RHID_FLOATING_CANCEL, "QuitMission"),
            Self::SherwoodTrading => (RHID_CONVERT_MONEY_TO_BLAZONS, "SherwoodTrading"),
            Self::PerksPanel => (RHID_CLOVER, "PerksPanel"),
        }
    }
    fn frame_state(self, state: usize, frame_counter: u32) -> usize {
        if self == Self::DisplayCampaignMap
            && state == BTN_STATE_NORMAL
            && (frame_counter / 25) & 1 == 1
        {
            BTN_STATE_HOVER
        } else {
            state
        }
    }
    fn shadow(self) -> bool {
        matches!(
            self,
            Self::StartMission | Self::QuitMission | Self::SherwoodTrading | Self::PerksPanel
        )
    }
}

pub fn draw_with_sprites(
    renderer: &mut Renderer,
    layout: &SherwoodHudLayout,
    rail: SherwoodRail,
    enable: SherwoodButtonEnable,
    hover: SherwoodHoverState,
    sprites: &SherwoodButtonSprites,
    frame_counter: u32,
) {
    for button in rail.visible() {
        let enabled = enable.is_enabled(button);
        let is_hovered = enabled && hover.hovered == Some(button);
        let pressed = is_hovered && hover.mouse_pressed;
        let state = button_sprite_state(enabled, is_hovered, pressed);
        sprites.draw(renderer, button, layout.rect(button), state, frame_counter);
    }
}

pub type SherwoodTooltipTracker = crate::hud_sprite::ButtonTooltipTracker<SherwoodButton>;

/// Menu-text id for the Start/Quit mission button tooltip in the
/// current mode: Sherwood + men-to-blazon → Farmers-to-blazon /
/// BackToMap; Sherwood + regular → BeginMission / BackToMap;
/// in-mission → MissionFinish / MissionAbandon.
///
/// Returns `None` for the `DisplayCampaignMap` / `GoToExit` buttons —
/// their tooltips are static and don't switch with mode.  Callers
/// handle those via the static IDs in [`sherwood_static_tooltip_mt_id`].
pub fn start_quit_tooltip_mt_ids(is_sherwood: bool, men_to_blazon: bool) -> (usize, usize) {
    use crate::ingame_menu::resources::{
        MT_INFOBULLE_BUTTON_FARMERS_TO_BLAZON, MT_INFOBULLE_MISSION_ABANDON,
        MT_INFOBULLE_MISSION_FINISH, MT_INFOBULLE_QG_BACKTOMAP, MT_INFOBULLE_QG_BEGIN_MISSION,
    };
    if is_sherwood {
        if men_to_blazon {
            (
                MT_INFOBULLE_BUTTON_FARMERS_TO_BLAZON,
                MT_INFOBULLE_QG_BACKTOMAP,
            )
        } else {
            (MT_INFOBULLE_QG_BEGIN_MISSION, MT_INFOBULLE_QG_BACKTOMAP)
        }
    } else {
        (MT_INFOBULLE_MISSION_FINISH, MT_INFOBULLE_MISSION_ABANDON)
    }
}

/// Menu-text id for the tooltip attached to a Sherwood button, given
/// the current mode.  Uses [`start_quit_tooltip_mt_ids`] for Start/Quit
/// and the fixed static IDs for the Sherwood-only DisplayCampaignMap /
/// GoToExit buttons.
pub fn sherwood_button_tooltip_mt_id(
    btn: SherwoodButton,
    is_sherwood: bool,
    men_to_blazon: bool,
) -> Option<usize> {
    let (start_id, quit_id) = start_quit_tooltip_mt_ids(is_sherwood, men_to_blazon);
    match btn {
        SherwoodButton::StartMission => Some(start_id),
        SherwoodButton::QuitMission => Some(quit_id),
        SherwoodButton::SherwoodTrading => {
            Some(crate::ingame_menu::resources::MT_TTL_SHERWOOD_TRADING)
        }
        SherwoodButton::PerksPanel => {
            Some(crate::ingame_menu::resources::MT_INFOBULLE_BUTTON_PERKS)
        }
        // DisplayCampaignMap / GoToExit use their own fixed menu-text
        // IDs (`MT_INFOBULLE_QG_SELECT_MISSION` = 299 and
        // `MT_INFOBULLE_QG_DEPLOY` = 296), set once at initialization.
        // They aren't wired through the start/quit refresh path, so
        // they're outside this chunk's scope — when a future pass wires
        // those static tooltips in, plug them in here.
        SherwoodButton::DisplayCampaignMap | SherwoodButton::GoToExit => None,
    }
}

pub use crate::hud_sprite::{TooltipPlacement, draw_tooltip};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mission_offers_finish_and_abandon() {
        let layout = SherwoodHudLayout::for_resolution(800, 600, &SherwoodButtonSprites::default());
        let point = (layout.start_mission.x() + 1, layout.start_mission.y() + 1);
        let rail = SherwoodRail::in_mission(Some("H03_Der_MK"));
        assert_eq!(
            layout.hit_test(
                point.0,
                point.1,
                rail,
                SherwoodButtonEnable::in_mission(true)
            ),
            Some(SherwoodButton::StartMission)
        );
        assert_eq!(
            layout.hit_test(
                point.0,
                point.1,
                rail,
                SherwoodButtonEnable::in_mission(false)
            ),
            None
        );
        // Abandoning is always available; finishing is gated on the win.
        assert!(SherwoodButtonEnable::in_mission(false).quit_mission);
        assert!(SherwoodButtonEnable::in_mission(true).quit_mission);
        let enable = SherwoodButtonEnable::in_mission(true);
        assert!(
            !enable.go_to_exit
                && !enable.display_campaign_map
                && !enable.sherwood_trading
                && !enable.perks_panel
        );
    }

    #[test]
    fn first_missions_hide_the_abandon_button() {
        use SherwoodButton::*;

        let sprites = SherwoodButtonSprites::default();
        let layout = SherwoodHudLayout::for_resolution(800, 600, &sprites);
        let enable = SherwoodButtonEnable::in_mission(true);

        // The first two campaign missions have no route back to Sherwood, so
        // the red abandon control is a false choice and must not be offered.
        for name in ["H01_Lin_VL", "H02_Not_EC"] {
            let rail = SherwoodRail::from_campaign(false, false, true, Some(name));
            assert!(!rail.present(QuitMission), "{name} must hide QuitMission");
            assert!(rail.present(StartMission), "{name} still offers finish");
            assert_eq!(rail.visible().collect::<Vec<_>>(), vec![StartMission]);
            let rect = layout.rect(QuitMission);
            assert_eq!(
                layout.hit_test(rect.x() + 2, rect.y() + 2, rail, enable),
                None,
                "{name} must not claim the old button's pixels"
            );
        }

        // Every later mission — and the Sherwood hub — keep the button and
        // its enabled state.
        let later = SherwoodRail::from_campaign(false, false, true, Some("H03_Der_MK"));
        assert!(later.present(QuitMission));
        assert!(enable.is_enabled(QuitMission));
        let hub = SherwoodRail::from_campaign(true, true, true, Some("H03_Der_MK"));
        assert!(hub.present(QuitMission));
    }

    #[test]
    fn unmet_requirements_remove_the_start_mission_button() {
        let sprites = SherwoodButtonSprites::default();
        let layout = SherwoodHudLayout::for_resolution(800, 600, &sprites);
        // Committed mission, but its team requirements are not met.
        let rail = SherwoodRail::from_campaign(true, true, false, Some("H03_Der_MK"));
        let mut enable = SherwoodButtonEnable::post_commit();
        enable.apply_update_mission_team(false, false, false, false, false);

        // StartMission is not offered at all: no pixels, hover or click claim.
        assert!(!rail.present(SherwoodButton::StartMission));
        assert!(!rail.visible().any(|b| b == SherwoodButton::StartMission));
        let rect = layout.rect(SherwoodButton::StartMission);
        let point = (rect.x() + 2, rect.y() + 2);
        assert_eq!(layout.hit_test(point.0, point.1, rail, enable), None);
        assert_eq!(layout.hover_test(point.0, point.1, rail), None);
        assert!(!layout.claims_point(rail, point.0, point.1));

        // The rest of the post-commit rail survives the closed gate.
        assert!(rail.present(SherwoodButton::GoToExit));
        assert!(rail.present(SherwoodButton::QuitMission));
        assert!(rail.present(SherwoodButton::PerksPanel));
        assert!(rail.present(SherwoodButton::SherwoodTrading));
    }

    #[test]
    fn a_hidden_start_button_frees_its_pixels_but_other_controls_keep_theirs() {
        let layout = SherwoodHudLayout::for_resolution(800, 600, &SherwoodButtonSprites::default());
        let rail = SherwoodRail::from_campaign(true, true, false, Some("H03_Der_MK"));
        let enable = SherwoodButtonEnable::post_commit();
        let rect = layout.rect(SherwoodButton::StartMission);
        let point = (rect.x() + 2, rect.y() + 2);

        // The requirements gate is closed, so StartMission is absent: no
        // hover, no click and no pixel ownership to swallow a world click.
        assert_eq!(layout.hit_test(point.0, point.1, rail, enable), None);
        assert_eq!(layout.hover_test(point.0, point.1, rail), None);
        assert!(!layout.claims_point(rail, point.0, point.1));

        // The change is specific to the gated button: another offered control
        // still owns its own pixels.
        let quit = layout.rect(SherwoodButton::QuitMission);
        let quit_point = (quit.x() + 2, quit.y() + 2);
        assert!(layout.claims_point(rail, quit_point.0, quit_point.1));
        assert!(!layout.claims_point(rail, 0, 0));
    }

    #[test]
    #[should_panic(expected = "Sherwood rail does not fit")]
    fn a_screen_too_short_for_the_rail_fails_loudly() {
        let _ = SherwoodHudLayout::for_resolution(800, 200, &SherwoodButtonSprites::default());
    }

    #[test]
    fn presence_follows_campaign_state() {
        use SherwoodButton::*;

        let pre = SherwoodRail::from_campaign(true, false, true, Some("H03_Der_MK"));
        assert_eq!(
            pre.visible().collect::<Vec<_>>(),
            vec![DisplayCampaignMap, SherwoodTrading, PerksPanel]
        );

        let post = SherwoodRail::from_campaign(true, true, true, Some("H03_Der_MK"));
        assert_eq!(
            post.visible().collect::<Vec<_>>(),
            vec![
                GoToExit,
                StartMission,
                QuitMission,
                SherwoodTrading,
                PerksPanel
            ]
        );

        let mission = SherwoodRail::from_campaign(false, false, true, Some("H03_Der_MK"));
        assert_eq!(
            mission.visible().collect::<Vec<_>>(),
            vec![StartMission, QuitMission]
        );
        // A committed campaign while a mission plays is still a mission.
        assert_eq!(
            SherwoodRail::from_campaign(false, true, true, Some("H03_Der_MK")),
            mission
        );

        // The two controls that share the wide slot are never both present:
        // the draft column must not hide the map button, and the map button
        // must not survive the commit.
        for rail in [pre, post, mission] {
            assert!(
                !(rail.present(DisplayCampaignMap) && rail.present(GoToExit)),
                "the wide slot offers exactly one control per state"
            );
            assert!(
                !(rail.present(DisplayCampaignMap) && rail.present(StartMission)),
                "campaign-map and drafting controls must never share a state"
            );
        }
    }

    #[test]
    fn unmet_requirements_remove_the_start_button_from_the_rail() {
        let sprites = SherwoodButtonSprites::default();
        let layout = SherwoodHudLayout::for_resolution(800, 600, &sprites);
        let rail = SherwoodRail::from_campaign(true, true, false, Some("H03_Der_MK"));
        let mut enable = SherwoodButtonEnable::post_commit();
        enable.apply_update_mission_team(false, false, false, false, false);

        // Not offered: neither the rail nor the layout's occupied rects know
        // about it, so there is no disabled StartMission to draw or tooltip.
        assert!(!rail.present(SherwoodButton::StartMission));
        assert!(!rail.visible().any(|b| b == SherwoodButton::StartMission));
        let rect = layout.rect(SherwoodButton::StartMission);
        let centre = (rect.x() + 2, rect.y() + 2);
        assert_eq!(layout.hit_test(centre.0, centre.1, rail, enable), None);
        assert!(
            !layout
                .occupied(rail)
                .into_iter()
                .any(|r| r.contains_point(Point::new(centre.0, centre.1)))
        );

        // The rest of the post-commit rail is untouched by the closed gate.
        assert!(rail.present(SherwoodButton::GoToExit));
        assert!(rail.present(SherwoodButton::QuitMission));
        assert!(rail.present(SherwoodButton::PerksPanel));
    }

    fn intersects(a: &ScreenRect, b: &ScreenRect) -> bool {
        a.x() < b.x() + b.width() as i32
            && b.x() < a.x() + a.width() as i32
            && a.y() < b.y() + b.height() as i32
            && b.y() < a.y() + a.height() as i32
    }

    #[test]
    fn visible_rects_never_intersect() {
        let sprites = SherwoodButtonSprites::default();
        let every_rail = [
            SherwoodRail::from_campaign(true, false, true, Some("H03_Der_MK")),
            SherwoodRail::from_campaign(true, true, true, Some("H03_Der_MK")),
            SherwoodRail::from_campaign(false, false, true, Some("H03_Der_MK")),
        ];

        for (width, height) in [(800, 600), (2259, 1271)] {
            let layout = SherwoodHudLayout::for_resolution(width, height, &sprites);
            for rail in every_rail {
                let rects: Vec<_> = rail
                    .visible()
                    .map(|button| (button, layout.rect(button)))
                    .collect();
                for (index, (left_button, left)) in rects.iter().enumerate() {
                    for (right_button, right) in &rects[index + 1..] {
                        assert!(
                            !intersects(left, right),
                            "{left_button:?} at {left:?} intersects {right_button:?} at {right:?} \
                             on {width}x{height}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn column_tops_hold_the_panel_anchors_when_artwork_is_absent() {
        // Fallback sizes: the state that put the checkmark at y=37.
        assert_eq!(column_tops(32, [40, 40, 40]), [105, 150, 195, 240]);
        // An absent bank yields `size() == None` and the fallback above; a
        // zero-sized frame cannot reach here, so the arithmetic is only
        // required to stay monotone (`column_tops(0, …) == anchors`).
        assert_eq!(column_tops(0, [0, 0, 0]), [105, 150, 195, 240]);
        // A wide slot that exactly meets the panel still leaves the anchor.
        assert_eq!(column_tops(100, [40, 40, 40]), [105, 150, 195, 240]);
        // The artwork this data set ships: DisplayCampaignMap 74x53,
        // GoToExit 74x60, the tall buttons 41x41.
        assert_eq!(column_tops(60, [41, 41, 41]), [105, 150, 195, 240]);
        assert_eq!(column_tops(53, [41, 41, 41]), [105, 150, 195, 240]);
    }

    #[test]
    fn column_tops_yield_to_a_wide_slot_taller_than_the_panel() {
        assert_eq!(column_tops(102, [40, 40, 40]), [106, 151, 196, 241]);
        assert_eq!(column_tops(120, [40, 40, 40]), [124, 169, 214, 259]);
    }

    #[test]
    fn column_tops_grow_the_pitch_for_tall_buttons() {
        assert_eq!(column_tops(32, [60, 60, 60]), [105, 169, 233, 297]);
        assert_eq!(column_tops(32, [40, 60, 60]), [105, 150, 214, 278]);
    }

    #[test]
    fn every_offered_button_answers_for_its_own_centre_when_enabled() {
        let sprites = SherwoodButtonSprites::default();
        let layout = SherwoodHudLayout::for_resolution(2259, 1271, &sprites);
        let all_gates_open = SherwoodButtonEnable {
            display_campaign_map: true,
            go_to_exit: true,
            start_mission: true,
            quit_mission: true,
            sherwood_trading: true,
            perks_panel: true,
        };

        for rail in [
            SherwoodRail::from_campaign(true, false, true, Some("H03_Der_MK")),
            SherwoodRail::from_campaign(true, true, true, Some("H03_Der_MK")),
            SherwoodRail::from_campaign(false, false, true, Some("H03_Der_MK")),
        ] {
            for button in rail.visible() {
                let rect = layout.rect(button);
                let centre = (
                    rect.x() + rect.width() as i32 / 2,
                    rect.y() + rect.height() as i32 / 2,
                );
                assert_eq!(
                    layout.hit_test(centre.0, centre.1, rail, all_gates_open),
                    Some(button),
                    "{button:?} must claim its own centre on {rail:?}"
                );
            }
        }
    }

    #[test]
    fn absent_buttons_do_not_claim_clicks() {
        let sprites = SherwoodButtonSprites::default();
        let layout = SherwoodHudLayout::for_resolution(2259, 1271, &sprites);
        let pre = SherwoodRail::from_campaign(true, false, true, Some("H03_Der_MK"));
        let all_gates_open = SherwoodButtonEnable {
            display_campaign_map: true,
            go_to_exit: true,
            start_mission: true,
            quit_mission: true,
            sherwood_trading: true,
            perks_panel: true,
        };
        for button in SherwoodButton::ALL {
            if pre.present(button) {
                continue;
            }
            let rect = layout.rect(button);
            let point = (rect.x() + 1, rect.y() + 1);
            assert_ne!(
                layout.hit_test(point.0, point.1, pre, all_gates_open),
                Some(button),
                "{button:?} is not offered before a mission is committed"
            );
            assert!(!pre.present(button));
        }
    }

    #[test]
    fn layout_fits_screen() {
        let sprites = SherwoodButtonSprites::default();
        let layout = SherwoodHudLayout::for_resolution(800, 600, &sprites);
        // Display / GoToExit sit at the top-right (on the parchment
        // scroll decoration), NOT the lower panel — screen-absolute
        // positioning.
        assert_eq!(layout.display_campaign_map.y(), 0);
        assert!(
            layout.display_campaign_map.x() + layout.display_campaign_map.width() as i32 <= 800
        );
        // The tall column holds the panel's anchors: 105, then a 45 px pitch.
        // Missing sprite frames must not move it (the drafting checkmark at
        // y=37 was exactly that defect).
        assert_eq!(layout.start_mission.y(), 105);
        assert_eq!(layout.quit_mission.y(), 150);
        assert_eq!(layout.sherwood_trading.y(), 195);
        assert_eq!(layout.perks_panel.y(), 240);
        assert_eq!(layout.start_mission.x(), 800 - 45);
    }

    #[test]
    fn the_wide_slot_resolves_to_the_offered_button() {
        let sprites = SherwoodButtonSprites::default();
        let layout = SherwoodHudLayout::for_resolution(800, 600, &sprites);
        let pt_map = (
            layout.display_campaign_map.x() + 1,
            layout.display_campaign_map.y() + 1,
        );
        // DisplayCampaignMap and GoToExit overlap; enabling only
        // GoToExit should return GoToExit.
        assert_eq!(
            layout.hit_test(
                pt_map.0,
                pt_map.1,
                SherwoodRail::from_campaign(true, true, true, Some("H03_Der_MK")),
                SherwoodButtonEnable::post_commit()
            ),
            Some(SherwoodButton::GoToExit)
        );
        assert_eq!(
            layout.hit_test(
                pt_map.0,
                pt_map.1,
                SherwoodRail::from_campaign(true, false, true, Some("H03_Der_MK")),
                SherwoodButtonEnable::pre_commit()
            ),
            Some(SherwoodButton::DisplayCampaignMap)
        );

        let mut trading = SherwoodButtonEnable::pre_commit();
        trading.sherwood_trading = true;
        let pt_trade = (
            layout.sherwood_trading.x() + 1,
            layout.sherwood_trading.y() + 1,
        );
        assert_eq!(
            layout.hit_test(
                pt_trade.0,
                pt_trade.1,
                SherwoodRail::from_campaign(true, false, true, Some("H03_Der_MK")),
                trading
            ),
            Some(SherwoodButton::SherwoodTrading)
        );
    }

    #[test]
    fn perks_panel_is_a_sherwood_only_live_button() {
        let sprites = SherwoodButtonSprites::default();
        let layout = SherwoodHudLayout::for_resolution(800, 600, &sprites);
        let point = (layout.perks_panel.x() + 1, layout.perks_panel.y() + 1);

        assert!(SherwoodButtonEnable::pre_commit().perks_panel);
        assert!(SherwoodButtonEnable::post_commit().perks_panel);
        assert_eq!(
            layout.hit_test(
                point.0,
                point.1,
                SherwoodRail::from_campaign(true, false, true, Some("H03_Der_MK")),
                SherwoodButtonEnable::pre_commit()
            ),
            Some(SherwoodButton::PerksPanel)
        );

        // The non-Sherwood (won mission) enable mask never exposes the perks
        // panel, so it cannot be opened outside Sherwood.
        assert!(!SherwoodButtonEnable::in_mission(true).perks_panel);
        assert_eq!(
            layout.hit_test(
                point.0,
                point.1,
                SherwoodRail::from_campaign(false, false, true, Some("H03_Der_MK")),
                SherwoodButtonEnable::in_mission(true)
            ),
            None
        );
    }

    #[test]
    fn hit_test_misses_outside_rects() {
        let sprites = SherwoodButtonSprites::default();
        let layout = SherwoodHudLayout::for_resolution(800, 600, &sprites);
        assert!(
            layout
                .hit_test(
                    0,
                    0,
                    SherwoodRail::from_campaign(true, true, true, Some("H03_Der_MK")),
                    SherwoodButtonEnable::post_commit()
                )
                .is_none()
        );
    }

    #[test]
    fn go_to_exit_tracks_the_selection_not_the_portrait_bar() {
        // A full gang (more portraits than the target mission's beam slots)
        // must not darken the button: the script sends only what is selected.
        let mut enable = SherwoodButtonEnable::post_commit();
        enable.apply_update_go_to_exit(true, 1, false);
        assert!(
            enable.go_to_exit,
            "one selected recruit is enough to send to the deployment zone"
        );

        // Nothing selected: nothing to send.
        enable.apply_update_go_to_exit(true, 0, true);
        assert!(!enable.go_to_exit);

        // Every selected PC already committed to the next mission.
        enable.apply_update_go_to_exit(true, 3, true);
        assert!(!enable.go_to_exit);

        // No next mission committed yet: pre-commit state is preserved.
        let mut pre = SherwoodButtonEnable::pre_commit();
        pre.apply_update_go_to_exit(false, 2, false);
        assert!(!pre.go_to_exit);
    }

    /// The sizes DEFAULT.RES ships: the two wide slots and the tall column.
    fn shipped_button_sizes(button: SherwoodButton) -> Option<(u16, u16)> {
        use SherwoodButton::*;
        match button {
            DisplayCampaignMap => Some((74, 53)),
            GoToExit => Some((74, 60)),
            StartMission | QuitMission | SherwoodTrading | PerksPanel => Some((41, 41)),
        }
    }

    #[test]
    fn artwork_sizes_define_the_rail_not_the_fallbacks() {
        let layout = SherwoodHudLayout::for_sizes(800, 600, shipped_button_sizes);
        for (button, (width, height)) in [
            (SherwoodButton::DisplayCampaignMap, (74u16, 53u16)),
            (SherwoodButton::GoToExit, (74, 60)),
            (SherwoodButton::StartMission, (41, 41)),
            (SherwoodButton::QuitMission, (41, 41)),
            (SherwoodButton::SherwoodTrading, (41, 41)),
            (SherwoodButton::PerksPanel, (41, 41)),
        ] {
            let rect = layout.rect(button);
            assert_eq!(
                (rect.width(), rect.height()),
                (u32::from(width), u32::from(height)),
                "{button:?} must take its size from the artwork, not a fallback"
            );
        }
    }

    #[test]
    fn visible_rects_never_intersect_with_the_shipped_artwork() {
        let every_rail = [
            SherwoodRail::from_campaign(true, false, true, Some("H03_Der_MK")),
            SherwoodRail::from_campaign(true, true, true, Some("H03_Der_MK")),
            SherwoodRail::from_campaign(false, false, true, Some("H03_Der_MK")),
        ];

        for (width, height) in [(800, 600), (1024, 768), (2259, 1271)] {
            let layout = SherwoodHudLayout::for_sizes(width, height, shipped_button_sizes);
            for rail in every_rail {
                let rects: Vec<_> = rail
                    .visible()
                    .map(|button| (button, layout.rect(button)))
                    .collect();
                for (index, (left_button, left)) in rects.iter().enumerate() {
                    for (right_button, right) in &rects[index + 1..] {
                        assert!(
                            !intersects(left, right),
                            "{left_button:?} at {left:?} intersects {right_button:?} at {right:?} \
                             on {width}x{height}"
                        );
                    }
                }
            }
        }
    }
}
