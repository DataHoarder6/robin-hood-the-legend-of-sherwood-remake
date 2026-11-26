//! Mission won / lost / quit transient popup.
//!
//! Creates a 629x480 frame window with the small menu background
//! inside, grows the panel in from the HUD's confirmation rail and shrinks it
//! back on dismissal, and on open advances a frame-driven confirmation prompt.
//! Reports `true` when the player confirms.

use crate::ingame_menu::widget_bridge::{ModalScreenIo, ScreenFrame};
use crate::renderer::Renderer;

use super::MissionEndKind;
use super::layout::{
    MENU_H, MENU_W, MenuRect, MenuTransform, TextAlign, VAlign, draw_background,
    render_clipped_text_in_box_font,
};
use super::resources::IngameMenuResources;
use super::yesno::YesNoModalState;

/// The panel travels the short distance from the right edge in nine ticks
/// (~150 ms at 60 Hz), rather than growing from wherever a button happened
/// to sit. Dismissal is quicker still: six ticks back to the same edge.
const OPEN_TICKS: f32 = 9.0;
const CLOSE_TICKS: f32 = 6.0;

fn final_x() -> i32 {
    (MENU_W - POPUP_W) / 2
}

/// Smallest uniform scale of the panel at the off-screen end of a flight.
const SMALL_SCALE: f32 = 0.6;

/// Virtual y of the Sherwood HUD's green/red confirmation buttons. The panel
/// flies toward this rail anchor instead of the right-edge vertical centre.
/// TODO: derive from the HUD layout when its rail anchor is exposed here.
const HUD_BUTTONS_Y: i32 = 105;

/// One frame of the confirmation's flight: top-left in virtual menu units
/// plus the uniform scale about that top-left.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ConfirmationPose {
    x: i32,
    y: i32,
    scale: f32,
}

impl ConfirmationPose {
    #[cfg(test)]
    fn into_parts(self) -> (i32, i32, f32) {
        (self.x, self.y, self.scale)
    }

    /// Map any virtual rect drawn on the panel — text box or clickable seal —
    /// through this pose. Drawing and pointer hit-testing must use the same
    /// mapping or the visible control and its hit box drift apart.
    fn scaled_rect(self, rect: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
        let (x, y, w, h) = rect;
        let sx = |v: i32| self.x + (v as f32 * self.scale).round() as i32;
        let sy = |v: i32| self.y + (v as f32 * self.scale).round() as i32;
        (
            sx(x),
            sy(y),
            (w as f32 * self.scale).round() as i32,
            (h as f32 * self.scale).round() as i32,
        )
    }
}

fn confirmation_pose(progress: f32, direction: TransitionDirection) -> ConfirmationPose {
    let t = progress.clamp(0.0, 1.0);
    let centre_y = (MENU_H - POPUP_H) / 2;
    match direction {
        TransitionDirection::Open => {
            let ease = 1.0 - (1.0 - t).powi(3);
            ConfirmationPose {
                x: MENU_W + ((final_x() - MENU_W) as f32 * ease).round() as i32,
                y: HUD_BUTTONS_Y + ((centre_y - HUD_BUTTONS_Y) as f32 * ease).round() as i32,
                scale: SMALL_SCALE + (1.0 - SMALL_SCALE) * ease,
            }
        }
        TransitionDirection::Close => {
            let ease = t.powi(2);
            ConfirmationPose {
                x: final_x() + ((MENU_W - final_x()) as f32 * ease).round() as i32,
                y: centre_y + ((HUD_BUTTONS_Y - centre_y) as f32 * ease).round() as i32,
                scale: 1.0 - (1.0 - SMALL_SCALE) * ease,
            }
        }
    }
}

fn fly_in_x(progress: f32) -> i32 {
    confirmation_pose(progress, TransitionDirection::Open).x
}

fn fly_out_x(progress: f32) -> i32 {
    confirmation_pose(progress, TransitionDirection::Close).x
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
    phase: MissionStatePopupPhase,
}

impl MissionStatePopupState {
    pub fn new(
        renderer: &Renderer,
        resources: &IngameMenuResources,
        message: String,
        end: MissionEndKind,
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
            phase,
        }
    }

    pub fn tick(&mut self, io: &mut ModalScreenIo<'_, '_>) -> Option<bool> {
        match &mut self.phase {
            MissionStatePopupPhase::Opening(transition) => {
                if transition.tick(io) {
                    self.phase =
                        MissionStatePopupPhase::Confirming(Box::new(YesNoModalState::new(
                            io.window,
                            io.renderer,
                            io.resources,
                            self.message.clone(),
                        )));
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

        let title = resources.menu_text.get(end.title_id());

        Self {
            transform,
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

        let pose = confirmation_pose(progress, self.direction);

        let renderer = &mut *io.renderer;
        let resources = io.resources;
        screen.begin_draw(renderer);

        let (tl_x, tl_y, w, h) = pose.scaled_rect((0, 0, POPUP_W, POPUP_H));
        if let Some(bg) = resources.menu_bg_small {
            draw_background(renderer, self.transform, &bg, tl_x, tl_y, w, h);
        }

        // Text uses the same scaled pose as the panel: entry grows from the
        // HUD rail toward the centre, exit shrinks back toward the rail.
        if let Some(font) = resources.title_font_any() {
            let tw = font.text_width(&self.title);
            let (tx, ty, _, _) = pose.scaled_rect(((POPUP_W - tw) / 2, 20, 0, 0));
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
            let (x, y, text_w, text_h) = pose.scaled_rect((TEXT_X, TEXT_Y, TEXT_W, TEXT_H));
            render_clipped_text_in_box_font(
                renderer,
                font,
                self.transform,
                &self.message,
                MenuRect {
                    x,
                    y,
                    w: text_w,
                    h: text_h,
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

    #[test]
    fn popup_grows_on_entry_and_shrinks_on_exit() {
        let entry: Vec<f32> = [0.0, 0.25, 0.5, 0.75, 1.0]
            .into_iter()
            .map(|p| confirmation_pose(p, TransitionDirection::Open).scale)
            .collect();
        assert!(entry.windows(2).all(|w| w[0] < w[1]), "{entry:?}");
        assert_eq!(entry[0], SMALL_SCALE);
        assert_eq!(entry[4], 1.0);

        let exit: Vec<f32> = [0.0, 0.25, 0.5, 0.75, 1.0]
            .into_iter()
            .map(|p| confirmation_pose(p, TransitionDirection::Close).scale)
            .collect();
        assert!(exit.windows(2).all(|w| w[0] > w[1]), "{exit:?}");
        assert_eq!(exit[0], 1.0);
        assert_eq!(exit[4], SMALL_SCALE);
    }

    #[test]
    fn popup_flight_ends_at_the_hud_buttons_not_the_edge_centre() {
        let centre_y = (MENU_H - POPUP_H) / 2;
        let (x, y, scale) = confirmation_pose(1.0, TransitionDirection::Close).into_parts();
        assert_eq!(x, MENU_W, "the panel still leaves through the right edge");
        assert_eq!(y, HUD_BUTTONS_Y);
        assert!(y < centre_y, "HUD buttons sit above the edge centre");
        assert_eq!(scale, SMALL_SCALE, "the panel shrinks as it leaves");
    }

    #[test]
    fn scaled_pose_moves_seal_hit_boxes_with_the_panel() {
        let open = confirmation_pose(0.5, TransitionDirection::Open);
        let settled = confirmation_pose(1.0, TransitionDirection::Open);
        let seal = (25, 130, 40, 20);
        let hit = open.scaled_rect(seal);
        let settled_hit = settled.scaled_rect(seal);
        assert!(
            hit.0 > settled_hit.0,
            "in-flight seal follows the panel right"
        );
        assert!(
            hit.2 < settled_hit.2,
            "in-flight seal is smaller than settled"
        );
    }
}
