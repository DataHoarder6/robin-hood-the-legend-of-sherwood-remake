//! "Report a bug" screen: one text field, one button, and the bundle path.
//!
//! The player reaches this from the pause menu (one click after Escape), types
//! what went wrong and presses Enter. Filing itself happens at the next frame
//! boundary (`game_session::report`) and on a worker thread; this screen only
//! reserves the bundle directory and queues the report, then shows where the
//! evidence went.
//!
//! It is deliberately modal — a report is written against the state the player
//! is looking at — but it never resumes or pauses the mission on its own: the
//! pause menu it was opened from stays in charge of that.
//!
//! Rendering follows [`super::yesno`]: one centred panel, the panel's own text
//! box font, and the game's round wax-seal buttons for Submit / Cancel.

use robin_engine::coordinates as engine_coordinates;
use robin_engine::sprite::BBox;

use crate::gfx_types::{GameEvent, Keycode};
use crate::renderer::Renderer;
use crate::ui::UiKeyboard;
use crate::widget::{FrameWnd, WidgetInputField};

use super::layout::{
    FALLBACK_PANEL_EDGE, FALLBACK_PANEL_FILL, MenuRect, MenuTransform, TextAlign, TooltipState,
    VAlign, dim_screen, draw_background, enter_modal_gpu_phase, render_clipped_text_in_box_font,
};
use super::resources::{IngameMenuResources, MT_INFOBULLE_BUTTON_NO, SealButton};
use super::widget_bridge::{self, ModalCursor, ModalInputState, ModalScreenIo, ScreenFrame};

/// Virtual window geometry.
const WIN_W: i32 = 540;
const WIN_H: i32 = 260;
const PAD: i32 = 20;
const PROMPT_Y: i32 = 20;
const FIELD_Y: i32 = 48;
const FIELD_H: i32 = 92;
const STATUS_Y: i32 = 148;
const HINT_Y: i32 = 176;
const BUTTON_Y: i32 = 204;
const BUTTON_GAP: i32 = 18;

const ID_SUBMIT: u32 = 0;
const ID_CANCEL: u32 = 1;

/// What the screen did this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BugReportOutcome {
    /// Still editing.
    Pending,
    /// The report is queued and its bundle directory is named in the path.
    Queued { directory: String },
    /// The player dismissed the screen without reporting.
    Cancelled,
}

/// One-frame state for the bug-report screen.
pub struct BugReportModalState {
    field: WidgetInputField,
    frame: FrameWnd,
    input_state: ModalInputState,
    tooltip: TooltipState,
    transform: MenuTransform,
    win_x: i32,
    win_y: i32,
    /// An empty keyboard: text insertion goes through `text_input`, and the
    /// buttons own the only keyboard bindings (Enter / Escape).
    empty_keyboard: UiKeyboard,
    status: String,
    /// Whether another seat exists; only then is a second report worth asking for.
    multiplayer: bool,
    queued_directory: Option<String>,
    outcome: BugReportOutcome,
    close_next_frame: bool,
}

impl BugReportModalState {
    /// `multiplayer` is whether this session has a transport: a report covers
    /// one seat, so the other seats have to file their own.
    pub fn new(
        event_pump: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        multiplayer: bool,
    ) -> Self {
        let transform = MenuTransform::for_renderer(renderer);
        let win_x = (super::layout::MENU_W - WIN_W) / 2;
        let win_y = (super::layout::MENU_H - WIN_H) / 2;

        let mut field = WidgetInputField::new(ID_CANCEL + 1);
        field.max_length = 2_000;
        field.enter_edit_mode();
        // The window owns text input while this screen is up; without this the
        // OS reports no characters at all.
        crate::window::start_text_input();

        let (btn_w, btn_h) = resources.seal_pair_dimensions(SealButton::Ok, SealButton::Cancel);
        let total_w = 2 * btn_w + BUTTON_GAP;
        let start_x = win_x + (WIN_W - total_w) / 2;
        let mut frame = FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            ID_SUBMIT,
            "",
            true,
            robin_engine::resource_ids::RHID_OK,
            start_x,
            win_y + BUTTON_Y,
            btn_w,
            btn_h,
        ));
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            ID_CANCEL,
            "",
            true,
            robin_engine::resource_ids::RHID_CANCEL,
            start_x + btn_w + BUTTON_GAP,
            win_y + BUTTON_Y,
            btn_w,
            btn_h,
        ));
        widget_bridge::attach_alpha_masks(&mut frame, resources, renderer);
        if let Some(widget) = frame.widget_mut(ID_CANCEL) {
            widget
                .base_mut()
                .set_tooltip_text(&resources.menu_text.get(MT_INFOBULLE_BUTTON_NO));
        }

        Self {
            field,
            frame,
            input_state: ModalInputState::from_window(event_pump, transform),
            tooltip: TooltipState::new(),
            transform,
            win_x,
            win_y,
            empty_keyboard: UiKeyboard::default(),
            status: if multiplayer {
                "Describe what went wrong. Enter files this seat; every player must \
                 file their own report. Escape cancels."
                    .to_owned()
            } else {
                "Describe what went wrong. Enter files it; Escape cancels.".to_owned()
            },
            multiplayer,
            queued_directory: None,
            outcome: BugReportOutcome::Pending,
            close_next_frame: false,
        }
    }

    pub fn outcome(&self) -> &BugReportOutcome {
        &self.outcome
    }

    pub fn tick(&mut self, io: &mut ModalScreenIo<'_, '_>) -> BugReportOutcome {
        // The queued path was on screen for the frame that produced it; now
        // the player's next action dismisses the screen.
        if self.close_next_frame {
            self.stop_text_input();
            self.outcome = match self.queued_directory.take() {
                Some(directory) => BugReportOutcome::Queued { directory },
                None => BugReportOutcome::Cancelled,
            };
            return self.outcome.clone();
        }
        if !matches!(self.outcome, BugReportOutcome::Pending) {
            return self.outcome.clone();
        }

        let screen = ScreenFrame::poll(io);
        self.handle_events(&screen.events, screen.transform);
        screen.begin_draw(io.renderer);
        self.draw_content(io.renderer, io.resources);
        screen.finish(io, &self.input_state);
        self.outcome.clone()
    }

    /// Advance the screen from one event batch without drawing or presenting.
    pub fn handle_events(&mut self, events: &[GameEvent], transform: MenuTransform) {
        self.transform = transform;
        if self.close_next_frame {
            return;
        }
        for event in events {
            self.input_state.update_from_event(event, transform);
            match event {
                GameEvent::Quit => self.cancel(),
                GameEvent::KeyDown { keycode, .. } => match keycode {
                    Keycode::Escape => self.cancel(),
                    Keycode::Return | Keycode::KpEnter => self.submit(),
                    _ => {
                        Self::edit_field(&mut self.field, event);
                    }
                },
                _ => {}
            }
        }
        // Text insertion (IME commit) is one-shot per frame and reaches the
        // field through the same widget path the save screen uses.
        self.feed_text();
        self.process_buttons();
    }

    fn edit_field(field: &mut WidgetInputField, event: &GameEvent) {
        match event {
            GameEvent::KeyDown { keycode, .. } => match keycode {
                Keycode::Backspace => {
                    field.backspace();
                }
                Keycode::Delete => {
                    field.delete_char();
                }
                Keycode::Left => field.move_caret_left(),
                Keycode::Right => field.move_caret_right(),
                Keycode::Home => field.move_caret_home(),
                Keycode::End => field.move_caret_end(),
                _ => (),
            },
            GameEvent::TextInput { .. } => {}
            _ => (),
        }
    }

    fn feed_text(&mut self) {
        let events = {
            let input = self.input_state.as_widget_input();
            crate::widget::WidgetInput {
                mouse_position: input.mouse_position,
                mouse_z: input.mouse_z,
                mouse_button: crate::ui::MouseButtons::empty(),
                keyboard: &self.empty_keyboard,
                text_input: input.text_input,
                capture: None,
            }
        };
        self.field.process_input(&events);
        if self.field.base.state != crate::ui::UiState::SelectedEditable {
            self.field.enter_edit_mode();
        }
        self.input_state.end_frame();
    }

    fn process_buttons(&mut self) {
        let events = {
            let input = self
                .input_state
                .as_widget_input_at(engine_coordinates::ScreenPoint::new(
                    self.input_state.virt_x,
                    self.input_state.virt_y,
                ));
            self.frame.process_input(&input)
        };
        for event in events {
            if event.msg_type != crate::ui::UiMsg::WidgetActivated {
                continue;
            }
            match event.origin_widget_id {
                ID_SUBMIT => self.submit(),
                ID_CANCEL => self.cancel(),
                id => panic!("bug-report screen received activation from unknown widget {id}"),
            }
        }
    }

    fn submit(&mut self) {
        let description = self.field.edit_text.trim().to_owned();
        if description.is_empty() {
            self.status = "A report needs a description.".to_owned();
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        match crate::bug_bundle::reserve(crate::bug_bundle::Origin::Menu).and_then(|directory| {
            let path = directory.display().to_string();
            crate::bug_bundle::enqueue(directory, &description, "", crate::bug_bundle::Origin::Menu)
                .map(|()| path)
        }) {
            Ok(path) => {
                self.status = if self.multiplayer {
                    format!("Report queued for this seat. Evidence goes to {path}.")
                } else {
                    format!("Report queued. Evidence goes to {path}.")
                };
                self.queued_directory = Some(path);
                self.close_next_frame = true;
            }
            Err(error) => {
                self.status = format!("Could not queue the report: {error:#}");
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.status = "Browser builds have no local bundle; use the report link.".to_owned();
        }
    }

    fn cancel(&mut self) {
        if self.queued_directory.is_none() {
            self.stop_text_input();
            self.outcome = BugReportOutcome::Cancelled;
        }
    }

    fn stop_text_input(&mut self) {
        crate::window::stop_text_input();
    }

    /// Draw the panel, prompt, field, status and buttons.
    fn draw_content(&mut self, renderer: &mut Renderer, resources: &IngameMenuResources) {
        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        if let Some(background) = resources.menu_bg_small {
            draw_background(
                renderer,
                self.transform,
                &background,
                self.win_x,
                self.win_y,
                WIN_W,
                WIN_H,
            );
        } else {
            let (sx, sy) = self.transform.to_screen(self.win_x, self.win_y);
            renderer.fill_screen(
                Some(&BBox::from_coords(
                    sx as f32,
                    sy as f32,
                    (sx + WIN_W) as f32,
                    (sy + WIN_H) as f32,
                )),
                FALLBACK_PANEL_FILL,
            );
            renderer.draw_rect_outline_screen(sx, sy, sx + WIN_W, sy + WIN_H, FALLBACK_PANEL_EDGE);
        }
        let Some(font) = resources.popup_font_any() else {
            widget_bridge::draw_frame_buttons(renderer, resources, self.transform, &self.frame);
            return;
        };
        let box_at = |y: i32, h: i32| MenuRect {
            x: self.win_x + PAD,
            y: self.win_y + y,
            w: WIN_W - 2 * PAD,
            h,
        };
        render_clipped_text_in_box_font(
            renderer,
            font,
            self.transform,
            "Report a bug",
            box_at(PROMPT_Y, 20),
            TextAlign::Center,
            VAlign::Top,
        );
        // The field's own box: the typed text, top-left, with a caret while it
        // is the edit target.
        let field_box = box_at(FIELD_Y, FIELD_H);
        let (fx, fy) = self.transform.to_screen(field_box.x, field_box.y);
        let (fx2, fy2) = self
            .transform
            .to_screen(field_box.x + field_box.w, field_box.y + field_box.h);
        renderer.fill_screen(
            Some(&BBox::from_coords(
                fx as f32, fy as f32, fx2 as f32, fy2 as f32,
            )),
            FALLBACK_PANEL_FILL,
        );
        renderer.draw_rect_outline_screen(fx, fy, fx2, fy2, FALLBACK_PANEL_EDGE);
        let mut text = self.field.edit_text.clone();
        // One visible caret position: while editing, typed characters land at
        // the end of the buffer, which is where the caret is drawn.
        text.push('|');
        render_clipped_text_in_box_font(
            renderer,
            font,
            self.transform,
            &text,
            MenuRect {
                x: field_box.x + 6,
                y: field_box.y + 6,
                w: field_box.w - 12,
                h: field_box.h - 12,
            },
            TextAlign::Left,
            VAlign::Top,
        );
        render_clipped_text_in_box_font(
            renderer,
            font,
            self.transform,
            &self.status,
            box_at(STATUS_Y, 24),
            TextAlign::Center,
            VAlign::Top,
        );
        render_clipped_text_in_box_font(
            renderer,
            font,
            self.transform,
            "Enter / the seal files it. Escape cancels.",
            box_at(HINT_Y, 20),
            TextAlign::Center,
            VAlign::Top,
        );
        widget_bridge::draw_frame_buttons(renderer, resources, self.transform, &self.frame);
        let mouse =
            engine_coordinates::ScreenPoint::new(self.input_state.virt_x, self.input_state.virt_y);
        self.tooltip.update(&self.frame, mouse);
        self.tooltip
            .draw(renderer, font, self.transform, &self.frame, mouse);
    }

    /// Draw over a caller's framebuffer, used by tests and nested callers.
    pub fn render_overlay(
        &mut self,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) {
        self.draw_content(renderer, resources);
        if let Some(cursor) = cursor {
            cursor.draw(renderer, &self.input_state);
        }
    }

    #[cfg(test)]
    pub(crate) fn description(&self) -> &str {
        &self.field.edit_text
    }
}

/// The screen is not `Drop`-safe for text input: a mission that exits while it
/// is up would otherwise leave the window in text-input mode.
impl Drop for BugReportModalState {
    fn drop(&mut self) {
        self.stop_text_input();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    fn state(resources: &IngameMenuResources) -> BugReportModalState {
        let mut field = WidgetInputField::new(ID_CANCEL + 1);
        field.enter_edit_mode();
        let mut frame = FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
        frame.add_widget_absolute(widget_bridge::make_button(
            ID_SUBMIT, "Submit", 0, 0, 80, 30,
        ));
        frame.add_widget_absolute(widget_bridge::make_button(
            ID_CANCEL, "Cancel", 90, 0, 80, 30,
        ));
        let _ = resources;
        BugReportModalState {
            field,
            frame,
            input_state: ModalInputState::new(),
            tooltip: TooltipState::new(),
            transform: MenuTransform::centered(640, 480),
            win_x: 0,
            win_y: 0,
            empty_keyboard: UiKeyboard::default(),
            status: String::new(),
            multiplayer: false,
            queued_directory: None,
            outcome: BugReportOutcome::Pending,
            close_next_frame: false,
        }
    }

    #[test]
    fn the_description_round_trips_through_the_field() {
        let resources = IngameMenuResources::stub();
        let mut state = state(&resources);
        state.handle_events(
            &[GameEvent::TextInput {
                text: "stuck on the sign".into(),
            }],
            MenuTransform::centered(640, 480),
        );
        assert_eq!(state.description(), "stuck on the sign");
        state.handle_events(
            &[GameEvent::KeyDown {
                keycode: Keycode::Backspace,
                physical_key: Some(KeyCode::Backspace),
            }],
            MenuTransform::centered(640, 480),
        );
        assert_eq!(state.description(), "stuck on the sig");
    }

    #[test]
    fn an_empty_description_is_refused_without_filing() {
        let resources = IngameMenuResources::stub();
        let mut state = state(&resources);
        state.submit();
        assert_eq!(state.outcome(), &BugReportOutcome::Pending);
        assert_eq!(state.status, "A report needs a description.");
    }

    #[test]
    fn escape_cancels() {
        let resources = IngameMenuResources::stub();
        let mut state = state(&resources);
        state.handle_events(
            &[GameEvent::KeyDown {
                keycode: Keycode::Escape,
                physical_key: Some(KeyCode::Escape),
            }],
            MenuTransform::centered(640, 480),
        );
        assert_eq!(state.outcome(), &BugReportOutcome::Cancelled);
    }

    /// Enter files what was typed: the directory is reserved now, the bundle is
    /// written at the next frame boundary.
    #[test]
    fn enter_with_text_reserves_a_bundle_and_queues() {
        let home = tempfile::tempdir().unwrap();
        let _guard = crate::install_home::ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // Safety: the install-home lock above serializes every reader.
        unsafe { std::env::set_var("ROBINHOOD_INSTALL_HOME", home.path()) };
        let resources = IngameMenuResources::stub();
        let mut state = state(&resources);
        state.handle_events(
            &[GameEvent::TextInput {
                text: "mission complete, no debriefing".into(),
            }],
            MenuTransform::centered(640, 480),
        );
        state.handle_events(
            &[GameEvent::KeyDown {
                keycode: Keycode::Return,
                physical_key: Some(KeyCode::Enter),
            }],
            MenuTransform::centered(640, 480),
        );
        let queued = crate::bug_bundle::take_requests();
        assert_eq!(queued.len(), 1, "one Enter files one report");
        assert_eq!(queued[0].description, "mission complete, no debriefing");
        assert!(
            queued[0].directory.is_dir(),
            "the bundle dir exists already"
        );
    }
}
