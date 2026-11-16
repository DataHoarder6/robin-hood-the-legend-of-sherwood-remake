//! Sherwood perks purchase panel.
//!
//! One selectable row per `(campaign character, perk)` built by
//! [`robin_engine::campaign::Campaign::perk_purchase_rows`].  The panel owns
//! only selection and rendering; the authoritative purchase is
//! [`PlayerCommand::CampaignPurchasePerk`] emitted by [`PerksPanelOutcome`].
//!
//! Input matches the trading panel: Up/Down (or Tab) move the row selection,
//! Return/Space buys the selected affordable row, a pointer click buys the row
//! under the cursor, and Escape or the Cancel button closes the panel.

use crate::gfx_types::{GameEvent, Keycode};
use robin_engine::campaign::{Campaign, CampaignValue, PerkPurchaseRow};
use robin_engine::pc_status::Perk;
use robin_engine::player_command::PlayerCommand;
use robin_engine::profiles::ProfileManager;
use robin_engine::sherwood_stat::MenuTextLookup;

use super::layout::{MENU_W, MenuTransform, draw_screen_background, render_text_virt_font};
use super::resources::{
    IngameMenuResources, MT_BTN_CANCEL, MT_STR_PERK_AFFORDABLE, MT_STR_PERK_AUTO_LOOT,
    MT_STR_PERK_HINT, MT_STR_PERK_KEEP_MOVING_IN_COMBAT, MT_STR_PERK_NO_FRIENDLY_FIRE_DAMAGE,
    MT_STR_PERK_NO_FRIENDLY_FIRE_KNOCKBACK, MT_STR_PERK_OWNED, MT_STR_PERK_ROW,
    MT_STR_PERK_UNAFFORDABLE, MT_STR_TRADE_RANSOM, MT_TTL_SHERWOOD_PERKS,
};
use super::trading::substitute;
use super::widget_bridge::{self, ModalInputState, ModalScreenIo, ScreenFrame, ScreenKey};

const ID_CLOSE: u32 = 500;
const ROW_X: i32 = 72;
const ROW_Y: i32 = 82;
const ROW_W: i32 = 496;
const ROW_H: i32 = 28;
/// Rows that fit between the ransom line and the hint/button area.
const VISIBLE_ROWS: usize = 10;
const HINT_Y: i32 = 374;
const BUTTON_Y: i32 = 414;

/// Result of one panel tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerksPanelOutcome {
    /// Escape, Quit, or the Cancel button.
    Close,
    /// A row was activated and may be bought.  The authoritative handler still
    /// owns price, affordability and already-owned checks.
    Purchase { character_idx: u16, perk: Perk },
}

impl PerksPanelOutcome {
    /// The command this outcome asks the host to dispatch.
    ///
    /// `Close` produces no command; a purchase produces exactly one.
    pub fn player_command(self) -> Option<PlayerCommand> {
        match self {
            Self::Close => None,
            Self::Purchase {
                character_idx,
                perk,
            } => Some(PlayerCommand::CampaignPurchasePerk {
                character_idx,
                perk,
            }),
        }
    }
}

/// Display labels for `campaign.characters`, in the same order.
///
/// `PcStatus::name` is only populated for spawned PCs, so a campaign roster
/// entry that never entered a mission would otherwise render blank.  Every
/// campaign character is created from a profile, so its display label lives
/// there: the localized hero name, or the mod profile's own label.
pub fn character_names(
    campaign: &Campaign,
    profiles: &ProfileManager,
    portraits: &crate::ui_panel::PortraitCache,
    menu_text: &dyn MenuTextLookup,
) -> Vec<String> {
    campaign
        .characters
        .iter()
        .map(|character| {
            let status_name = character.status.display_name(menu_text);
            if !status_name.is_empty() {
                return status_name.into_owned();
            }
            let profile_index = character
                .character_profile_idx
                .expect("campaign character is not linked to a profile");
            crate::hud_text::profile_character_display_name(profiles, portraits, profile_index)
                .expect("campaign character references a missing profile")
                .into_owned()
        })
        .collect()
}

/// The rows and live ransom the perks panel renders.
pub fn campaign_rows(
    campaign: &Campaign,
    profiles: &ProfileManager,
    portraits: &crate::ui_panel::PortraitCache,
    menu_text: &dyn MenuTextLookup,
) -> (Vec<PerkPurchaseRow>, i32) {
    let names = character_names(campaign, profiles, portraits, menu_text);
    (
        campaign.perk_purchase_rows(&names),
        campaign.get_value(CampaignValue::Ransom),
    )
}

pub struct PerksPanelModalState {
    rows: Vec<PerkPurchaseRow>,
    ransom: i32,
    selected: usize,
    scroll_top: usize,
    input: ModalInputState,
    frame: crate::widget::FrameWnd,
    close_x: i32,
    close_y: i32,
    close_w: i32,
    close_h: i32,
}

impl PerksPanelModalState {
    pub fn new(
        window: &crate::window::GameWindow,
        renderer: &crate::renderer::Renderer,
        resources: &IngameMenuResources,
        rows: Vec<PerkPurchaseRow>,
        ransom: i32,
    ) -> Self {
        Self::build(
            resources,
            rows,
            ransom,
            ModalInputState::for_screen(window, renderer),
        )
    }

    fn build(
        resources: &IngameMenuResources,
        rows: Vec<PerkPurchaseRow>,
        ransom: i32,
        input: ModalInputState,
    ) -> Self {
        let (button_w, button_h) = resources.button_dimensions();
        let close_x = (MENU_W - button_w) / 2;
        let mut frame = crate::widget::FrameWnd::interactive();
        frame.add_widget_absolute(widget_bridge::make_button(
            ID_CLOSE,
            &resources.menu_text.get(MT_BTN_CANCEL),
            close_x,
            BUTTON_Y,
            button_w,
            button_h,
        ));
        let mut state = Self {
            rows,
            ransom,
            selected: 0,
            scroll_top: 0,
            input,
            frame,
            close_x,
            close_y: BUTTON_Y,
            close_w: button_w,
            close_h: button_h,
        };
        state.clamp_selection();
        state
    }

    /// Replace the rows and ransom from the current campaign state.
    ///
    /// Called each tick by the driver so a purchase lands on the next frame
    /// without the panel caching a stale affordability or owned bit.
    pub fn set_rows(&mut self, rows: Vec<PerkPurchaseRow>, ransom: i32) {
        self.rows = rows;
        self.ransom = ransom;
        self.clamp_selection();
    }

    pub fn tick(&mut self, io: &mut ModalScreenIo<'_, '_>) -> Option<PerksPanelOutcome> {
        let screen = ScreenFrame::poll(io);
        let outcome = self.handle_events(&screen.events, screen.transform);
        self.render(io, &screen);
        outcome
    }

    fn handle_events(
        &mut self,
        events: &[GameEvent],
        transform: MenuTransform,
    ) -> Option<PerksPanelOutcome> {
        let mut outcome = None;
        for event in events {
            self.input.update_from_event(event, transform);
            let keyboard_activation = match ScreenKey::from_event(event) {
                Some(ScreenKey::Quit | ScreenKey::Cancel) => Some(PerksPanelOutcome::Close),
                Some(ScreenKey::Next) => {
                    self.move_selection(1);
                    None
                }
                Some(ScreenKey::Confirm) => self.activate_selected(),
                None => match event {
                    GameEvent::KeyDown {
                        keycode: Keycode::Up,
                        ..
                    } => {
                        self.move_selection(-1);
                        None
                    }
                    GameEvent::KeyDown {
                        keycode: Keycode::Down,
                        ..
                    } => {
                        self.move_selection(1);
                        None
                    }
                    GameEvent::KeyDown {
                        keycode: Keycode::Char(b' '),
                        ..
                    } => self.activate_selected(),
                    _ => None,
                },
            };

            let widget_input = self.input.as_widget_input();
            self.frame.process_input(&widget_input);
            self.input.end_frame();
            if outcome.is_none()
                && let GameEvent::MouseUp(x, y, 1) = event
            {
                let (vx, vy) = transform.from_screen(*x, *y);
                if self.close_rect_contains(vx, vy) {
                    outcome = Some(PerksPanelOutcome::Close);
                } else if let Some(row) = self.row_at(vx, vy) {
                    self.selected = row;
                    self.clamp_selection();
                    outcome = self.activate_selected();
                }
            }
            if outcome.is_none() {
                outcome = keyboard_activation;
            }
        }
        outcome
    }

    fn activate_selected(&self) -> Option<PerksPanelOutcome> {
        let row = self.rows.get(self.selected)?;
        if row.owned || !row.affordable {
            return None;
        }
        Some(PerksPanelOutcome::Purchase {
            character_idx: row.character_idx,
            perk: row.perk,
        })
    }

    fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let len = self.rows.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        if self.rows.is_empty() {
            self.selected = 0;
            self.scroll_top = 0;
            return;
        }
        self.selected = self.selected.min(self.rows.len() - 1);
        if self.selected < self.scroll_top {
            self.scroll_top = self.selected;
        }
        if self.selected >= self.scroll_top + VISIBLE_ROWS {
            self.scroll_top = self.selected + 1 - VISIBLE_ROWS;
        }
        self.scroll_top = self
            .scroll_top
            .min(self.rows.len().saturating_sub(VISIBLE_ROWS));
    }

    fn visible_range(&self) -> std::ops::Range<usize> {
        let end = (self.scroll_top + VISIBLE_ROWS).min(self.rows.len());
        self.scroll_top..end
    }

    fn close_rect_contains(&self, vx: i32, vy: i32) -> bool {
        (self.close_x..self.close_x + self.close_w).contains(&vx)
            && (self.close_y..self.close_y + self.close_h).contains(&vy)
    }

    fn row_at(&self, vx: i32, vy: i32) -> Option<usize> {
        if !(ROW_X..ROW_X + ROW_W).contains(&vx)
            || !(ROW_Y..ROW_Y + VISIBLE_ROWS as i32 * ROW_H).contains(&vy)
        {
            return None;
        }
        let visible = ((vy - ROW_Y) / ROW_H) as usize;
        let index = self.scroll_top + visible;
        (index < self.rows.len()).then_some(index)
    }

    fn render(&self, io: &mut ModalScreenIo<'_, '_>, screen: &ScreenFrame) {
        let renderer = &mut *io.renderer;
        let resources = io.resources;
        let transform = screen.transform;
        screen.begin_draw(renderer);
        if let Some(background) = resources.menu_bg[0] {
            draw_screen_background(renderer, &background);
        }
        if let Some(font) = resources.title_font_any() {
            let title = resources.menu_text.get(MT_TTL_SHERWOOD_PERKS);
            render_text_virt_font(
                renderer,
                font,
                transform,
                &title,
                (MENU_W - font.text_width(&title)) / 2,
                24,
            );
        }
        if let Some(font) = resources.label_font_any() {
            let ransom = substitute(
                &resources.menu_text.get(MT_STR_TRADE_RANSOM),
                &[&self.ransom.to_string()],
            );
            render_text_virt_font(renderer, font, transform, &ransom, ROW_X, 57);
            for index in self.visible_range() {
                let row = &self.rows[index];
                let y = ROW_Y + (index - self.scroll_top) as i32 * ROW_H;
                if index == self.selected {
                    let (sx, sy) = transform.to_screen(ROW_X - 4, y - 3);
                    renderer.fill_screen(
                        Some(&robin_engine::sprite::BBox::from_coords(
                            sx as f32,
                            sy as f32,
                            (sx + ROW_W) as f32,
                            (sy + ROW_H - 2) as f32,
                        )),
                        crate::renderer::Renderer::create_color_16(55, 74, 45),
                    );
                }
                let line = substitute(
                    &resources.menu_text.get(MT_STR_PERK_ROW),
                    &[
                        &row.character_name,
                        &resources.menu_text.get(perk_label_key(row.perk)),
                        &row.price.to_string(),
                        &resources.menu_text.get(perk_state_key(row)),
                    ],
                );
                render_text_virt_font(renderer, font, transform, &line, ROW_X, y);
            }
            render_text_virt_font(
                renderer,
                font,
                transform,
                &resources.menu_text.get(MT_STR_PERK_HINT),
                ROW_X,
                HINT_Y,
            );
        }
        if let Some(widget) = self.frame.widget(ID_CLOSE) {
            widget_bridge::draw_widget_button(renderer, resources, transform, widget, false);
        }
        screen.finish(io, &self.input);
    }
}

fn perk_label_key(perk: Perk) -> usize {
    match perk {
        Perk::KeepMovingInCombat => MT_STR_PERK_KEEP_MOVING_IN_COMBAT,
        Perk::AutoLoot => MT_STR_PERK_AUTO_LOOT,
        Perk::NoFriendlyFireDamage => MT_STR_PERK_NO_FRIENDLY_FIRE_DAMAGE,
        Perk::NoFriendlyFireKnockback => MT_STR_PERK_NO_FRIENDLY_FIRE_KNOCKBACK,
    }
}

fn perk_state_key(row: &PerkPurchaseRow) -> usize {
    if row.owned {
        MT_STR_PERK_OWNED
    } else if row.affordable {
        MT_STR_PERK_AFFORDABLE
    } else {
        MT_STR_PERK_UNAFFORDABLE
    }
}

#[cfg(test)]
mod tests {
    use super::{PerksPanelModalState, PerksPanelOutcome, ROW_H, ROW_X, ROW_Y};
    use crate::gfx_types::GameEvent;
    use crate::ingame_menu::IngameMenuResources;
    use crate::ingame_menu::layout::MenuTransform;
    use crate::ingame_menu::widget_bridge::ModalInputState;
    use robin_engine::campaign::{PcDescription, perk_purchase_rows};
    use robin_engine::pc_status::{PcStatus, Perk};

    fn character(name: &str, owned: &[Perk]) -> PcDescription {
        let mut description = PcDescription {
            status: PcStatus {
                name: name.to_string(),
                ..PcStatus::default()
            },
            ..PcDescription::default()
        };
        for perk in owned {
            description.status.grant_perk(*perk);
        }
        description
    }

    fn panel(ransom: i32) -> PerksPanelModalState {
        let resources = IngameMenuResources::stub();
        let rows = perk_purchase_rows(
            &[
                character("Robin", &[Perk::KeepMovingInCombat]),
                character("Little John", &[]),
            ],
            &["Robin".to_owned(), "Little John".to_owned()],
            ransom,
        );
        PerksPanelModalState::build(&resources, rows, ransom, ModalInputState::new())
    }

    fn key(keycode: crate::gfx_types::Keycode) -> Vec<GameEvent> {
        vec![GameEvent::KeyDown {
            keycode,
            physical_key: None,
        }]
    }

    fn tap(x: i32, y: i32) -> Vec<GameEvent> {
        vec![
            GameEvent::MouseMove {
                x,
                y,
                xrel: 0,
                yrel: 0,
            },
            GameEvent::MouseDown(x, y, 1, 1),
            GameEvent::MouseUp(x, y, 1),
        ]
    }

    #[test]
    fn return_buys_the_selected_characters_row_exactly_once() {
        let mut state = panel(1_000);
        // Row 5 is Little John's AutoLoot row (4 perks per character).
        state.move_selection(5);
        let outcome = state.handle_events(
            &key(crate::gfx_types::Keycode::Return),
            MenuTransform::centered(640, 480),
        );
        assert_eq!(
            outcome,
            Some(PerksPanelOutcome::Purchase {
                character_idx: 1,
                perk: Perk::AutoLoot,
            })
        );
        let commands: Vec<_> = outcome
            .into_iter()
            .filter_map(PerksPanelOutcome::player_command)
            .collect();
        assert_eq!(
            commands.len(),
            1,
            "activating one row must emit exactly one command"
        );
        let robin_engine::player_command::PlayerCommand::CampaignPurchasePerk {
            character_idx,
            perk,
        } = &commands[0]
        else {
            panic!("expected a perk purchase, got {:?}", commands[0]);
        };
        assert_eq!((*character_idx, *perk), (1, Perk::AutoLoot));
    }

    #[test]
    fn a_pointer_click_buys_the_row_under_the_cursor() {
        let mut state = panel(1_000);
        let row = 5;
        let y = ROW_Y + row as i32 * ROW_H + ROW_H / 2;
        let x = ROW_X + 20;
        let outcome = state.handle_events(&tap(x, y), MenuTransform::centered(640, 480));
        assert_eq!(
            outcome,
            Some(PerksPanelOutcome::Purchase {
                character_idx: 1,
                perk: Perk::AutoLoot,
            })
        );
    }

    #[test]
    fn owned_and_unaffordable_rows_do_not_emit_a_purchase() {
        let resources = IngameMenuResources::stub();
        let ownable = perk_purchase_rows(
            &[character("Robin", &[Perk::KeepMovingInCombat])],
            &["Robin".to_owned()],
            1_000,
        );
        let mut owned =
            PerksPanelModalState::build(&resources, ownable, 1_000, ModalInputState::new());
        assert_eq!(
            owned.handle_events(
                &key(crate::gfx_types::Keycode::Return),
                MenuTransform::centered(640, 480),
            ),
            None,
            "an owned perk is not purchasable"
        );

        let unaffordable = perk_purchase_rows(&[character("Robin", &[])], &["Robin".to_owned()], 0);
        let mut poor =
            PerksPanelModalState::build(&resources, unaffordable, 0, ModalInputState::new());
        assert_eq!(
            poor.handle_events(
                &key(crate::gfx_types::Keycode::Return),
                MenuTransform::centered(640, 480),
            ),
            None,
            "an unaffordable perk is not purchasable"
        );
    }

    #[test]
    fn escape_closes_without_a_command() {
        let resources = IngameMenuResources::stub();
        let mut state = panel(1_000);
        let outcome = state.handle_events(
            &key(crate::gfx_types::Keycode::Escape),
            MenuTransform::centered(640, 480),
        );
        assert_eq!(outcome, Some(PerksPanelOutcome::Close));
        assert!(
            outcome
                .and_then(PerksPanelOutcome::player_command)
                .is_none()
        );
    }

    #[test]
    fn character_names_fall_back_to_the_profile_for_unspawned_roster_entries() {
        use robin_engine::campaign::Campaign;
        use robin_engine::profiles::{CharacterProfile, CharacterProfileIdx, ProfileManager};

        let profiles = ProfileManager {
            characters: vec![CharacterProfile {
                filename: "RobinHood".to_owned(),
                profile_name: "Robin des bois".to_owned(),
                ..CharacterProfile::default()
            }],
            ..ProfileManager::default()
        };
        let portraits = crate::ui_panel::PortraitCache::new();
        let menu_text = crate::ingame_menu::resources::MenuText::default();
        let mut campaign = Campaign::default();
        campaign.characters.push(PcDescription {
            character_profile_idx: Some(CharacterProfileIdx(0)),
            ..PcDescription::default()
        });

        // A character that never spawned has no `PcStatus::name`; the row label
        // comes from the profile the campaign created it from.
        assert_eq!(
            super::character_names(&campaign, &profiles, &portraits, &menu_text),
            vec!["Robin des bois".to_owned()]
        );

        // A spawned/renamed character keeps its live status name.
        campaign.characters[0].status.name = "Robin".to_owned();
        assert_eq!(
            super::character_names(&campaign, &profiles, &portraits, &menu_text),
            vec!["Robin".to_owned()]
        );
    }
}
