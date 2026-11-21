//! Host-only map/screen transform with an optional camera rotation.
//!
//! The terrain shader, sprites, decals, overlays and mouse picking must all
//! use this same transform. Rotation happens in screen pixels about the
//! playfield centre; map-space quantisation stays downstream of it.

use robin_engine::coordinates::{MapPoint, MapSize, ScreenPoint, ScreenSize};

use super::PANNEL_HEIGHT;

/// Angle and pivot shared by every world-to-screen mapping in one view.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CameraRotation {
    pub(crate) angle: f32,
    pub(crate) center: ScreenPoint,
}

impl CameraRotation {
    pub(crate) fn new(angle: f32, screen: ScreenSize) -> Self {
        Self {
            angle,
            center: ScreenPoint::new(screen.x * 0.5, (screen.y - PANNEL_HEIGHT) * 0.5),
        }
    }

    pub(crate) fn to_screen(self, point: MapPoint, view: MapPoint, zoom: f32) -> ScreenPoint {
        map_to_screen(point, view, zoom, self)
    }

    /// Undo the screen rotation about the playfield centre.
    pub(crate) fn screen_inverse(&self, point: ScreenPoint) -> ScreenPoint {
        // Angle zero must preserve the legacy arithmetic exactly: a rotate
        // followed by its inverse is not bit-identical in IEEE-754.
        if self.angle == 0.0 {
            return point;
        }
        rotate_screen_point(point, -self.angle, self.center)
    }
}

fn rotate(point: (f32, f32), angle: f32) -> (f32, f32) {
    let (sin, cos) = angle.sin_cos();
    (point.0 * cos - point.1 * sin, point.0 * sin + point.1 * cos)
}

pub(crate) fn rotate_screen_point(
    point: ScreenPoint,
    angle: f32,
    center: ScreenPoint,
) -> ScreenPoint {
    let (x, y) = rotate((point.x - center.x, point.y - center.y), angle);
    ScreenPoint::new(x + center.x, y + center.y)
}

pub(crate) fn map_to_screen(
    point: MapPoint,
    view: MapPoint,
    zoom: f32,
    rotation: CameraRotation,
) -> ScreenPoint {
    let pixel = ((point.x - view.x) * zoom, (point.y - view.y) * zoom);
    if rotation.angle == 0.0 {
        return ScreenPoint::new(pixel.0, pixel.1);
    }
    let (x, y) = rotate(
        (pixel.0 - rotation.center.x, pixel.1 - rotation.center.y),
        rotation.angle,
    );
    ScreenPoint::new(x + rotation.center.x, y + rotation.center.y)
}

pub(crate) fn screen_to_map(
    point: ScreenPoint,
    view: MapPoint,
    zoom: f32,
    rotation: CameraRotation,
) -> MapPoint {
    if rotation.angle == 0.0 {
        return MapPoint::new(view.x + point.x / zoom, view.y + point.y / zoom);
    }
    let (x, y) = rotate(
        (point.x - rotation.center.x, point.y - rotation.center.y),
        -rotation.angle,
    );
    MapPoint::new(
        view.x + (x + rotation.center.x) / zoom,
        view.y + (y + rotation.center.y) / zoom,
    )
}

/// Axis-aligned bound of a set of screen points, as `(min_x, min_y, max_x, max_y)`.
pub(crate) fn aabb_of_points(points: impl IntoIterator<Item = (f32, f32)>) -> (f32, f32, f32, f32) {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (x, y) in points {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    (min_x, min_y, max_x, max_y)
}

/// Size of the playfield's axis-aligned bound after rotation, in map units.
pub(crate) fn rotated_playfield_size(screen: ScreenSize, zoom: f32, angle: f32) -> (f32, f32) {
    let w = screen.x / zoom;
    let h = (screen.y - PANNEL_HEIGHT) / zoom;
    let (sin, cos) = angle.sin_cos();
    (w * cos.abs() + h * sin.abs(), w * sin.abs() + h * cos.abs())
}

/// Allowed range for the unrotated view origin so the rotated screen corners
/// all land inside the level.
pub(crate) fn rotated_view_bounds(
    screen: ScreenSize,
    level: MapSize,
    zoom: f32,
    angle: f32,
) -> (MapPoint, MapPoint) {
    let (rotated_w, rotated_h) = rotated_playfield_size(screen, zoom, angle);
    let centre_x = screen.x * 0.5 / zoom;
    let centre_y = (screen.y - PANNEL_HEIGHT) * 0.5 / zoom;
    (
        MapPoint::new(rotated_w * 0.5 - centre_x, rotated_h * 0.5 - centre_y),
        MapPoint::new(
            level.x - rotated_w * 0.5 - centre_x,
            level.y - rotated_h * 0.5 - centre_y,
        ),
    )
}

/// Map points sampled by the four playfield corners, for fill assertions.
#[cfg(test)]
pub(crate) fn screen_corners_to_map(
    view: MapPoint,
    screen: ScreenSize,
    zoom: f32,
    rotation: CameraRotation,
) -> [MapPoint; 4] {
    let corners = [
        (0.0, 0.0),
        (screen.x, 0.0),
        (0.0, screen.y - PANNEL_HEIGHT),
        (screen.x, screen.y - PANNEL_HEIGHT),
    ];
    corners.map(|(x, y)| screen_to_map(ScreenPoint::new(x, y), view, zoom, rotation))
}

/// Smallest zoom that keeps the rotated playfield inside the level bounds.
pub(crate) fn minimum_zoom(screen: ScreenSize, level: MapSize, angle: f32) -> f32 {
    assert!(
        level.x > 0.0 && level.y > 0.0,
        "rotation needs a sized level"
    );
    let (sin, cos) = angle.sin_cos();
    let need_x = (screen.x * cos.abs() + (screen.y - PANNEL_HEIGHT) * sin.abs()) / level.x;
    let need_y = (screen.x * sin.abs() + (screen.y - PANNEL_HEIGHT) * cos.abs()) / level.y;
    let needed = need_x.max(need_y);
    if needed > 2.0 {
        tracing::warn!(
            needed,
            "rotation needs zoom beyond the usual cap; view will widen"
        );
    }
    needed.max(0.5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_4, FRAC_PI_8};

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn rotation_round_trip_and_zero_parity() {
        let view = MapPoint::new(12.5, 40.0);
        let zoom = 1.5;
        let screen = ScreenSize::new(800.0, 680.0);
        let center = ScreenPoint::new(screen.x * 0.5, (screen.y - 80.0) * 0.5);
        let point = MapPoint::new(123.25, 267.75);
        for angle in [0.0, FRAC_PI_8, FRAC_PI_4, -FRAC_PI_8, -FRAC_PI_4] {
            let rotation = CameraRotation { angle, center };
            let screen_point = map_to_screen(point, view, zoom, rotation);
            let back = screen_to_map(screen_point, view, zoom, rotation);
            assert!(near(back.x, point.x), "x round-trip at {angle}: {back:?}");
            assert!(near(back.y, point.y), "y round-trip at {angle}: {back:?}");
            if angle == 0.0 {
                assert!(near(screen_point.x, (point.x - view.x) * zoom));
                assert!(near(screen_point.y, (point.y - view.y) * zoom));
            }
        }
    }

    #[test]
    fn angle_zero_is_bit_identical_to_the_legacy_transform() {
        let view = MapPoint::new(12.5, 40.0);
        let zoom = 1.5;
        let rotation = CameraRotation::new(0.0, ScreenSize::new(800.0, 680.0));
        for x in [0.0_f32, 0.1, 123.25, 8000.0] {
            for y in [0.0_f32, -0.1, 267.75, 8000.0] {
                let point = MapPoint::new(x, y);
                let screen = map_to_screen(point, view, zoom, rotation);
                assert_eq!(screen.x, (point.x - view.x) * zoom);
                assert_eq!(screen.y, (point.y - view.y) * zoom);
                let screen_point = ScreenPoint::new(x, y);
                let map = screen_to_map(screen_point, view, zoom, rotation);
                assert_eq!(map.x, view.x + screen_point.x / zoom);
                assert_eq!(map.y, view.y + screen_point.y / zoom);
                assert_eq!(rotation.screen_inverse(screen_point), screen_point);
            }
        }
    }

    #[test]
    fn positive_angle_rotates_screen_clockwise() {
        let rotation = CameraRotation {
            angle: FRAC_PI_4,
            center: ScreenPoint::new(400.0, 300.0),
        };
        let point = map_to_screen(MapPoint::new(500.0, 300.0), MapPoint::ZERO, 1.0, rotation);
        assert!(
            point.y > rotation.center.y,
            "point must move below centre: {point:?}"
        );
        assert!(near(point.x, 400.0 + 100.0 / 2.0_f32.sqrt()));
        assert!(near(point.y, 300.0 + 100.0 / 2.0_f32.sqrt()));
    }

    #[test]
    fn rotated_size_and_minimum_zoom_cover_playfield() {
        let screen = ScreenSize::new(800.0, 680.0);
        let level = MapSize::new(1000.0, 1000.0);
        let (w, h) = rotated_playfield_size(screen, 1.0, FRAC_PI_4);
        let expected = (800.0 + 600.0) / 2.0_f32.sqrt();
        assert!(near(w, expected), "rotated width {w} != {expected}");
        assert!(near(h, expected), "rotated height {h} != {expected}");
        let zoom = minimum_zoom(screen, level, FRAC_PI_4);
        assert!(zoom >= expected / 1000.0 - 1e-3 && zoom <= expected / 1000.0 + 1e-3);
        let (w, h) = rotated_playfield_size(screen, zoom, FRAC_PI_4);
        assert!(w <= level.x + 1e-3 && h <= level.y + 1e-3);
        assert_eq!(minimum_zoom(screen, level, 0.0), 0.8);
    }
}
