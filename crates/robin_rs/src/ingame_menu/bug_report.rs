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
use crate::native_font::Font;
use crate::renderer::Renderer;
use crate::ui::UiKeyboard;
use crate::widget::{FrameWnd, WidgetInputField};

use super::layout::{
    FALLBACK_PANEL_EDGE, FALLBACK_PANEL_FILL, MenuRect, MenuTransform, TextAlign, TooltipState,
    VAlign, dim_screen, draw_background, enter_modal_gpu_phase, render_clipped_text_in_box_font,
    render_text_virt_font, wrap_text_for_box_font,
};
use super::resources::{IngameMenuResources, MT_INFOBULLE_BUTTON_NO, SealButton};
use super::widget_bridge::{self, ModalCursor, ModalInputState, ModalScreenIo, ScreenFrame};

/// Virtual window geometry. The field is tall enough to show many wrapped
/// lines and the status has room for a multi-line error: a status the modal
/// dismisses in the same frame is not a visible error.
const WIN_W: i32 = 540;
const WIN_H: i32 = 460;
const PAD: i32 = 20;
const PROMPT_Y: i32 = 14;
const FIELD_Y: i32 = 40;
const FIELD_H: i32 = 250;
const STATUS_Y: i32 = 298;
const STATUS_H: i32 = 70;
const HINT_Y: i32 = 372;
const BUTTON_Y: i32 = 394;
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

/// What the submit path actually did, before any policy is applied. The I/O
/// lives in `submit`; this is the only input the decision needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SubmitAttempt {
    /// A bundle was reserved and the request queued at this path.
    Queued(String),
    /// Reserve or enqueue failed with this message.
    QueueFailed(String),
    /// This build has no bundle mechanism at all (browser).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    Unsupported,
}

/// The visible result of a submission. A queued report closes the modal; a
/// failure keeps it open so the error stays on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SubmitDecision {
    Queued {
        directory: String,
        status: String,
    },
    Failed {
        error: String,
        /// A bundle was saved even though filing failed: remember it so a
        /// second Enter cannot file a duplicate.
        directory: Option<String>,
    },
}

/// Pure policy for a submission. It does no I/O: `submit` gathers the facts
/// and applies the result. Missing configuration and an unsupported build are
/// failures that must be read, not a success that quietly closes the screen.
pub(crate) fn decide_submit(
    attempt: SubmitAttempt,
    endpoint_configured: bool,
    multiplayer: bool,
) -> SubmitDecision {
    match attempt {
        SubmitAttempt::Unsupported => SubmitDecision::Failed {
            error: "Reporting is not supported in this browser build: there is no local \
                    bundle. Use the report link on the website instead."
                .to_owned(),
            directory: None,
        },
        SubmitAttempt::QueueFailed(error) => SubmitDecision::Failed {
            error: format!("Could not queue the report: {error}"),
            directory: None,
        },
        SubmitAttempt::Queued(path) if !endpoint_configured => SubmitDecision::Failed {
            error: format!(
                "Evidence was saved locally at {path}. No GitHub issue was filed: \
                 {} is not set, so the report was not uploaded. Ask the operator to set \
                 {} to the bug-receiver URL, then file again.",
                "ROBIN_BUG_ENDPOINT", "ROBIN_BUG_ENDPOINT"
            ),
            directory: Some(path),
        },
        SubmitAttempt::Queued(path) => SubmitDecision::Queued {
            status: if multiplayer {
                format!("Report queued for this seat. Evidence goes to {path}.")
            } else {
                format!("Report queued. Evidence goes to {path}.")
            },
            directory: path,
        },
    }
}

/// The font the typed report text uses. The edit-field face is the readable
/// one every other menu text field uses; the popup face is only a fallback
/// for the surrounding labels.
pub(crate) fn report_field_font(resources: &IngameMenuResources) -> Option<&Font> {
    resources
        .edit_field_font_any()
        .or_else(|| resources.popup_font_any())
}

/// How many wrapped lines fit in a box of `box_h` at `line_height` pixels.
pub(crate) fn visible_line_count(box_h: i32, line_height: i32) -> usize {
    (box_h / line_height.max(1)).max(1) as usize
}

/// Pure scroll window for the report field: the first wrapped line to draw.
/// The caret line is always kept inside the window; wheel scrolling may move
/// it elsewhere, clamped to the content that exists.
pub(crate) fn scroll_window(
    total_lines: usize,
    visible_lines: usize,
    caret_line: usize,
    current: usize,
) -> usize {
    if visible_lines == 0 {
        return 0;
    }
    let max_start = total_lines.saturating_sub(visible_lines);
    let mut start = current.min(max_start);
    if caret_line < start {
        start = caret_line;
    } else if caret_line >= start + visible_lines {
        // Caret is on or past the last visible line; scroll it into view.
        start = caret_line + 1 - visible_lines;
    }
    start.min(max_start)
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
    /// First wrapped line shown in the field.
    scroll_lines: usize,
    /// `true` while typing moves the caret and the view follows it; the wheel
    /// turns this off so the player can read earlier lines.
    follow_caret: bool,
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
            scroll_lines: 0,
            follow_caret: true,
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
                        if matches!(
                            keycode,
                            Keycode::Backspace
                                | Keycode::Delete
                                | Keycode::Left
                                | Keycode::Right
                                | Keycode::Home
                                | Keycode::End
                        ) {
                            self.follow_caret = true;
                        }
                        Self::edit_field(&mut self.field, event);
                    }
                },
                GameEvent::TextInput { .. } => self.follow_caret = true,
                GameEvent::MouseWheel(delta) => {
                    // The wheel is the player's own view, not caret movement.
                    self.follow_caret = false;
                    if *delta > 0 {
                        self.scroll_lines = self.scroll_lines.saturating_add(*delta as usize);
                    } else {
                        self.scroll_lines = self
                            .scroll_lines
                            .saturating_sub(delta.unsigned_abs() as usize);
                    }
                }
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
        if self.queued_directory.is_some() {
            // A bundle is already on disk (possibly without an upload). One
            // screen files one report; another Enter must not duplicate it.
            return;
        }
        #[cfg(target_arch = "wasm32")]
        let attempt = SubmitAttempt::Unsupported;
        #[cfg(not(target_arch = "wasm32"))]
        let attempt = match crate::bug_bundle::reserve(crate::bug_bundle::Origin::Menu).and_then(
            |directory| {
                let path = directory.display().to_string();
                crate::bug_bundle::enqueue(
                    directory,
                    &description,
                    "",
                    crate::bug_bundle::Origin::Menu,
                )
                .map(|()| path)
            },
        ) {
            Ok(path) => SubmitAttempt::Queued(path),
            Err(error) => SubmitAttempt::QueueFailed(format!("{error:#}")),
        };
        #[cfg(target_arch = "wasm32")]
        let endpoint_configured = false;
        #[cfg(not(target_arch = "wasm32"))]
        let endpoint_configured = crate::bug_bundle::endpoint_configured();
        let decision = decide_submit(attempt, endpoint_configured, self.multiplayer);
        self.apply_decision(decision);
    }

    fn apply_decision(&mut self, decision: SubmitDecision) {
        match decision {
            SubmitDecision::Queued { directory, status } => {
                self.status = status;
                self.queued_directory = Some(directory);
                self.close_next_frame = true;
            }
            SubmitDecision::Failed { error, directory } => {
                // No `close_next_frame`: the error has to stay on screen.
                self.status = error;
                if let Some(directory) = directory {
                    self.queued_directory = Some(directory);
                }
            }
        }
    }

    fn cancel(&mut self) {
        self.stop_text_input();
        self.outcome = BugReportOutcome::Cancelled;
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
        // Typed text uses the same readable EditField face as every other menu
        // input; only the surrounding labels use the popup face.
        let field_font = report_field_font(resources).unwrap_or(font);
        let inner = MenuRect {
            x: field_box.x + 6,
            y: field_box.y + 6,
            w: field_box.w - 12,
            h: field_box.h - 12,
        };
        let line_h = field_font.height().max(1) as i32;
        let visible_lines = visible_line_count(inner.h, line_h);
        let wrap_w = (inner.w - 4).max(1);
        let text = &self.field.edit_text;
        let caret_byte = text
            .char_indices()
            .nth(self.field.caret_offset)
            .map_or(text.len(), |(byte, _)| byte);
        // Show the caret where the edit buffer actually has it, not always at
        // the end of the text.
        let mut display = String::with_capacity(text.len() + 1);
        display.push_str(&text[..caret_byte]);
        display.push('|');
        display.push_str(&text[caret_byte..]);
        let wrapped = wrap_text_for_box_font(field_font, &display, wrap_w, usize::MAX);
        let total_lines = wrapped.lines.len();
        let caret_line =
            wrap_text_for_box_font(field_font, &display[..caret_byte + 1], wrap_w, usize::MAX)
                .lines
                .len()
                .saturating_sub(1);
        let first_line = if self.follow_caret {
            scroll_window(total_lines, visible_lines, caret_line, self.scroll_lines)
        } else {
            self.scroll_lines
                .min(total_lines.saturating_sub(visible_lines))
        };
        self.scroll_lines = first_line;
        for (index, line) in wrapped
            .lines
            .iter()
            .skip(first_line)
            .take(visible_lines)
            .enumerate()
        {
            render_text_virt_font(
                renderer,
                field_font,
                self.transform,
                &line.text,
                inner.x + 2,
                inner.y + index as i32 * line_h,
            );
        }
        render_clipped_text_in_box_font(
            renderer,
            font,
            self.transform,
            &self.status,
            box_at(STATUS_Y, STATUS_H),
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::font::TrueTypeFont;
    use winit::keyboard::KeyCode;

    /// Hold the process-global environment lock and restore both variables on
    /// drop, so a submission test cannot leak its configuration into a neighbour.
    struct EnvGuard {
        _home: tempfile::TempDir,
        old_home: Option<std::ffi::OsString>,
        old_endpoint: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn with_endpoint(endpoint: Option<&str>) -> Self {
            let home = tempfile::tempdir().unwrap();
            let lock = crate::install_home::ENV_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let old_home = std::env::var_os("ROBINHOOD_INSTALL_HOME");
            let old_endpoint = std::env::var_os(crate::bug_bundle::ENDPOINT_ENV);
            // Safety: the lock above serializes every reader and writer of these.
            unsafe {
                std::env::set_var("ROBINHOOD_INSTALL_HOME", home.path());
                match endpoint {
                    Some(value) => {
                        std::env::set_var(crate::bug_bundle::ENDPOINT_ENV, value);
                    }
                    None => std::env::remove_var(crate::bug_bundle::ENDPOINT_ENV),
                }
            }
            Self {
                _home: home,
                old_home,
                old_endpoint,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // Safety: the guard owns the lock for its whole lifetime.
            unsafe {
                match &self.old_home {
                    Some(value) => std::env::set_var("ROBINHOOD_INSTALL_HOME", value),
                    None => std::env::remove_var("ROBINHOOD_INSTALL_HOME"),
                }
                match &self.old_endpoint {
                    Some(value) => std::env::set_var(crate::bug_bundle::ENDPOINT_ENV, value),
                    None => std::env::remove_var(crate::bug_bundle::ENDPOINT_ENV),
                }
            }
        }
    }

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
            scroll_lines: 0,
            follow_caret: true,
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

    /// The player must see why a report did not get filed: a status line that
    /// is rendered for one frame and then dismissed is not a visible error.
    #[test]
    fn a_missing_endpoint_keeps_the_modal_open_and_surfaces_the_error() {
        let _guard = EnvGuard::with_endpoint(None);
        let resources = IngameMenuResources::stub();
        let mut state = state(&resources);
        state.handle_events(
            &[GameEvent::TextInput {
                text: "stuck on the sign".into(),
            }],
            MenuTransform::centered(640, 480),
        );
        state.submit();
        assert_eq!(
            state.outcome(),
            &BugReportOutcome::Pending,
            "a failed submission must not dismiss the modal"
        );
        assert!(
            !state.close_next_frame,
            "the modal must stay open so the player can read the error"
        );
        assert!(
            state.status.contains(crate::bug_bundle::ENDPOINT_ENV),
            "the status must name the missing configuration, got {:?}",
            state.status
        );
        assert!(
            state.status.contains("Evidence was saved locally at"),
            "the status must say where the evidence is, got {:?}",
            state.status
        );
        assert!(
            state.status.contains("No GitHub issue was filed"),
            "the status must say that no issue was filed, got {:?}",
            state.status
        );
        assert!(
            state.status.contains("not uploaded"),
            "the status must say that nothing was uploaded, got {:?}",
            state.status
        );
        assert!(
            state
                .status
                .contains("Ask the operator to set ROBIN_BUG_ENDPOINT"),
            "the status must name the provisioning step, got {:?}",
            state.status
        );
        assert!(
            state.status.contains("file again"),
            "the status must tell the player what to do next, got {:?}",
            state.status
        );
    }

    /// The old 92px box dropped overflow with no scroll, so a longer report
    /// could not be read at all. The field needs real height before scrolling
    /// can show the text it wraps to.
    #[test]
    fn the_report_field_is_tall_enough_to_scroll_long_text() {
        assert!(
            visible_line_count(FIELD_H, 15) >= 10,
            "the field must show many wrapped lines"
        );
        assert!(
            visible_line_count(STATUS_H, 15) >= 4,
            "a multi-line error must fit where the status was shown"
        );
    }

    #[test]
    fn report_field_font_prefers_the_edit_field_face() {
        let make_font = |height| {
            Font::TrueType(TrueTypeFont::from_parts(
                &[0; 32],
                height,
                0,
                0,
                &[0; 32],
                0x00FF_FFFF,
                &[],
            ))
        };
        let mut resources = IngameMenuResources::stub();
        resources.fonts.popup_scroll_any = Some(make_font(12));
        assert_eq!(
            report_field_font(&resources).unwrap().height(),
            12,
            "the popup face is the last fallback"
        );
        resources.fonts.edit_field_any = Some(make_font(18));
        assert_eq!(
            report_field_font(&resources).unwrap().height(),
            18,
            "typed text uses the readable edit-field face"
        );
    }

    #[test]
    fn report_field_scroll_keeps_the_caret_in_view() {
        // Ten wrapped lines, four visible: a caret on line seven starts the
        // window at line four, with the caret on its last visible line.
        assert_eq!(scroll_window(10, 4, 7, 0), 4);
        // A caret already inside the window leaves the player's scroll alone.
        assert_eq!(scroll_window(10, 4, 5, 4), 4);
        // Caret above the window scrolls back to it.
        assert_eq!(scroll_window(10, 4, 1, 6), 1);
        // Scroll is clamped to the content that exists.
        assert_eq!(scroll_window(3, 4, 2, 9), 0);
    }

    #[test]
    fn an_unsupported_build_surfaces_an_error_instead_of_closing() {
        let decision = decide_submit(SubmitAttempt::Unsupported, false, false);
        assert!(matches!(decision, SubmitDecision::Failed { .. }));
        let resources = IngameMenuResources::stub();
        let mut state = state(&resources);
        state.apply_decision(decision);
        assert_eq!(state.outcome(), &BugReportOutcome::Pending);
        assert!(!state.close_next_frame);
        assert!(state.status.contains("not supported"));
    }

    #[test]
    fn a_queued_report_is_distinguishable_from_a_failure() {
        let failed = decide_submit(SubmitAttempt::Queued("dir".into()), false, false);
        assert!(matches!(
            failed,
            SubmitDecision::Failed {
                directory: Some(_),
                ..
            }
        ));
        let queued = decide_submit(SubmitAttempt::Queued("dir".into()), true, false);
        assert!(matches!(queued, SubmitDecision::Queued { .. }));
    }
}
