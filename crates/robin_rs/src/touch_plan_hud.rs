//! Sticky touch controls: automatic quick-action planning and the quick-group
//! modifier latch. Both are host-only HUD affordances; neither produces a
//! `PlayerCommand` on its own tap.

use crate::input_translator::TouchQuickGroupMode;
use crate::renderer::Renderer;

pub const WIDTH: i32 = 118;
pub const HEIGHT: i32 = 34;
pub const QUICK_GROUP_WIDTH: i32 = 118;
pub const QUICK_GROUP_HEIGHT: i32 = 34;
const CONTROL_GAP: i32 = 10;

pub const fn platform_has_touch_planning_hud() -> bool {
    cfg!(any(
        target_os = "android",
        target_os = "ios",
        target_arch = "wasm32"
    ))
}

pub fn rect(screen_width: u16) -> (i32, i32, i32, i32) {
    let right = i32::from(screen_width).saturating_sub(10);
    (right - WIDTH, 46, right, 46 + HEIGHT)
}

pub fn hit_test(screen_width: u16, x: i32, y: i32) -> bool {
    let (left, top, right, bottom) = rect(screen_width);
    (left..=right).contains(&x) && (top..=bottom).contains(&y)
}

/// The quick-group modifier sits immediately left of the planning control,
/// sharing its vertical band. Task 3 pads this rect with the shared
/// finger-sized helper; the geometry here stays the raw drawing box.
pub fn quick_group_rect(screen_width: u16) -> (i32, i32, i32, i32) {
    let (plan_left, top, _, bottom) = rect(screen_width);
    let right = plan_left - CONTROL_GAP;
    (right - QUICK_GROUP_WIDTH, top, right, bottom)
}

pub fn hit_test_quick_group(screen_width: u16, x: i32, y: i32) -> bool {
    let (left, top, right, bottom) = quick_group_rect(screen_width);
    (left..=right).contains(&x) && (top..=bottom).contains(&y)
}

pub fn quick_group_label(mode: TouchQuickGroupMode) -> &'static str {
    match mode {
        TouchQuickGroupMode::Off => "QG OFF",
        TouchQuickGroupMode::Assign => "QG ASSIGN",
        TouchQuickGroupMode::Recall => "QG RECALL",
    }
}

/// In the browser build the idle box reads as a permanent UI artifact; show
/// it only while a plan is latched. Native touch builds keep the idle affordance.
pub fn should_render(active: bool, is_browser: bool) -> bool {
    active || !is_browser
}

/// Same browser rule for the modifier control: an idle browser box would read
/// as a permanent artifact, so show it only while a mode is latched.
pub fn should_render_quick_group(active: bool, is_browser: bool) -> bool {
    active || !is_browser
}

/// Border colour carries mode and pressed state without changing any input.
pub fn quick_group_border(mode: TouchQuickGroupMode, pressed: bool) -> u16 {
    if pressed {
        Renderer::create_color_16(255, 255, 210)
    } else if mode.is_active() {
        Renderer::create_color_16(238, 192, 55)
    } else {
        Renderer::create_color_16(224, 211, 157)
    }
}

pub fn render(renderer: &mut Renderer, fonts: Option<&crate::hud_text::HudFonts>, active: bool) {
    if !platform_has_touch_planning_hud() || !should_render(active, cfg!(target_arch = "wasm32")) {
        return;
    }
    let (left, top, right, bottom) = rect(renderer.screen_width());
    let border = if active {
        Renderer::create_color_16(238, 192, 55)
    } else {
        Renderer::create_color_16(224, 211, 157)
    };
    renderer.draw_rect_outline_screen(left, top, right, bottom, border);
    renderer.draw_rect_outline_screen(left + 2, top + 2, right - 2, bottom - 2, border);
    if let Some(fonts) = fonts {
        let label = if active {
            "CANCEL PLAN"
        } else {
            "PLAN ACTIONS"
        };
        let width = fonts.tooltip_font.text_width(label);
        crate::ingame_menu::layout::render_text_screen_font(
            renderer,
            &fonts.tooltip_font,
            label,
            left + (WIDTH - width) / 2,
            top + (HEIGHT - fonts.tooltip_font.height() as i32) / 2,
        );
    }
}

/// Draw the quick-group modifier beside the planning control. Pressed state is
/// presentation-only: it comes from the same touch/left-button sample as every
/// other sprite HUD button and never enters command dispatch.
pub fn render_quick_group(
    renderer: &mut Renderer,
    fonts: Option<&crate::hud_text::HudFonts>,
    mode: TouchQuickGroupMode,
    pressed: bool,
) {
    if !platform_has_touch_planning_hud()
        || !should_render_quick_group(mode.is_active(), cfg!(target_arch = "wasm32"))
    {
        return;
    }
    let (left, top, right, bottom) = quick_group_rect(renderer.screen_width());
    let border = quick_group_border(mode, pressed);
    renderer.draw_rect_outline_screen(left, top, right, bottom, border);
    renderer.draw_rect_outline_screen(left + 2, top + 2, right - 2, bottom - 2, border);
    if let Some(fonts) = fonts {
        let label = quick_group_label(mode);
        let width = fonts.tooltip_font.text_width(label);
        crate::ingame_menu::layout::render_text_screen_font(
            renderer,
            &fonts.tooltip_font,
            label,
            left + (QUICK_GROUP_WIDTH - width) / 2,
            top + (QUICK_GROUP_HEIGHT - fonts.tooltip_font.height() as i32) / 2,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_hides_the_idle_plan_actions_box() {
        assert!(!should_render(false, true));
        assert!(should_render(true, true));
        assert!(should_render(false, false));
    }

    #[test]
    fn hit_box_tracks_right_edge_and_includes_boundary() {
        let (left, top, right, bottom) = rect(1024);
        assert!(hit_test(1024, left, top));
        assert!(hit_test(1024, right, bottom));
        assert!(!hit_test(1024, left - 1, top));
    }

    #[test]
    fn quick_group_hit_box_sits_beside_plan_without_overlap() {
        use crate::input_translator::TouchQuickGroupMode;

        let (plan_left, plan_top, _, plan_bottom) = rect(1024);
        let (left, top, right, bottom) = quick_group_rect(1024);
        assert_eq!(top, plan_top);
        assert_eq!(bottom, plan_bottom);
        assert!(
            right < plan_left,
            "quick-group control sits left of planning"
        );
        assert!(
            hit_test_quick_group(1024, left, top),
            "boundary-inclusive hit target"
        );
        assert!(hit_test_quick_group(1024, right, bottom));
        assert!(!hit_test_quick_group(1024, left - 1, top));
        assert!(!hit_test_quick_group(1024, right + 1, bottom));
        assert!(
            !hit_test_quick_group(1024, plan_left, plan_top + 1),
            "padded targets must not steal the neighbouring control"
        );

        // Both controls render on touch platforms; the browser hides them
        // until latched so idle boxes do not linger in screenshots.
        assert!(should_render_quick_group(false, false));
        assert!(!should_render_quick_group(false, true));
        assert!(should_render_quick_group(true, true));

        // The reported label names the active mode.
        assert_eq!(quick_group_label(TouchQuickGroupMode::Off), "QG OFF");
        assert_eq!(quick_group_label(TouchQuickGroupMode::Assign), "QG ASSIGN");
        assert_eq!(quick_group_label(TouchQuickGroupMode::Recall), "QG RECALL");
    }

    #[test]
    fn quick_group_border_shows_latch_and_pressed_state() {
        use crate::input_translator::TouchQuickGroupMode;

        let idle = quick_group_border(TouchQuickGroupMode::Off, false);
        let latched = quick_group_border(TouchQuickGroupMode::Assign, false);
        let pressed = quick_group_border(TouchQuickGroupMode::Assign, true);
        assert_ne!(idle, latched, "an armed mode must be visible");
        assert_ne!(latched, pressed, "a press must be visible");
        assert_eq!(
            quick_group_border(TouchQuickGroupMode::Recall, false),
            latched,
            "assign and recall share the armed border; the label differs"
        );
    }
}
