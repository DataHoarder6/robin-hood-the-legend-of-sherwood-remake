//! Modal Yes/No confirmation dialog.
//!
//! A 400x200 window using `RHID_MENU_BACKGROUND_SMALL` with the message
//! word-wrapped inside a `(25,50)..(375,120)` label, the round Yes / No
//! wax-seal buttons (`RHID_OK` / `RHID_CANCEL`) centred horizontally at
//! y=130 with 18px spacing, and shortcuts binding Return / Numpad Enter /
//! Space → Yes and Escape → No. The visible hint lists Enter and Escape only;
//! Space is a second confirm-OK path, not a new message.
//!
//! Buttons are driven by the [`crate::widget`] system: a [`FrameWnd`]
//! holds two [`WidgetButton`]s whose state machines handle hover, push
//! and select transitions.  The bridge module renders them using the
//! existing sprite pipeline.

use crate::ingame_menu::resources::SealButton;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::sprite::BBox;
use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

use crate::gfx_types::GameEvent;
use crate::renderer::Renderer;
use crate::ui::{KeyState as WidgetKeyState, MouseButtons, TypeWriter, UiEvent, UiMsg};
use crate::widget::WidgetInput;

use super::layout::{
    FALLBACK_PANEL_EDGE, FALLBACK_PANEL_FILL, MenuRect, MenuTransform, TextAlign, TooltipState,
    VAlign, dim_screen, draw_background, enter_modal_gpu_phase, render_clipped_text_in_box_font,
};
use super::resources::{IngameMenuResources, MT_INFOBULLE_BUTTON_NO, MT_INFOBULLE_BUTTON_YES};
use super::widget_bridge::{self, ModalCursor, ModalInputState, ModalScreenIo, ScreenFrame};

/// Virtual window geometry.
pub const WIN_W: i32 = 400;
pub const WIN_H: i32 = 200;

/// Shortcut hint under the seal row. The original only rendered hover
/// tooltips, so the keyboard bindings were invisible until discovered by
/// trial; the bug-report screen already shows its bindings, so do the same
/// here.
const SHORTCUT_HINT: &str = "Enter = Yes · Escape = No";
/// Top of the hint box, below the 130-px seal row.
const HINT_Y: i32 = 166;
const HINT_H: i32 = 20;

/// Message label bounding box `(25,50)..(375,120)`.
const MSG_X: i32 = 25;
const MSG_Y: i32 = 50;
const MSG_W: i32 = 350; // 375 - 25
const MSG_H: i32 = 70; // 120 - 50

/// Horizontal spacing between the Yes / No buttons.
const BUTTON_GAP: i32 = 18;

/// Widget IDs for the two buttons.
const ID_YES: u32 = 0;
const ID_NO: u32 = 1;

/// Frames the mission-start confirmation spends flying in from the right
/// edge. Nine ticks at 60 Hz is ~150 ms — the same budget the leave-mission
/// popup uses in [`super::mission_state`].
const FLY_IN_TICKS: f32 = 9.0;

/// Horizontal offset from the centred position of a dialog flying in from
/// the right edge, as a pure function of `progress` in `[0, 1]`.
///
/// At progress 0 the panel sits just off the menu's right edge; at progress
/// 1 it lands centred. The ease-out cubic is monotonic decreasing, so each
/// frame moves the panel left until it settles.
fn fly_in_offset_x(progress: f32) -> i32 {
    let t = 1.0 - (1.0 - progress.clamp(0.0, 1.0)).powi(3);
    let final_x = (super::layout::MENU_W - WIN_W) / 2;
    // Round rather than truncate: truncation reaches zero a frame early and
    // breaks pixel-level monotonicity on the last two ticks.
    ((super::layout::MENU_W - final_x) as f32 * (1.0 - t)).round() as i32
}

/// Frame-driven progress of the mission-start confirmation's fly-in.
#[derive(Debug, Clone, Copy)]
struct FlyInTransition {
    elapsed_ticks: f32,
}

impl FlyInTransition {
    fn new() -> Self {
        Self { elapsed_ticks: 0.0 }
    }

    fn advance(&mut self) {
        self.elapsed_ticks += 1.0;
    }

    fn offset_x(self) -> i32 {
        fly_in_offset_x(self.elapsed_ticks / FLY_IN_TICKS)
    }
}

/// Resolved result of the modal's widget event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum YesNoChoice {
    Yes,
    No,
}

/// Display the modal confirmation dialog.  Returns `true` if the player
/// chose Yes (or pressed Return / Numpad Enter), `false` if the player
/// chose No (or pressed Escape / closed the window).
pub async fn show_yesno(io: &mut ModalScreenIo<'_, '_>, message: &str) -> bool {
    let mut state = YesNoModalState::new(io.window, io.renderer, io.resources, message.to_string());
    widget_bridge::run_modal(io, |io| state.tick(io)).await
}

/// One-frame state for the standard yes/no modal.
pub struct YesNoModalState {
    message: String,
    frame: crate::widget::FrameWnd,
    input_state: ModalInputState,
    tooltip: TooltipState,
    transform: MenuTransform,
    win_x: i32,
    win_y: i32,
    focus: FrameButtonFocusManager,
    choice: Option<YesNoChoice>,
    /// Set when a seal is activated by mouse or shortcut. The dialog stays
    /// open for one more frame after the choice is recorded so the pressed
    /// sprite is presented at least once; otherwise a fast click can resolve
    /// in the frame it is pressed and appear to do nothing.
    pressed_frame_pending: bool,
    /// `Some` only for the mission-start confirmation; every other Yes/No
    /// dialog keeps opening in place.
    fly_in: Option<FlyInTransition>,
}

impl YesNoModalState {
    pub fn new(
        event_pump: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        message: String,
    ) -> Self {
        let transform = MenuTransform::for_renderer(renderer);

        let win_x = (super::layout::MENU_W - WIN_W) / 2;
        let win_y = (super::layout::MENU_H - WIN_H) / 2;
        // Yes / No are the round wax-seal sprites (`RHID_OK` /
        // `RHID_CANCEL`) with no label, like the original dialog.  Both
        // get the max intrinsic size so they render at native
        // dimensions when centred as a pair.
        let (btn_w, btn_h) = resources.seal_pair_dimensions(SealButton::Ok, SealButton::Cancel);
        // Every confirmation is a choice: the decline seal is structural, not
        // a per-mission option that a caller can forget to pass.
        let n = 2;
        let total_w = n * btn_w + (n - 1) * BUTTON_GAP;
        let start_x = win_x + (WIN_W - total_w) / 2;
        let btn_y = win_y + 130;

        let mut frame = crate::widget::FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            ID_YES,
            "",
            true,
            robin_engine::resource_ids::RHID_OK,
            start_x,
            btn_y,
            btn_w,
            btn_h,
        ));
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            ID_NO,
            "",
            true,
            robin_engine::resource_ids::RHID_CANCEL,
            start_x + btn_w + BUTTON_GAP,
            btn_y,
            btn_w,
            btn_h,
        ));
        // Per-pixel hit masks so the transparent corners around each
        // round seal don't capture clicks.
        widget_bridge::attach_alpha_masks(&mut frame, resources, renderer);

        let yes_tooltip = resources.menu_text.get(MT_INFOBULLE_BUTTON_YES);
        let no_tooltip = resources.menu_text.get(MT_INFOBULLE_BUTTON_NO);
        if let Some(w) = frame.widget_mut(ID_YES) {
            w.base_mut().set_tooltip_text(&yes_tooltip);
        }
        if let Some(w) = frame.widget_mut(ID_NO) {
            w.base_mut().set_tooltip_text(&no_tooltip);
        }

        let mut focus = FrameButtonFocusManager::new();
        configure_focus(&mut focus, &frame);

        let input_state = ModalInputState::from_window(event_pump, transform);

        Self {
            message,
            frame,
            input_state,
            tooltip: TooltipState::new(),
            transform,
            win_x,
            win_y,
            focus,
            choice: None,
            pressed_frame_pending: false,
            fly_in: None,
        }
    }

    /// Mission-start confirmation: the panel flies in from the right edge
    /// over [`FLY_IN_TICKS`] frames. Other Yes/No dialogs use [`Self::new`]
    /// and open in place.
    pub fn new_mission_start_confirmation(
        event_pump: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        message: String,
    ) -> Self {
        let mut state = Self::new(event_pump, renderer, resources, message);
        state.fly_in = Some(FlyInTransition::new());
        state
    }

    /// Current horizontal displacement from the centred position, in
    /// virtual menu units. Zero for dialogs that do not fly in.
    fn fly_in_offset_x(&self) -> i32 {
        self.fly_in.map_or(0, FlyInTransition::offset_x)
    }

    /// The transform shared by drawing and pointer hit-testing. During the
    /// mission-start fly-in the panel is displaced, so input must map screen
    /// coordinates through the same transform the player sees — otherwise a
    /// click on the visible seal lands on the settled, invisible position.
    fn active_transform(&self) -> MenuTransform {
        MenuTransform {
            origin_x: self.transform.origin_x + self.fly_in_offset_x(),
            origin_y: self.transform.origin_y,
        }
    }

    pub fn tick(&mut self, io: &mut ModalScreenIo<'_, '_>) -> Option<bool> {
        if let Some(choice) = self.choice {
            if self.pressed_frame_pending {
                // The activation frame drew the pressed seal. Clear the
                // marker and draw once more, then report the choice.
                self.pressed_frame_pending = false;
            } else {
                return Some(choice == YesNoChoice::Yes);
            }
        }

        let screen = ScreenFrame::poll(io);
        // `handle_events` adopts `screen.transform`, so the content and the
        // cursor drawn by `finish` share this frame's transform.
        self.handle_events(&screen.events, screen.transform);
        screen.begin_draw(io.renderer);
        self.draw_content(io.renderer, io.resources);
        screen.finish(io, &self.input_state);
        if self.pressed_frame_pending {
            // The pressed sprite was drawn this frame; hold the modal open
            // for the host's next tick so the player sees the activation.
            self.pressed_frame_pending = false;
            return None;
        }
        self.result()
    }

    /// Advance the dialog from an event batch without drawing or presenting.
    ///
    /// Nested side screens use this split API so they can draw their picker
    /// first and then place the confirmation over it in the same frame. Pass the
    /// transform returned when polling this batch, not the dialog creation transform.
    pub fn handle_events(
        &mut self,
        events: &[GameEvent],
        transform: MenuTransform,
    ) -> Option<bool> {
        if let Some(fly_in) = &mut self.fly_in {
            fly_in.advance();
        }
        self.transform = transform;
        if self.choice.is_some() {
            return self.result();
        }
        let active_transform = self.active_transform();
        for event in events {
            self.input_state.update_from_event(event, active_transform);
            if matches!(event, GameEvent::Quit) {
                self.resolve(YesNoChoice::No);
            }
        }
        self.process_widget_input();
        self.result()
    }

    /// Draw the dialog over the caller's current framebuffer without
    /// clearing or presenting it.
    pub fn render_overlay(
        &mut self,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) {
        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        self.draw_content(renderer, resources);
        if let Some(c) = cursor {
            c.draw(renderer, &self.input_state);
        }
    }

    pub fn result(&self) -> Option<bool> {
        self.choice.map(|choice| choice == YesNoChoice::Yes)
    }

    fn process_widget_input(&mut self) {
        let hover_before_press = self.input_state.take_hover_before_press();
        let events = {
            let widget_input = self.input_state.as_widget_input();
            if hover_before_press {
                // A menu button leaves `Default` only on a hover frame. When
                // a press arrives with no preceding `MouseMove`, deliver the
                // pointer position once with no button held so the seal under
                // it reaches `Focused`; the real pass then pushes it and the
                // release frame turns that into `WidgetActivated`.
                let hover = WidgetInput {
                    mouse_button: MouseButtons::empty(),
                    ..widget_input
                };
                let _ = self.frame.process_input(&hover);
            }
            let events = self.frame.process_input(&widget_input);
            let mouse_captured = widget_input
                .capture
                .is_some_and(|capture| capture.get().is_some());
            self.focus.process_input(
                &mut self.frame,
                events,
                widget_input.keyboard,
                mouse_captured,
            )
        };
        self.input_state.end_frame();
        self.apply_widget_events(&events);
    }

    fn apply_widget_events(&mut self, events: &[UiEvent]) {
        let mut accepted = false;
        let mut declined = false;
        for event in events {
            if event.msg_type != UiMsg::WidgetActivated {
                continue;
            }
            match event.origin_widget_id {
                ID_YES => accepted = true,
                ID_NO => declined = true,
                id => panic!("yes/no modal received activation from unknown widget {id}"),
            }
        }
        // A stale keyboard focus can emit the blue seal's activation in the
        // same batch as a red click. The decline wins: the two seals must be
        // mutually exclusive, and treating red as accept is the worse error.
        if declined {
            self.note_pressed(ID_NO);
            self.resolve(YesNoChoice::No);
        } else if accepted {
            self.note_pressed(ID_YES);
            self.resolve(YesNoChoice::Yes);
        }
    }

    /// Show the activated seal in its pressed sprite for one frame. Mouse
    /// clicks can press and release inside a single frame, skipping the
    /// widget's pushed state, and keyboard activation never sets it at all —
    /// forcing it here makes both paths visibly acknowledge the click.
    ///
    /// TODO(#72): play the original's seal-activation noise here once
    /// `ModalScreenIo` carries `ScreenAudio`; the yes/no dialog has no audio
    /// service today, so only the visual acknowledgement is wired.
    fn note_pressed(&mut self, id: u32) {
        if self.choice.is_some() || self.pressed_frame_pending {
            return;
        }
        self.pressed_frame_pending = true;
        if let Some(crate::widget::Widget::Button(button)) = self.frame.widget_mut(id) {
            button.base.state = crate::ui::UiState::Pushed;
        }
    }

    /// Resolve at most once. Nested/modal callers may observe the state more
    /// than once while unwinding; a later cancel event must not overwrite an
    /// already-confirmed choice (or vice versa).
    fn resolve(&mut self, choice: YesNoChoice) {
        if self.choice.is_none() {
            self.choice = Some(choice);
        }
    }

    /// Dialog panel, message, buttons and tooltip; the caller owns the modal
    /// phase/dim before and the cursor/present after.
    fn draw_content(&mut self, renderer: &mut Renderer, resources: &IngameMenuResources) {
        // Drawing and pointer hit-testing both use `active_transform`, so a
        // click on the flying seal resolves against what the player sees.
        let draw_transform = self.active_transform();

        if let Some(bg) = resources.menu_bg_small {
            draw_background(
                renderer,
                draw_transform,
                &bg,
                self.win_x,
                self.win_y,
                WIN_W,
                WIN_H,
            );
        } else {
            let (sx, sy) = draw_transform.to_screen(self.win_x, self.win_y);
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

        if let Some(font) = resources.popup_font_any() {
            render_clipped_text_in_box_font(
                renderer,
                font,
                draw_transform,
                &self.message,
                MenuRect {
                    x: self.win_x + MSG_X,
                    y: self.win_y + MSG_Y,
                    w: MSG_W,
                    h: MSG_H,
                },
                TextAlign::Center,
                // Top-origin with word-wrap: the original renders the
                // message with centered text rendering inside
                // the box, wrapping lines from the box top.
                VAlign::Top,
            );
        }

        if let Some(font) = resources.popup_font_any() {
            render_clipped_text_in_box_font(
                renderer,
                font,
                draw_transform,
                SHORTCUT_HINT,
                MenuRect {
                    x: self.win_x,
                    y: self.win_y + HINT_Y,
                    w: WIN_W,
                    h: HINT_H,
                },
                TextAlign::Center,
                VAlign::Top,
            );
        }

        widget_bridge::draw_frame_buttons(renderer, resources, draw_transform, &self.frame);

        let mouse_pt =
            engine_coordinates::ScreenPoint::new(self.input_state.virt_x, self.input_state.virt_y);
        self.tooltip.update(&self.frame, mouse_pt);
        if let Some(font) = resources.popup_font_any() {
            self.tooltip
                .draw(renderer, font, draw_transform, &self.frame, mouse_pt);
        }
    }
}

/// Shortcut focus for the two dialog buttons owned by the frame.
///
/// Stores widget IDs rather than parallel widget state, so mouse input and
/// keyboard focus resolve through the same canonical [`UiEvent`] stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FrameButtonFocusManager {
    group: Vec<FrameButtonEntry>,
    shortcuts: Vec<(KeyCode, crate::widget::WidgetId)>,
    focused_idx: Option<usize>,
    pending_shortcut: Option<KeyCode>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct FrameButtonEntry {
    widget_id: crate::widget::WidgetId,
}

/// Register the dialog's two non-navigable seals and their shortcuts.
///
/// Both the production constructor and the modal-state test helper go through
/// this one seam, so a binding added here is the binding the player gets — a
/// test cannot exercise a shortcut the dialog never registers.
fn configure_focus(focus: &mut FrameButtonFocusManager, frame: &crate::widget::FrameWnd) {
    focus.add_button(frame, ID_YES);
    focus.add_shortcut(ID_YES, KeyCode::Enter);
    focus.add_shortcut(ID_YES, KeyCode::NumpadEnter);
    focus.add_shortcut(ID_YES, KeyCode::Space);
    focus.add_button(frame, ID_NO);
    focus.add_shortcut(ID_NO, KeyCode::Escape);
}

impl FrameButtonFocusManager {
    fn new() -> Self {
        Self {
            group: Vec::new(),
            shortcuts: Vec::new(),
            focused_idx: None,
            pending_shortcut: None,
        }
    }

    /// Register a button already owned by `frame`.
    ///
    /// # Panics
    ///
    /// Panics for a missing/non-button widget or a duplicate ID. A focus
    /// registration that points nowhere is a construction error, not an
    /// inactive button.
    fn add_button(&mut self, frame: &crate::widget::FrameWnd, widget_id: crate::widget::WidgetId) {
        assert!(
            matches!(
                frame.widget(widget_id),
                Some(crate::widget::Widget::Button(_))
            ),
            "focus button {widget_id} is missing from its frame"
        );
        assert!(
            !self.group.iter().any(|entry| entry.widget_id == widget_id),
            "focus button {widget_id} is already registered"
        );
        self.group.push(FrameButtonEntry { widget_id });
    }

    /// Bind a physical key to a registered button.
    ///
    /// Shortcut activation follows the original focus manager's two-edge
    /// behavior: key-down focuses/selects and key-up emits
    /// [`crate::ui::UiMsg::WidgetActivated`].
    fn add_shortcut(&mut self, widget_id: crate::widget::WidgetId, key: KeyCode) {
        assert!(
            self.group.iter().any(|entry| entry.widget_id == widget_id),
            "shortcut target {widget_id} is not registered"
        );
        self.shortcuts.retain(|(bound, _)| *bound != key);
        self.shortcuts.push((key, widget_id));
    }

    #[cfg(test)]
    fn focused_button(&self) -> Option<crate::widget::WidgetId> {
        self.focused_idx.map(|idx| self.group[idx].widget_id)
    }

    /// Append keyboard focus events to the widget events produced by the
    /// frame for this input pass.
    fn process_input(
        &mut self,
        frame: &mut crate::widget::FrameWnd,
        mut events: Vec<crate::ui::UiEvent>,
        keyboard: &crate::ui::UiKeyboard,
        mouse_captured: bool,
    ) -> Vec<crate::ui::UiEvent> {
        if mouse_captured || !keyboard.has_changed() {
            return events;
        }

        let mut focus_events = self.process_navigation(frame, keyboard);
        if focus_events.is_empty() {
            focus_events = self.process_shortcuts(frame, keyboard);
        }
        if let Some(origin) = focus_events.first().map(|event| event.origin_widget_id) {
            events.retain(|event| event.origin_widget_id != origin);
        }
        events.extend(focus_events);
        events
    }

    fn process_navigation(
        &mut self,
        frame: &mut crate::widget::FrameWnd,
        keyboard: &crate::ui::UiKeyboard,
    ) -> Vec<crate::ui::UiEvent> {
        // The dialog buttons are shortcut-only: arrow keys never move focus,
        // but a repeating arrow still pre-empts Enter handling this pass.
        if key_repeats(keyboard, KeyCode::ArrowLeft) || key_repeats(keyboard, KeyCode::ArrowRight) {
            return Vec::new();
        }
        if keyboard.get_state_of_key(KeyCode::Enter) == WidgetKeyState::KeyDown
            && keyboard.get_typewriter_state(KeyCode::Enter) == TypeWriter::None
            && let Some(idx) = self.focused_idx
        {
            return button_mut(frame, self.group[idx].widget_id).set_group_selected(true);
        }
        if key_released(keyboard, KeyCode::Enter)
            && let Some(idx) = self.focused_idx
        {
            return self.activate_focused(frame, idx);
        }
        Vec::new()
    }

    fn process_shortcuts(
        &mut self,
        frame: &mut crate::widget::FrameWnd,
        keyboard: &crate::ui::UiKeyboard,
    ) -> Vec<crate::ui::UiEvent> {
        for &(key, widget_id) in &self.shortcuts {
            if keyboard.get_state_of_key(key) == WidgetKeyState::KeyDown
                && keyboard.get_typewriter_state(key) == TypeWriter::None
                && self.focused_idx.is_none()
            {
                self.pending_shortcut = Some(key);
                let mut events = self.focus_button(frame, widget_id);
                events.extend(button_mut(frame, widget_id).set_group_selected(true));
                return events;
            }
        }

        if let Some(key) = self.pending_shortcut
            && key_released(keyboard, key)
        {
            self.pending_shortcut = None;
            let widget_id = self
                .shortcuts
                .iter()
                .find_map(|&(bound, id)| (bound == key).then_some(id))
                .expect("pending shortcut lost its registered button");
            let idx = self
                .focused_idx
                .filter(|&idx| self.group[idx].widget_id == widget_id)
                .expect("pending shortcut lost focus before key release");
            return self.activate_focused(frame, idx);
        }

        Vec::new()
    }

    fn focus_button(
        &mut self,
        frame: &mut crate::widget::FrameWnd,
        widget_id: crate::widget::WidgetId,
    ) -> Vec<crate::ui::UiEvent> {
        let mut events = self.clear_focus(frame);
        let idx = self
            .group
            .iter()
            .position(|entry| entry.widget_id == widget_id)
            .expect("focus target is not registered");
        self.focused_idx = Some(idx);
        let target = button_mut(frame, widget_id);
        target.hide_focus(false);
        events.extend(target.set_group_focused(true));
        events
    }

    fn clear_focus(&mut self, frame: &mut crate::widget::FrameWnd) -> Vec<crate::ui::UiEvent> {
        let Some(idx) = self.focused_idx.take() else {
            return Vec::new();
        };
        let target = button_mut(frame, self.group[idx].widget_id);
        let mut events = target.set_group_focused(false);
        events.extend(target.set_group_selected(false));
        events
    }

    fn activate_focused(
        &mut self,
        frame: &mut crate::widget::FrameWnd,
        idx: usize,
    ) -> Vec<crate::ui::UiEvent> {
        let widget_id = self.group[idx].widget_id;
        self.pending_shortcut = None;
        let mut events = self.clear_focus(frame);
        events.extend(button_mut(frame, widget_id).activate());
        events
    }
}

fn key_repeats(keyboard: &crate::ui::UiKeyboard, key: KeyCode) -> bool {
    keyboard.get_state_of_key(key) == WidgetKeyState::KeyDown
        && matches!(
            keyboard.get_typewriter_state(key),
            TypeWriter::None | TypeWriter::Repeat
        )
}

fn key_released(keyboard: &crate::ui::UiKeyboard, key: KeyCode) -> bool {
    matches!(
        keyboard.get_state_of_key(key),
        WidgetKeyState::KeyPressed | WidgetKeyState::KeyDouble
    ) && keyboard.has_key_changed(key)
}

fn button_mut(
    frame: &mut crate::widget::FrameWnd,
    widget_id: crate::widget::WidgetId,
) -> &mut crate::widget::WidgetButton {
    match frame.widget_mut(widget_id) {
        Some(crate::widget::Widget::Button(button)) => button,
        Some(_) => panic!("focus target {widget_id} is not a button"),
        None => panic!("focus target {widget_id} is missing from its frame"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx_types::Keycode;
    use crate::ingame_menu::layout::MENU_W;

    fn modal_state() -> YesNoModalState {
        let frame = widget_bridge::make_button_frame(&[
            (ID_YES, "Yes", 0, 0, 80, 30),
            (ID_NO, "No", 100, 0, 80, 30),
        ]);
        let mut focus = FrameButtonFocusManager::new();
        configure_focus(&mut focus, &frame);

        let input_state = ModalInputState::new();

        YesNoModalState {
            message: "Continue?".to_string(),
            frame,
            input_state,
            tooltip: TooltipState::new(),
            transform: MenuTransform::centered(640, 480),
            win_x: 0,
            win_y: 0,
            focus,
            choice: None,
            pressed_frame_pending: false,
            fly_in: None,
        }
    }

    fn key_event(keycode: Keycode, physical_key: KeyCode, down: bool) -> GameEvent {
        if down {
            GameEvent::KeyDown {
                keycode,
                physical_key: Some(physical_key),
            }
        } else {
            GameEvent::KeyUp {
                keycode,
                physical_key: Some(physical_key),
            }
        }
    }

    fn send_key(state: &mut YesNoModalState, keycode: Keycode, physical: KeyCode, down: bool) {
        let event = key_event(keycode, physical, down);
        state.input_state.update_from_event(&event, state.transform);
        state.process_widget_input();
    }

    #[test]
    fn split_event_batches_refresh_pointer_and_render_transforms() {
        let mut state = modal_state();
        for (width, height) in [(640, 480), (1280, 720), (800, 600), (480, 360)] {
            let transform = MenuTransform::centered(width, height);
            let (x, y) = transform.to_screen(400, 300);
            assert_eq!(
                state.handle_events(
                    &[GameEvent::MouseMove {
                        x,
                        y,
                        xrel: 0,
                        yrel: 0
                    }],
                    transform
                ),
                None
            );
            assert_eq!(
                (state.input_state.virt_x, state.input_state.virt_y),
                (400.0, 300.0)
            );
            assert_eq!(state.transform.to_screen(400, 300), (x, y));
        }
        state.resolve(YesNoChoice::No);
        let transform = MenuTransform::centered(1920, 1080);
        assert_eq!(state.handle_events(&[], transform), Some(false));
        assert_eq!(state.transform.to_screen(0, 0), transform.to_screen(0, 0));
    }

    #[test]
    fn yesno_hint_names_both_shortcuts() {
        assert!(SHORTCUT_HINT.contains("Enter"), "{SHORTCUT_HINT}");
        assert!(SHORTCUT_HINT.contains("Yes"), "{SHORTCUT_HINT}");
        assert!(SHORTCUT_HINT.contains("Escape"), "{SHORTCUT_HINT}");
        assert!(SHORTCUT_HINT.contains("No"), "{SHORTCUT_HINT}");
    }

    #[test]
    fn activated_seal_renders_pressed_before_the_dialog_closes() {
        let mut state = modal_state();
        send_key(&mut state, Keycode::Return, KeyCode::Enter, true);
        send_key(&mut state, Keycode::Return, KeyCode::Enter, false);

        // The choice is recorded, but the modal holds for one frame so the
        // pressed sprite is actually drawn before the host closes it.
        assert_eq!(state.choice, Some(YesNoChoice::Yes));
        assert!(state.pressed_frame_pending);
        let yes = state.frame.widget(ID_YES).expect("yes seal");
        assert_eq!(yes.base().state, crate::ui::UiState::Pushed);
    }

    #[test]
    fn return_focuses_then_confirms_on_release() {
        let mut state = modal_state();
        send_key(&mut state, Keycode::Return, KeyCode::Enter, true);
        assert_eq!(state.choice, None);
        assert_eq!(state.focus.focused_button(), Some(ID_YES));

        send_key(&mut state, Keycode::Return, KeyCode::Enter, false);
        assert_eq!(state.choice, Some(YesNoChoice::Yes));
    }

    #[test]
    fn escape_focuses_then_cancels_on_release() {
        let mut state = modal_state();
        send_key(&mut state, Keycode::Escape, KeyCode::Escape, true);
        assert_eq!(state.choice, None);
        assert_eq!(state.focus.focused_button(), Some(ID_NO));

        send_key(&mut state, Keycode::Escape, KeyCode::Escape, false);
        assert_eq!(state.choice, Some(YesNoChoice::No));
    }

    #[test]
    fn space_and_enter_confirm_while_escape_cancels() {
        for (keycode, physical, expected, expected_focus) in [
            (Keycode::Space, KeyCode::Space, YesNoChoice::Yes, ID_YES),
            (Keycode::Return, KeyCode::Enter, YesNoChoice::Yes, ID_YES),
            (Keycode::Escape, KeyCode::Escape, YesNoChoice::No, ID_NO),
        ] {
            let mut state = modal_state();
            send_key(&mut state, keycode, physical, true);
            assert_eq!(state.choice, None, "{keycode:?} must resolve on release");
            assert_eq!(state.focus.focused_button(), Some(expected_focus));

            send_key(&mut state, keycode, physical, false);
            assert_eq!(state.choice, Some(expected), "{keycode:?}");
            assert!(state.pressed_frame_pending, "{keycode:?}");
            let seal = state.frame.widget(expected_focus).expect("focused seal");
            assert_eq!(seal.base().state, crate::ui::UiState::Pushed);
        }
    }

    #[test]
    fn original_non_navigable_group_ignores_arrows() {
        let mut state = modal_state();
        send_key(&mut state, Keycode::Right, KeyCode::ArrowRight, true);
        assert_eq!(state.focus.focused_button(), None);
        assert_eq!(state.choice, None);
    }

    #[test]
    fn red_decline_never_repeats_the_blue_accept_in_one_event_batch() {
        let mut state = modal_state();
        state.apply_widget_events(&[
            UiEvent {
                msg_type: UiMsg::WidgetActivated,
                origin_widget_id: ID_YES,
                data: None,
            },
            UiEvent {
                msg_type: UiMsg::WidgetActivated,
                origin_widget_id: ID_NO,
                data: None,
            },
        ]);
        assert_eq!(
            state.choice,
            Some(YesNoChoice::No),
            "a simultaneous red activation must decline, not repeat accept"
        );
    }

    #[test]
    fn resolution_is_reentrant_and_first_choice_wins() {
        let mut state = modal_state();
        let accept = UiEvent {
            msg_type: UiMsg::WidgetActivated,
            origin_widget_id: ID_YES,
            data: None,
        };
        let decline = UiEvent {
            msg_type: UiMsg::WidgetActivated,
            origin_widget_id: ID_NO,
            data: None,
        };
        state.apply_widget_events(&[accept]);
        state.apply_widget_events(&[decline]);
        state.resolve(YesNoChoice::No);
        assert_eq!(state.choice, Some(YesNoChoice::Yes));
    }

    fn mission_start_modal_state() -> YesNoModalState {
        let mut state = modal_state();
        state.fly_in = Some(FlyInTransition::new());
        state
    }

    /// Click a seal at its *visible* screen position. During a fly-in the
    /// panel is displaced, so each frame's screen point is computed with the
    /// offset that transform will have after `handle_events` advances it.
    fn click_visible_seal(state: &mut YesNoModalState, virt_x: f32, virt_y: f32) {
        let base = MenuTransform::centered(640, 480);
        for down in [true, false] {
            let offset = match state.fly_in {
                Some(f) => fly_in_offset_x((f.elapsed_ticks + 1.0) / FLY_IN_TICKS),
                None => 0,
            };
            let visible = MenuTransform {
                origin_x: base.origin_x + offset,
                origin_y: base.origin_y,
            };
            let (sx, sy) = visible.to_screen(virt_x as i32, virt_y as i32);
            let event = if down {
                GameEvent::MouseDown(sx, sy, 1, 0)
            } else {
                GameEvent::MouseUp(sx, sy, 1)
            };
            let _ = state.handle_events(&[event], base);
        }
    }

    #[test]
    fn clicking_each_seal_resolves_the_dialog() {
        let mut yes = modal_state();
        click_visible_seal(&mut yes, 40.0, 15.0);
        assert_eq!(yes.choice, Some(YesNoChoice::Yes));

        let mut no = modal_state();
        click_visible_seal(&mut no, 140.0, 15.0);
        assert_eq!(no.choice, Some(YesNoChoice::No));
    }

    #[test]
    fn a_click_on_the_flying_seal_resolves_the_dialog() {
        let transform = MenuTransform::centered(640, 480);
        let mut state = mission_start_modal_state();
        for _ in 0..3 {
            state.handle_events(&[], transform);
        }
        assert!(state.fly_in_offset_x() > 0, "seal must still be flying in");

        click_visible_seal(&mut state, 40.0, 15.0);
        assert_eq!(state.choice, Some(YesNoChoice::Yes));
    }

    #[test]
    fn mission_start_fly_in_starts_off_screen_right_and_lands_centered() {
        let final_x = (MENU_W - WIN_W) / 2;
        assert_eq!(
            fly_in_offset_x(0.0) + final_x,
            MENU_W,
            "at progress 0 the panel's left edge sits at the right edge of the menu"
        );
        assert_eq!(
            fly_in_offset_x(1.0),
            0,
            "at progress 1 the panel sits at its centred position"
        );
    }

    #[test]
    fn a_frame_advances_the_mission_start_fly_in_monotonically() {
        let transform = MenuTransform::centered(640, 480);
        let mut state = mission_start_modal_state();
        let mut previous = state.fly_in_offset_x();
        assert_eq!(previous, fly_in_offset_x(0.0));
        for _ in 0..FLY_IN_TICKS as usize {
            state.handle_events(&[], transform);
            let current = state.fly_in_offset_x();
            assert!(current < previous, "each frame must move the panel left");
            previous = current;
        }
        assert_eq!(
            previous, 0,
            "the flight ends exactly at the centred position"
        );
    }
}
