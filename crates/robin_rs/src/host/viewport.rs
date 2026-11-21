//! Client-local camera and touch motion.
use super::*;
use std::f32::consts::{FRAC_PI_4, FRAC_PI_8};

/// Host-local viewport state. This is deliberately outside
/// `robin_engine`: screen size, mouse anchoring, render culling, and
/// local scroll/zoom are presentation concerns and may differ on every
/// multiplayer peer.
#[derive(Debug, Clone)]
pub struct ViewportState {
    pub view_position: MapPoint,
    pub old_view_position: MapPoint,
    pub zoom_factor: f32,
    pub old_zoom_factor: f32,
    pub screen_size: ScreenSize,
    pub level_size: MapSize,
    touch_motion: TouchCameraMotion,
    /// In-flight scripted pan: the director owns the target and its timing,
    /// the host owns the path from the view the player was actually looking at.
    director_slide: Option<DirectorSlide>,
    /// Presentation state captured the frame a cutscene takes the camera.
    cutscene_view: Option<(MapPoint, f32)>,
    /// Whether ordinary selection may pan the view to reveal an off-screen
    /// PC (XAG 117). Enabled by default; players can switch it off.
    ensure_selection_on_screen: bool,
    /// Requested angle, in radians. Host-only: no engine state depends on it.
    camera_angle: f32,
    /// Rendered angle, eased towards `camera_angle` between frames.
    display_angle: f32,
    saved_camera_angle: Option<f32>,
    last_rotation_ms: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
struct DirectorSlide {
    target: MapPoint,
    engine_from: MapPoint,
    host_from: MapPoint,
}

/// Fraction of the director's pan completed, read from its deterministic
/// cursor. The host uses it to travel the same distance over the same ticks.
fn director_slide_progress(slide: &DirectorSlide, current: MapPoint) -> f32 {
    let dx = slide.target.x - slide.engine_from.x;
    let dy = slide.target.y - slide.engine_from.y;
    let (span, moved) = if dx.abs() >= dy.abs() {
        (dx, current.x - slide.engine_from.x)
    } else {
        (dy, current.y - slide.engine_from.y)
    };
    if span.abs() < f32::EPSILON {
        1.0
    } else {
        (moved / span).clamp(0.0, 1.0)
    }
}

/// Host-only touch-camera state. Velocities are expressed in screen pixels
/// per second so momentum feels consistent at every zoom level.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct TouchCameraMotion {
    transform_active: bool,
    velocity_x: f32,
    velocity_y: f32,
    last_inertia_ms: u32,
}

impl ViewportState {
    pub fn new(screen_width: f32, screen_height: f32) -> Self {
        Self {
            view_position: MapPoint::ZERO,
            old_view_position: MapPoint::ZERO,
            zoom_factor: 1.0,
            old_zoom_factor: 1.0,
            screen_size: ScreenSize::new(screen_width, screen_height),
            level_size: MapSize::ZERO,
            touch_motion: TouchCameraMotion::default(),
            director_slide: None,
            cutscene_view: None,
            ensure_selection_on_screen: true,
            camera_angle: 0.0,
            display_angle: 0.0,
            saved_camera_angle: None,
            last_rotation_ms: None,
        }
    }

    pub fn set_screen_size(&mut self, width: f32, height: f32) {
        self.screen_size = ScreenSize::new(width, height);
        self.clip_view();
    }

    pub fn set_level_size(&mut self, width: f32, height: f32) {
        self.level_size = MapSize::new(width, height);
        self.clip_view();
    }

    /// Requested camera angle, in radians.
    pub fn camera_angle(&self) -> f32 {
        self.camera_angle
    }

    /// Angle currently used by every renderer transform.
    pub fn display_angle(&self) -> f32 {
        self.display_angle
    }

    /// Angle quantised to the sprite's 22.5° view frames.
    ///
    /// Sprite rows snap to the requested angle; only terrain/screen geometry
    /// eases between frames.
    pub fn camera_steps(&self) -> i8 {
        (self.camera_angle / FRAC_PI_8).round() as i8
    }

    pub fn playfield_center(&self) -> ScreenPoint {
        ScreenPoint::new(
            self.screen_size.x * 0.5,
            (self.screen_size.y - PANNEL_HEIGHT) * 0.5,
        )
    }

    pub(crate) fn camera_rotation(&self) -> CameraRotation {
        CameraRotation::new(self.display_angle, self.screen_size)
    }

    /// Rotate by whole sprite-view steps, clamped to ±45°.
    pub fn rotate_by(&mut self, steps: i8) {
        let next = (self.camera_angle + f32::from(steps) * FRAC_PI_8).clamp(-FRAC_PI_4, FRAC_PI_4);
        if (next - self.camera_angle).abs() < f32::EPSILON {
            return;
        }
        self.camera_angle = next;
        self.apply_rotation_constraints();
    }

    pub fn reset_rotation(&mut self) {
        self.camera_angle = 0.0;
        self.display_angle = 0.0;
        self.saved_camera_angle = None;
        self.last_rotation_ms = None;
        self.apply_rotation_constraints();
    }

    /// Ease the rendered angle towards the requested one.
    pub fn advance_rotation(&mut self, now_ms: u32) {
        let dt = self
            .last_rotation_ms
            .map_or(0, |last| now_ms.saturating_sub(last));
        self.last_rotation_ms = Some(now_ms);
        let factor = 1.0 - (-(dt as f32) / 50.0).exp();
        self.display_angle += (self.camera_angle - self.display_angle) * factor;
        if (self.camera_angle - self.display_angle).abs() < 0.001 {
            self.display_angle = self.camera_angle;
        }
        self.clip_view();
    }

    fn apply_rotation_constraints(&mut self) {
        if self.level_size.x > 0.0 && self.level_size.y > 0.0 {
            let needed = minimum_zoom(self.screen_size, self.level_size, self.camera_angle);
            if needed > self.zoom_factor {
                self.zoom_factor = needed;
            }
        }
        self.clip_view();
    }

    pub fn center_on_point(&mut self, point: MapPoint) {
        self.view_position = MapPoint::new(
            (point.x - self.screen_size.x / (2.0 * self.zoom_factor)).floor(),
            (point.y - self.screen_size.y / (2.0 * self.zoom_factor)).floor(),
        );
        self.clip_view();
    }

    /// Pan just enough to bring `point` inside the visible view, leaving the
    /// camera untouched when it is already on screen.
    ///
    /// This is the selection-driven motion allowed by XAG 117: never a
    /// hard-centre, always disable-able, and a no-op for an on-screen
    /// target.  The bottom panel is excluded from the visible height to
    /// match [`Self::clip_view`].
    pub fn ensure_on_screen(&mut self, point: MapPoint) {
        if !self.ensure_selection_on_screen {
            return;
        }
        let view_w = self.screen_size.x / self.zoom_factor;
        let view_h = (self.screen_size.y - PANNEL_HEIGHT) / self.zoom_factor;
        let right = self.view_position.x + view_w;
        let bottom = self.view_position.y + view_h;

        let mut x = self.view_position.x;
        if point.x < x {
            x = point.x;
        } else if point.x > right {
            x = point.x - view_w;
        }
        let mut y = self.view_position.y;
        if point.y < y {
            y = point.y;
        } else if point.y > bottom {
            y = point.y - view_h;
        }
        if x != self.view_position.x || y != self.view_position.y {
            self.view_position = MapPoint::new(x, y);
            self.clip_view();
        }
    }

    /// Enable or disable selection-driven ensure-on-screen panning.
    pub fn set_ensure_selection_on_screen(&mut self, enabled: bool) {
        self.ensure_selection_on_screen = enabled;
    }

    /// Apply scripted motion from the local view. Merely locking input must
    /// not restore the stale shared view left by an earlier cutscene.
    pub fn advance_director_camera(
        &mut self,
        before: engine_api::DirectorCameraFrame,
        after: engine_api::DirectorCameraFrame,
        view_size: ScreenSize,
    ) {
        if !before.owns_view && !after.owns_view {
            return;
        }
        if !before.owns_view && after.owns_view {
            self.cutscene_view = Some((self.view_position, self.zoom_factor));
        }
        if after.owns_view {
            self.saved_camera_angle.get_or_insert(self.camera_angle);
            self.camera_angle = 0.0;
            self.display_angle = 0.0;
        } else if before.owns_view {
            let saved = self.saved_camera_angle.take();
            if let Some(saved) = saved {
                self.camera_angle = saved;
                self.display_angle = saved;
                self.apply_rotation_constraints();
            }
        }
        if before.zoom_factor != after.zoom_factor {
            // Script zooms retain the local focal point until a pan or jump
            // explicitly chooses a new one.
            self.zoom_by(after.zoom_factor / self.zoom_factor, None);
        }

        let target = after.slide_target.or_else(|| {
            before
                .slide_target
                .filter(|target| *target == after.view_position)
        });
        let mut jumped = false;
        if let Some(target) = target {
            // A new target rebases the pan onto the view the player has now;
            // later segments chain off the previous segment's end pose.
            if self
                .director_slide
                .is_none_or(|slide| slide.target != target)
            {
                self.director_slide = Some(DirectorSlide {
                    target,
                    engine_from: after.view_position,
                    host_from: self.view_position,
                });
            }
            let slide = self.director_slide.unwrap();
            let progress = director_slide_progress(&slide, after.view_position);
            let shift_x = (view_size.x - self.screen_size.x) / (2.0 * after.zoom_factor);
            let shift_y = (view_size.y - self.screen_size.y) / (2.0 * after.zoom_factor);
            self.old_view_position = self.view_position;
            self.view_position = MapPoint::new(
                slide.host_from.x + (target.x + shift_x - slide.host_from.x) * progress,
                slide.host_from.y + (target.y + shift_y - slide.host_from.y) * progress,
            );
        } else {
            // No pan: an explicit jump or a follow update adopts the framing.
            jumped = before.zoom_factor == after.zoom_factor
                && self
                    .director_slide
                    .is_none_or(|slide| after.view_position != slide.target)
                && after.view_position != before.view_position;
            self.director_slide = None;
            if jumped {
                self.adopt_director_camera(after.view_position, view_size, after.zoom_factor);
            }
        }

        if before.owns_view && !after.owns_view {
            self.director_slide = None;
            if jumped {
                // An authored jump back is intent; it wins over the restore.
                self.cutscene_view = None;
            } else if let Some((view, zoom)) = self.cutscene_view.take() {
                // Teleport, not a glide: the presentation interpolator reads
                // `old_view_position`, so both endpoints must be the restore.
                self.old_view_position = view;
                self.old_zoom_factor = zoom;
                self.view_position = view;
                self.zoom_factor = zoom;
            }
        }
        self.apply_rotation_constraints();
    }

    /// Mirror the shared script/director camera for an explicit placement.
    ///
    /// The director camera is deterministic shared state, so the engine
    /// frames its focal point inside a fixed virtual view (`view_size`)
    /// rather than any peer's canvas. Shift the top-left by half the size
    /// difference so that same focal point lands in the centre of this
    /// host's (possibly widescreen) view. The shift is exactly zero when
    /// both sizes agree, which keeps the classic 1024x768 path bit-identical.
    pub fn adopt_director_camera(
        &mut self,
        view_position: MapPoint,
        view_size: ScreenSize,
        zoom_factor: f32,
    ) {
        assert!(
            zoom_factor.is_finite() && zoom_factor > 0.0,
            "director camera supplied invalid zoom factor {zoom_factor}"
        );
        self.saved_camera_angle.get_or_insert(self.camera_angle);
        self.camera_angle = 0.0;
        self.display_angle = 0.0;
        self.old_view_position = self.view_position;
        self.old_zoom_factor = self.zoom_factor;
        self.zoom_factor = zoom_factor;
        let shift_x = (view_size.x - self.screen_size.x) / (2.0 * zoom_factor);
        let shift_y = (view_size.y - self.screen_size.y) / (2.0 * zoom_factor);
        self.view_position = MapPoint::new(view_position.x + shift_x, view_position.y + shift_y);
        self.clip_view();
    }

    pub fn sound_listen_point(&self) -> MapPoint {
        MapPoint::new(
            self.view_position.x + self.screen_size.x * 0.5 / self.zoom_factor,
            self.view_position.y + (self.screen_size.y - PANNEL_HEIGHT) * 0.5 / self.zoom_factor,
        )
    }

    pub fn scroll_by(&mut self, delta: ScreenVec) {
        self.old_view_position = self.view_position;
        self.view_position.x += delta.x / self.zoom_factor;
        self.view_position.y += delta.y / self.zoom_factor;
        self.clip_view();
    }

    /// Place a host-local follow camera so a map point remains at a chosen
    /// screen anchor. This is presentation state and must not feed the
    /// deterministic director camera.
    pub fn follow_map_point(&mut self, point: MapPoint, anchor: ScreenPoint) {
        self.old_view_position = self.view_position;
        self.view_position = MapPoint::new(
            point.x - anchor.x / self.zoom_factor,
            point.y - anchor.y / self.zoom_factor,
        );
        self.clip_view();
    }

    pub fn zoom_by(&mut self, factor: f32, mouse_screen: Option<ScreenPoint>) {
        let next = (self.zoom_factor * factor).clamp(0.5, 2.0);
        if (next - self.zoom_factor).abs() < f32::EPSILON {
            return;
        }
        let anchor = mouse_screen.unwrap_or_else(|| {
            ScreenPoint::new(self.screen_size.x * 0.5, self.screen_size.y * 0.5)
        });
        let before = self.screen_to_map_unchecked(anchor);
        self.old_zoom_factor = self.zoom_factor;
        self.zoom_factor = next;
        self.view_position = MapPoint::new(
            before.x - anchor.x / self.zoom_factor,
            before.y - anchor.y / self.zoom_factor,
        );
        self.clip_view();
    }

    /// Begin a two-finger camera transform after the gameplay layer has
    /// decided whether both fingers originated in the world viewport.
    pub fn begin_touch_transform(&mut self, accepted: bool) {
        self.touch_motion = TouchCameraMotion {
            transform_active: accepted,
            ..TouchCameraMotion::default()
        };
    }

    /// Atomically apply centroid translation and pinch scaling. The map point
    /// beneath the previous centroid remains beneath the current centroid,
    /// avoiding the order-dependent wobble caused by separate pan/zoom calls.
    pub fn apply_touch_transform(
        &mut self,
        centroid: ScreenPoint,
        pan: ScreenVec,
        scale: f32,
    ) -> bool {
        if !self.touch_motion.transform_active {
            return false;
        }
        if !scale.is_finite()
            || scale <= 0.0
            || !centroid.x.is_finite()
            || !centroid.y.is_finite()
            || !pan.x.is_finite()
            || !pan.y.is_finite()
        {
            tracing::warn!(?centroid, ?pan, scale, "ignored non-finite touch transform");
            return false;
        }

        let previous_centroid = ScreenPoint::new(centroid.x - pan.x, centroid.y - pan.y);
        let anchor = self.screen_to_map_unchecked(previous_centroid);
        self.old_view_position = self.view_position;
        self.old_zoom_factor = self.zoom_factor;
        self.zoom_factor = (self.zoom_factor * scale).clamp(0.5, 2.0);
        self.view_position = MapPoint::new(
            anchor.x - centroid.x / self.zoom_factor,
            anchor.y - centroid.y / self.zoom_factor,
        );
        self.clip_view();
        true
    }

    pub fn end_touch_transform(&mut self, velocity: ScreenVec, cancelled: bool, now_ms: u32) {
        const MAX_INERTIA_SPEED: f32 = 5_000.0;

        if !self.touch_motion.transform_active {
            self.touch_motion = TouchCameraMotion::default();
            return;
        }
        self.touch_motion.transform_active = false;
        if cancelled || !velocity.x.is_finite() || !velocity.y.is_finite() {
            self.touch_motion.velocity_x = 0.0;
            self.touch_motion.velocity_y = 0.0;
        } else {
            self.touch_motion.velocity_x = velocity.x;
            self.touch_motion.velocity_y = velocity.y;
            let speed = velocity.x.hypot(velocity.y);
            if speed > MAX_INERTIA_SPEED {
                let scale = MAX_INERTIA_SPEED / speed;
                self.touch_motion.velocity_x *= scale;
                self.touch_motion.velocity_y *= scale;
            }
        }
        self.touch_motion.last_inertia_ms = now_ms;
    }

    pub fn cancel_touch_motion(&mut self) {
        self.touch_motion = TouchCameraMotion::default();
    }

    /// Advance hard-clamped pan inertia using wall time. Returns whether the
    /// camera moved, allowing future display-rate render loops to skip static
    /// recomposition without coupling momentum to the 25 Hz simulation.
    pub fn advance_touch_inertia(&mut self, now_ms: u32) -> bool {
        const DECAY_PER_SECOND: f32 = 6.5;
        const STOP_SPEED: f32 = 18.0;
        const MAX_STEP_SECONDS: f32 = 0.050;

        if self.touch_motion.transform_active {
            self.touch_motion.last_inertia_ms = now_ms;
            return false;
        }
        let speed = self
            .touch_motion
            .velocity_x
            .hypot(self.touch_motion.velocity_y);
        if speed < STOP_SPEED {
            self.touch_motion.velocity_x = 0.0;
            self.touch_motion.velocity_y = 0.0;
            self.touch_motion.last_inertia_ms = now_ms;
            return false;
        }
        let elapsed = now_ms.wrapping_sub(self.touch_motion.last_inertia_ms) as f32 / 1000.0;
        let dt = elapsed.min(MAX_STEP_SECONDS);
        self.touch_motion.last_inertia_ms = now_ms;
        if dt <= 0.0 {
            return false;
        }

        let before = self.view_position;
        self.scroll_by(ScreenVec::new(
            -self.touch_motion.velocity_x * dt,
            -self.touch_motion.velocity_y * dt,
        ));
        if (self.view_position.x - before.x).abs() < f32::EPSILON {
            self.touch_motion.velocity_x = 0.0;
        }
        if (self.view_position.y - before.y).abs() < f32::EPSILON {
            self.touch_motion.velocity_y = 0.0;
        }
        let decay = (-DECAY_PER_SECOND * dt).exp();
        self.touch_motion.velocity_x *= decay;
        self.touch_motion.velocity_y *= decay;
        self.view_position != before
    }

    pub fn screen_to_map(&self, screen_pt: ScreenPoint) -> Option<MapPoint> {
        let map_pt = self.screen_to_map_unchecked(screen_pt);
        if map_pt.x > 0.0
            && map_pt.y > 0.0
            && map_pt.x <= self.level_size.x
            && map_pt.y <= self.level_size.y
        {
            Some(map_pt)
        } else {
            None
        }
    }

    pub fn screen_to_map_unchecked(&self, screen_pt: ScreenPoint) -> MapPoint {
        screen_to_map(
            screen_pt,
            self.view_position,
            self.zoom_factor,
            self.camera_rotation(),
        )
    }

    pub fn map_to_screen(&self, map_pt: MapPoint) -> Option<ScreenPoint> {
        let screen_pt = self.map_to_screen_unclamped(map_pt);
        if screen_pt.x >= 0.0
            && screen_pt.y >= 0.0
            && screen_pt.x <= self.screen_size.x
            && screen_pt.y <= self.screen_size.y
        {
            Some(screen_pt)
        } else {
            None
        }
    }

    pub fn map_to_screen_unclamped(&self, map_pt: MapPoint) -> ScreenPoint {
        map_to_screen(
            map_pt,
            self.view_position,
            self.zoom_factor,
            self.camera_rotation(),
        )
    }

    fn clip_view(&mut self) {
        if self.view_position.x < 0.0 {
            self.view_position.x = 0.0;
        }
        if self.view_position.y < 0.0 {
            self.view_position.y = 0.0;
        }
        if self.level_size.x > 0.0 && self.level_size.y > 0.0 {
            // Clamp the rotated AABB, not the unrotated playfield: its centre
            // sits at view + playfield_centre / zoom. Cover both the target and
            // eased angles so no corner goes dark mid-transition.
            let (lower_target, upper_target) = rotated_view_bounds(
                self.screen_size,
                self.level_size,
                self.zoom_factor,
                self.camera_angle,
            );
            let (lower_display, upper_display) = rotated_view_bounds(
                self.screen_size,
                self.level_size,
                self.zoom_factor,
                self.display_angle,
            );
            let min_x = lower_target.x.max(lower_display.x).max(0.0);
            let min_y = lower_target.y.max(lower_display.y).max(0.0);
            let max_x = upper_target.x.min(upper_display.x).max(min_x);
            let max_y = upper_target.y.min(upper_display.y).max(min_y);
            self.view_position.x = self.view_position.x.max(min_x).min(max_x);
            self.view_position.y = self.view_position.y.max(min_y).min(max_y);
        }
    }
}

impl Default for ViewportState {
    fn default() -> Self {
        Self::new(1024.0, 768.0)
    }
}

#[cfg(test)]
mod viewport_touch_tests {
    use super::*;

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.001,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
<<<<<<< HEAD
    fn ensure_on_screen_leaves_the_view_unchanged_when_the_target_is_visible() {
        let mut viewport = ViewportState::new(800.0, 600.0);
        viewport.set_level_size(2000.0, 2000.0);
        viewport.center_on_point(MapPoint::new(1000.0, 1000.0));
        let before = viewport.view_position;

        viewport.ensure_on_screen(MapPoint::new(1000.0, 1000.0));
        assert_eq!(viewport.view_position, before);
        viewport.ensure_on_screen(MapPoint::new(1100.0, 950.0));
        assert_eq!(viewport.view_position, before);
    }

    #[test]
    fn ensure_on_screen_pans_minimally_for_an_off_screen_target() {
        let mut viewport = ViewportState::new(800.0, 600.0);
        viewport.set_level_size(2000.0, 2000.0);
        viewport.center_on_point(MapPoint::new(1000.0, 1000.0));
        let before = viewport.view_position;

        viewport.ensure_on_screen(MapPoint::new(before.x - 10.0, before.y + 10.0));
        close(viewport.view_position.x, before.x - 10.0);
        close(viewport.view_position.y, before.y);
    }

    #[test]
    fn ensure_on_screen_can_be_disabled() {
        let mut viewport = ViewportState::new(800.0, 600.0);
        viewport.set_level_size(2000.0, 2000.0);
        viewport.center_on_point(MapPoint::new(1000.0, 1000.0));
        let before = viewport.view_position;
        viewport.set_ensure_selection_on_screen(false);

        viewport.ensure_on_screen(MapPoint::new(before.x - 100.0, before.y));
        assert_eq!(viewport.view_position, before);
    }

    #[test]
    fn rotated_view_corners_stay_inside_level() {
        let mut viewport = ViewportState::new(800.0, 680.0);
        viewport.set_level_size(1000.0, 1000.0);
        for steps in [-2, -1, 1, 2] {
            viewport.reset_rotation();
            viewport.rotate_by(steps);
            viewport.advance_rotation(0);
            viewport.advance_rotation(u32::MAX);
            let corners = screen_corners_to_map(
                viewport.view_position,
                viewport.screen_size,
                viewport.zoom_factor,
                viewport.camera_rotation(),
            );
            for corner in corners {
                assert!(
                    corner.x >= -0.01 && corner.x <= 1000.01,
                    "corner x {:.2} outside level at steps {steps}",
                    corner.x
                );
                assert!(
                    corner.y >= -0.01 && corner.y <= 1000.01,
                    "corner y {:.2} outside level at steps {steps}",
                    corner.y
                );
            }
        }
    }

    #[test]
    fn rotate_by_steps_and_director_restore() {
        use std::f32::consts::FRAC_PI_4;
        use std::f32::consts::FRAC_PI_8;
        let mut viewport = ViewportState::new(800.0, 680.0);
        viewport.set_level_size(1000.0, 1000.0);
        viewport.rotate_by(1);
        close(viewport.camera_angle(), FRAC_PI_8);
        assert_eq!(viewport.camera_steps(), 1);
        viewport.rotate_by(1);
        close(viewport.camera_angle(), FRAC_PI_4);
        viewport.rotate_by(1);
        close(viewport.camera_angle(), FRAC_PI_4);
        viewport.rotate_by(-3);
        close(viewport.camera_angle(), -FRAC_PI_8);
        viewport.reset_rotation();
        close(viewport.camera_angle(), 0.0);

        viewport.rotate_by(1);
        let idle = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(100.0, 100.0),
            zoom_factor: 1.0,
            slide_target: None,
            owns_view: false,
        };
        let owned = engine_api::DirectorCameraFrame {
            owns_view: true,
            ..idle
        };
        viewport.advance_director_camera(idle, owned, viewport.screen_size);
        close(viewport.camera_angle(), 0.0);
        viewport.advance_director_camera(owned, idle, viewport.screen_size);
        close(viewport.camera_angle(), FRAC_PI_8);
    }

    #[test]
    fn zoom_anchor_holds_at_rotated_angle() {
        let mut viewport = ViewportState::new(800.0, 680.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.rotate_by(1);
        let anchor = ScreenPoint::new(600.0, 200.0);
        let before = viewport.screen_to_map_unchecked(anchor);
        viewport.zoom_by(1.5, Some(anchor));
        let after = viewport.screen_to_map_unchecked(anchor);
        close(after.x, before.x);
        close(after.y, before.y);
    }

    #[test]
    fn combined_touch_transform_preserves_anchor() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(500.0, 400.0);
        let previous_centroid = ScreenPoint::new(300.0, 250.0);
        let anchor = viewport.screen_to_map_unchecked(previous_centroid);

        viewport.begin_touch_transform(true);
        viewport.apply_touch_transform(
            ScreenPoint::new(340.0, 270.0),
            ScreenVec::new(40.0, 20.0),
            1.5,
        );

        close(viewport.zoom_factor, 1.5);
        let transformed_anchor = viewport.screen_to_map_unchecked(ScreenPoint::new(340.0, 270.0));
        close(transformed_anchor.x, anchor.x);
        close(transformed_anchor.y, anchor.y);
    }

    #[test]
    fn director_pan_starts_at_local_view_and_finishes_at_script_target() {
        let mut viewport = ViewportState::new(1280.0, 720.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(2200.0, 1800.0);
        let view_size = ScreenSize::new(1024.0, 768.0);
        let idle = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(100.0, 200.0),
            zoom_factor: 1.0,
            slide_target: None,
            owns_view: false,
        };

        // LockUser and a timer before CameraGoto must leave the local view
        // where the player put it, even though the director is far away.
        let locked = engine_api::DirectorCameraFrame {
            owns_view: true,
            ..idle
        };
        viewport.advance_director_camera(idle, locked, view_size);
        assert_eq!(viewport.view_position, MapPoint::new(2200.0, 1800.0));
        let start = engine_api::DirectorCameraFrame {
            slide_target: Some(MapPoint::new(1100.0, 1200.0)),
            ..locked
        };
        viewport.advance_director_camera(locked, start, view_size);
        assert_eq!(viewport.view_position, MapPoint::new(2200.0, 1800.0));

        let halfway = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(600.0, 700.0),
            ..start
        };
        viewport.advance_director_camera(start, halfway, view_size);
        // The pan's origin is the host's own view, not the director cursor the
        // script left at the mission spawn. Halfway through the engine's pan,
        // the host is halfway from where the player was looking to the target.
        close(viewport.view_position.x, 1586.0);
        close(viewport.view_position.y, 1512.0);

        // Completion can clear the slide in the same tick that it arrives.
        let end = engine_api::DirectorCameraFrame {
            view_position: start.slide_target.unwrap(),
            slide_target: None,
            ..start
        };
        viewport.advance_director_camera(halfway, end, view_size);
        assert_eq!(viewport.view_position, MapPoint::new(972.0, 1224.0));
        viewport.advance_director_camera(end, end, view_size);
        assert_eq!(viewport.view_position, MapPoint::new(972.0, 1224.0));
    }

    #[test]
    fn cutscene_release_teleports_back_to_the_view_before_the_cutscene() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(2200.0, 1800.0);
        let view_size = viewport.screen_size;
        let spawn = MapPoint::new(100.0, 200.0);
        let room = MapPoint::new(1100.0, 1200.0);
        let idle = engine_api::DirectorCameraFrame {
            view_position: spawn,
            zoom_factor: 1.0,
            slide_target: None,
            owns_view: false,
        };
        let locked = engine_api::DirectorCameraFrame {
            owns_view: true,
            ..idle
        };
        viewport.advance_director_camera(idle, locked, view_size);

        let panning = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(600.0, 700.0),
            slide_target: Some(room),
            ..locked
        };
        viewport.advance_director_camera(locked, panning, view_size);
        let arrived = engine_api::DirectorCameraFrame {
            view_position: room,
            slide_target: None,
            ..locked
        };
        viewport.advance_director_camera(panning, arrived, view_size);
        assert_eq!(viewport.view_position, room);

        // The script unlocks without an explicit jump back: the camera must
        // return to where the player was before the cutscene, instantly.
        let released = engine_api::DirectorCameraFrame {
            owns_view: false,
            ..arrived
        };
        viewport.advance_director_camera(arrived, released, view_size);
        assert_eq!(viewport.view_position, MapPoint::new(2200.0, 1800.0));
    }

    #[test]
    fn cutscene_restore_brings_back_the_zoom_from_before_the_cutscene() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(2200.0, 1800.0);
        viewport.zoom_factor = 0.5;
        let view_size = viewport.screen_size;
        let idle = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(100.0, 200.0),
            zoom_factor: 0.5,
            slide_target: None,
            owns_view: false,
        };
        let locked = engine_api::DirectorCameraFrame {
            owns_view: true,
            ..idle
        };
        viewport.advance_director_camera(idle, locked, view_size);
        let zoomed = engine_api::DirectorCameraFrame {
            zoom_factor: 2.0,
            ..locked
        };
        viewport.advance_director_camera(locked, zoomed, view_size);
        let released = engine_api::DirectorCameraFrame {
            owns_view: false,
            ..zoomed
        };
        viewport.advance_director_camera(zoomed, released, view_size);
        close(viewport.zoom_factor, 0.5);
        assert_eq!(viewport.view_position, MapPoint::new(2200.0, 1800.0));
    }

    #[test]
    fn director_jump_interrupts_pan_and_adopts_destination() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(2200.0, 1800.0);
        let before = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(100.0, 200.0),
            zoom_factor: 1.0,
            slide_target: Some(MapPoint::new(1100.0, 1200.0)),
            owns_view: true,
        };
        let after = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(500.0, 600.0),
            slide_target: None,
            owns_view: false,
            ..before
        };
        viewport.advance_director_camera(before, after, viewport.screen_size);
        assert_eq!(viewport.view_position, after.view_position);
    }

    #[test]
    fn director_zoom_keeps_local_focal_point() {
        let mut viewport = ViewportState::new(1280.0, 720.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(2200.0, 1800.0);
        let center = ScreenPoint::new(640.0, 360.0);
        let focal_point = viewport.screen_to_map_unchecked(center);
        let before = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(100.0, 200.0),
            zoom_factor: 1.0,
            slide_target: None,
            owns_view: true,
        };
        let after = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(356.0, 392.0),
            zoom_factor: 2.0,
            ..before
        };
        viewport.advance_director_camera(before, after, ScreenSize::new(1024.0, 768.0));
        assert_eq!(viewport.zoom_factor, 2.0);
        assert_eq!(viewport.screen_to_map_unchecked(center), focal_point);
    }

    #[test]
    fn director_camera_is_identity_on_matching_canvas() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.adopt_director_camera(
            MapPoint::new(1234.5678, 987.6543),
            ScreenSize::new(1024.0, 768.0),
            0.5,
        );
        assert_eq!(viewport.view_position, MapPoint::new(1234.5678, 987.6543));
        assert_eq!(viewport.zoom_factor, 0.5);
    }

    #[test]
    fn director_camera_recentres_focal_point_on_widescreen_canvas() {
        // The engine framed focal point (1512, 1384) as top-left (1000, 1000)
        // in its 1024x768 virtual view. A 1280x720 host must show that same
        // point in the middle of its own canvas.
        let mut viewport = ViewportState::new(1280.0, 720.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.adopt_director_camera(
            MapPoint::new(1000.0, 1000.0),
            ScreenSize::new(1024.0, 768.0),
            1.0,
        );
        assert_eq!(viewport.view_position, MapPoint::new(872.0, 1024.0));
        let centre = viewport.screen_to_map_unchecked(ScreenPoint::new(640.0, 360.0));
        assert_eq!(centre, MapPoint::new(1512.0, 1384.0));

        // Zoomed out, the shift is measured in map pixels, so it doubles.
        viewport.adopt_director_camera(
            MapPoint::new(1000.0, 1000.0),
            ScreenSize::new(1024.0, 768.0),
            0.5,
        );
        assert_eq!(viewport.view_position, MapPoint::new(744.0, 1048.0));
    }

    #[test]
    fn rejected_touch_transform_does_not_move_camera() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(500.0, 400.0);
        viewport.begin_touch_transform(false);
        viewport.apply_touch_transform(
            ScreenPoint::new(400.0, 300.0),
            ScreenVec::new(50.0, 20.0),
            1.2,
        );
        assert_eq!(viewport.view_position, MapPoint::new(500.0, 400.0));
        assert_eq!(viewport.zoom_factor, 1.0);
    }

    #[test]
    fn touch_zoom_and_inertia_hard_clamp_to_map() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(1200.0, 900.0);
        viewport.begin_touch_transform(true);
        viewport.apply_touch_transform(
            ScreenPoint::new(0.0, 0.0),
            ScreenVec::new(1000.0, 1000.0),
            0.01,
        );
        assert_eq!(viewport.zoom_factor, 0.5);
        assert_eq!(viewport.view_position, MapPoint::ZERO);

        viewport.end_touch_transform(ScreenVec::new(2000.0, 1200.0), false, 100);
        assert!(!viewport.advance_touch_inertia(116));
        assert_eq!(viewport.view_position, MapPoint::ZERO);
        assert!(!viewport.advance_touch_inertia(132));
    }

    #[test]
    fn cancelling_transform_disables_momentum() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(1000.0, 1000.0);
        viewport.begin_touch_transform(true);
        viewport.end_touch_transform(ScreenVec::new(1000.0, 0.0), true, 100);
        assert!(!viewport.advance_touch_inertia(150));
        assert_eq!(viewport.view_position, MapPoint::new(1000.0, 1000.0));
    }

    #[test]
    fn touch_inertia_clamps_implausible_release_velocity() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(1000.0, 1000.0);
        viewport.begin_touch_transform(true);
        viewport.end_touch_transform(ScreenVec::new(1_000_000.0, 0.0), false, 100);

        assert!(viewport.advance_touch_inertia(116));
        close(viewport.view_position.x, 920.0);
        close(viewport.view_position.y, 1000.0);
    }
}
