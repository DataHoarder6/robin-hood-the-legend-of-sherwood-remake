//! Mission won / lost / quit transient popup.
//!
//! Creates a 629x480 frame window with the small menu background
//! inside, flies the panel in from the right edge, and on open advances a
//! frame-driven confirmation prompt. Reports `true` when the player confirms.

use crate::ingame_menu::widget_bridge::{ModalScreenIo, ScreenFrame};
use crate::renderer::Renderer;

use super::MissionEndKind;
use super::layout::{
    MENU_H, MENU_W, MenuRect, MenuTransform, TextAlign, VAlign, draw_background,
    render_clipped_text_in_box_font,
};
use super::resources::IngameMenuResources;
use super::yesno::{ConfirmButtons, YesNoModalState};

/// The panel travels the short distance from the right edge in nine ticks
/// (~150 ms at 60 Hz), rather than growing from wherever a button happened
/// to sit. Dismissal is quicker still: six ticks back to the same edge.
const OPEN_TICKS: f32 = 9.0;
const CLOSE_TICKS: f32 = 6.0;

fn final_x() -> i32 {
    (MENU_W - POPUP_W) / 2
}

fn fly_in_x(progress: f32) -> i32 {
    let t = 1.0 - (1.0 - progress.clamp(0.0, 1.0)).powi(3);
    MENU_W + ((final_x() - MENU_W) as f32 * t) as i32
}

fn fly_out_x(progress: f32) -> i32 {
    let t = progress.clamp(0.0, 1.0).powi(2);
    final_x() + ((MENU_W - final_x()) as f32 * t) as i32
}

/// Popup small-background dimensions.
const POPUP_W: i32 = 400;
const POPUP_H: i32 = 200;

/// Text region inside the popup.
const TEXT_X: i32 = 25;
const TEXT_Y: i32 = 50;
const TEXT_W: i32 = 350;
const TEXT_H: i32 = 70;

#[derive(Copy, Clone)]
enum TransitionDirection {
    Open,
    Close,
}

enum MissionStatePopupPhase {
    Opening(MissionStateTransition),
    Confirming(Box<YesNoModalState>),
    Closing(MissionStateTransition),
    Done,
}

/// One-frame state for the mission won/lost/aborted popup.
pub struct MissionStatePopupState {
    message: String,
    end: MissionEndKind,
    buttons: ConfirmButtons,
    phase: MissionStatePopupPhase,
}

impl MissionStatePopupState {
    pub fn new(
        renderer: &Renderer,
        resources: &IngameMenuResources,
        message: String,
        end: MissionEndKind,
        buttons: ConfirmButtons,
    ) -> Self {
        let phase = MissionStatePopupPhase::Opening(MissionStateTransition::new(
            renderer,
            resources,
            &message,
            end,
            TransitionDirection::Open,
        ));
        Self {
            message,
            end,
            buttons,
            phase,
        }
    }

    pub fn tick(&mut self, io: &mut ModalScreenIo<'_, '_>) -> Option<bool> {
        match &mut self.phase {
            MissionStatePopupPhase::Opening(transition) => {
                if transition.tick(io) {
                    self.phase = MissionStatePopupPhase::Confirming(Box::new(
                        YesNoModalState::new_with_buttons(
                            io.window,
                            io.renderer,
                            io.resources,
                            self.message.clone(),
                            self.buttons,
                        ),
                    ));
                }
                None
            }
            MissionStatePopupPhase::Confirming(yesno) => {
                if let Some(confirmed) = yesno.tick(io) {
                    if confirmed {
                        self.phase = MissionStatePopupPhase::Done;
                        Some(true)
                    } else {
                        self.phase = MissionStatePopupPhase::Closing(MissionStateTransition::new(
                            io.renderer,
                            io.resources,
                            &self.message,
                            self.end,
                            TransitionDirection::Close,
                        ));
                        None
                    }
                } else {
                    None
                }
            }
            MissionStatePopupPhase::Closing(transition) => {
                if transition.tick(io) {
                    self.phase = MissionStatePopupPhase::Done;
                    Some(false)
                } else {
                    None
                }
            }
            MissionStatePopupPhase::Done => Some(false),
        }
    }
}

struct MissionStateTransition {
    transform: MenuTransform,
    final_y: i32,
    elapsed_ticks: f32,
    title: String,
    message: String,
    direction: TransitionDirection,
}

impl MissionStateTransition {
    fn new(
        renderer: &Renderer,
        resources: &IngameMenuResources,
        message: &str,
        end: MissionEndKind,
        direction: TransitionDirection,
    ) -> Self {
        let transform = MenuTransform::for_renderer(renderer);
        let final_y = (MENU_H - POPUP_H) / 2;

        let title = resources.menu_text.get(end.title_id());

        Self {
            transform,
            final_y,
            elapsed_ticks: 0.0,
            title,
            message: message.to_string(),
            direction,
        }
    }

    fn tick(&mut self, io: &mut ModalScreenIo<'_, '_>) -> bool {
        let duration = match self.direction {
            TransitionDirection::Open => OPEN_TICKS,
            TransitionDirection::Close => CLOSE_TICKS,
        };
        self.elapsed_ticks += 1.0;
        let progress = self.elapsed_ticks / duration;
        if progress >= 1.0 {
            return true;
        }

        // Drain pending input so the OS doesn't think the window is hung, and
        // keep the transition centred if the presentation aspect changes.
        let screen = ScreenFrame::poll(io);
        self.transform = screen.transform;

        let (tl_x, tl_y) = match self.direction {
            TransitionDirection::Open => (fly_in_x(progress), self.final_y),
            TransitionDirection::Close => (fly_out_x(progress), self.final_y),
        };

        let renderer = &mut *io.renderer;
        let resources = io.resources;
        screen.begin_draw(renderer);

        if let Some(bg) = resources.menu_bg_small {
            draw_background(renderer, self.transform, &bg, tl_x, tl_y, POPUP_W, POPUP_H);
        }

        // The panel keeps its full size throughout the flight: the motion is
        // a translation from the right edge, never a scale from the centre.
        if let Some(font) = resources.title_font_any() {
            let tw = font.text_width(&self.title);
            let tx = tl_x + (POPUP_W - tw) / 2;
            let ty = tl_y + 20;
            super::layout::render_text_virt_font(
                renderer,
                font,
                self.transform,
                &self.title,
                tx,
                ty,
            );
        }
        if let Some(font) = resources.popup_font_any() {
            render_clipped_text_in_box_font(
                renderer,
                font,
                self.transform,
                &self.message,
                MenuRect {
                    x: tl_x + TEXT_X,
                    y: tl_y + TEXT_Y,
                    w: TEXT_W,
                    h: TEXT_H,
                },
                TextAlign::Center,
                VAlign::Top,
            );
        }

        // The transition has no modal input state, so the cursor follows the
        // raw window position instead of `ScreenFrame::finish`'s input cursor.
        if let Some(c) = io.cursor {
            let (mx, my) = io.window.cursor_pos();
            c.cursor
                .render(io.renderer, mx as f32, my as f32, c.opacity, c.shadow_color);
        }

        io.renderer.present();
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_flies_in_from_the_right_edge_and_eases_out() {
        let final_x = (MENU_W - POPUP_W) / 2;
        assert_eq!(fly_in_x(0.0), MENU_W);
        assert_eq!(fly_in_x(1.0), final_x);
        let quarter = fly_in_x(0.25) as f32;
        let linear_quarter = MENU_W as f32 + (final_x - MENU_W) as f32 * 0.25;
        assert!(
            quarter < linear_quarter,
            "ease-out must cover more of the distance than a linear pan"
        );
    }

    #[test]
    fn popup_flies_back_to_the_right_edge_on_dismiss() {
        let final_x = (MENU_W - POPUP_W) / 2;
        assert_eq!(fly_out_x(0.0), final_x);
        assert_eq!(fly_out_x(1.0), MENU_W);
        assert!(fly_out_x(0.5) < (final_x + MENU_W) / 2);
    }
}
