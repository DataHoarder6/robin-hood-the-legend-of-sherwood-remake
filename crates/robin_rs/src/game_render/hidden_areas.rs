//! Dotted outlines of the ground hidden behind opaque building volumes.
//!
//! Level data stores no building graphic footprint, so each zone is derived
//! from the same `SightObstacle` footprint/top projection the shadow-polygon
//! solver uses.  Nothing in this pass is latched: zones, feet positions and
//! the held modifier are recomputed every frame, so the lines disappear on
//! the frame the modifier is released or the last character leaves a zone.

use std::collections::BTreeSet;

use geo::{Area, BooleanOps, ConvexHull, Coord, Intersects, MultiPoint, MultiPolygon, Point};
use robin_engine::coordinates::{GroundPoint, MapPoint};
use robin_engine::sight_obstacle::{ObstacleList, SightObstacle};
use winit::keyboard::KeyCode;

use crate::host::HostDraw;
use crate::key_config::HIDDEN_AREAS_MODIFIER_KEYS;
use crate::renderer::Renderer;
use robin_engine::engine as engine_api;
use robin_engine::engine::{LevelAssets, PresentationView};

use super::debug::for_each_polygon_edge;

/// Dot spacing / thickness in map units, matched to the patrol-route overlay.
const DOT_SPACING: f32 = 9.0;
const DOT_THICKNESS: f32 = 1.5;
const ZONE_COLOR: u32 = 0x7F_DF;

/// Draw dotted outlines for the zones revealed this frame.
///
/// Every input is read fresh from the frame's presentation inputs: the held
/// modifier comes from the keyboard state, occupancy from live entity feet,
/// and the override from the per-screenshot `DevState` clone. No state is
/// persisted, so releasing the modifier or leaving a zone removes the lines
/// on that same frame.
pub(crate) fn render_hidden_area_outlines(
    host: &HostDraw<'_>,
    engine: &PresentationView<'_>,
    assets: &LevelAssets,
    dev: &engine_api::DevState,
    keys: &BTreeSet<KeyCode>,
    renderer: &mut Renderer,
) {
    let reveal_all = reveal_all_requested(keys, dev.debug.hidden_areas);
    let obstacles = engine.sight_obstacles(assets);
    let zones = derive_hidden_zones(&obstacles);
    if zones.is_empty() {
        return;
    }

    let feet: Vec<GroundPoint> = engine
        .entities_iter()
        .filter(|entity| entity.is_active())
        .map(|entity| {
            let position = entity.element_data().position();
            GroundPoint::new(position.x, position.y)
        })
        .collect();
    let shown = visible_zone_indices(&zones, &feet, reveal_all);
    if shown.is_empty() {
        return;
    }
    let draw_manager = host.draw_manager();
    let color = draw_manager.pack_color(ZONE_COLOR);
    for index in shown {
        let mut phase = 0.0;
        let boundary = zones[index].boundary();
        for_each_polygon_edge(&boundary, |a, b| {
            draw_manager.draw_dotted_line(
                renderer,
                a,
                b,
                &mut phase,
                DOT_SPACING,
                DOT_THICKNESS,
                color,
            );
        });
    }
}

/// One occluded ground region, derived from a single opaque obstacle.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HiddenZone {
    polygon: geo::Polygon<f32>,
}

impl HiddenZone {
    /// Exterior boundary in map coordinates (ground points project to
    /// themselves because their Z is zero).
    fn boundary(&self) -> Vec<MapPoint> {
        self.polygon
            .exterior()
            .0
            .iter()
            .map(|c| MapPoint::new(c.x, c.y))
            .collect()
    }

    fn contains_feet(&self, feet: GroundPoint) -> bool {
        self.polygon.intersects(&Point::new(feet.x, feet.y))
    }
}

/// Derive every occluded ground zone from the active opaque obstacles.
pub(crate) fn derive_hidden_zones(obstacles: &ObstacleList<'_>) -> Vec<HiddenZone> {
    let mut zones = Vec::new();
    for (index, obstacle) in obstacles.iter_indexed() {
        if !obstacles.is_active(index as usize) {
            continue;
        }
        if let Some(zone) = zone_for_obstacle(obstacle) {
            zones.push(zone);
        }
    }
    zones
}

/// The ground hidden by one obstacle is the sweep of its footprint to its
/// projected top surface, minus the footprint itself.  A ground point projects
/// to map coordinates `(x, y)` while the top of an obstacle projects to
/// `(x, y - z_top)`, so the convex hull of both sets is exactly the map-space
/// silhouette of the volume.  No building footprint value is invented here.
fn zone_for_obstacle(obstacle: &SightObstacle) -> Option<HiddenZone> {
    if obstacle.is_projection_area() || !obstacle.is_opaque() {
        return None;
    }
    let points = &obstacle.obstacle_points;
    if points.len() < 3 {
        return None;
    }
    let footprint = obstacle.polygon.as_geo();
    if footprint.exterior().0.len() < 3 || footprint.unsigned_area() < MIN_ZONE_AREA {
        return None;
    }

    let mut hull_points: Vec<Coord<f32>> = footprint.exterior().0.clone();
    hull_points.extend(points.iter().flat_map(|point| {
        let top = Coord {
            x: point.x,
            y: point.y - point.z_top,
        };
        let bottom = Coord {
            x: point.x,
            y: point.y
                - if obstacle.on_ground {
                    0.0
                } else {
                    point.z_bottom
                },
        };
        [top, bottom]
    }));
    let silhouette =
        MultiPoint::new(hull_points.into_iter().map(Point::from).collect()).convex_hull();
    if silhouette.unsigned_area() < MIN_ZONE_AREA {
        return None;
    }

    let difference = MultiPolygon::new(vec![silhouette]).difference(footprint);
    let mut best: Option<geo::Polygon<f32>> = None;
    for candidate in difference.0 {
        let area = candidate.unsigned_area();
        if !area.is_finite() || area < MIN_ZONE_AREA || candidate.exterior().0.len() < 3 {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|current| area > current.unsigned_area())
        {
            best = Some(candidate);
        }
    }
    best.map(|polygon| HiddenZone { polygon })
}

/// Smallest footprint-difference area worth outlining (square map units).
const MIN_ZONE_AREA: f32 = 1.0;

/// Whether the whole-map reveal is requested this frame. `keys` is the
/// live physical key set; `debug_hidden_areas` is the screenshot override.
pub(crate) fn reveal_all_requested(keys: &BTreeSet<KeyCode>, debug_hidden_areas: bool) -> bool {
    debug_hidden_areas
        || HIDDEN_AREAS_MODIFIER_KEYS
            .iter()
            .any(|key| keys.contains(key))
}

/// Indices of zones to outline: all when revealing, otherwise only zones
/// containing at least one character's feet.
pub(crate) fn visible_zone_indices(
    zones: &[HiddenZone],
    feet: &[GroundPoint],
    reveal_all: bool,
) -> Vec<usize> {
    if reveal_all {
        return (0..zones.len()).collect();
    }
    (0..zones.len())
        .filter(|&index| feet.iter().any(|&foot| zones[index].contains_feet(foot)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::sight_obstacle::{ObstaclePoint, SIGHTOBSTACLE_OPAQUE, SIGHTOBSTACLE_SOLID};

    fn prism(id: u32, min_x: f32, min_y: f32, size: f32, z_top: f32, flags: u32) -> SightObstacle {
        let corners = [
            (min_x, min_y),
            (min_x + size, min_y),
            (min_x + size, min_y + size),
            (min_x, min_y + size),
        ];
        let mut obstacle = SightObstacle::new(id, flags);
        obstacle.obstacle_points = corners
            .iter()
            .map(|&(x, y)| ObstaclePoint {
                x,
                y,
                z_top,
                z_bottom: 0.0,
            })
            .collect();
        obstacle.rebuild_geometry();
        obstacle
    }

    fn building(id: u32, min_x: f32, min_y: f32, z_top: f32) -> SightObstacle {
        prism(
            id,
            min_x,
            min_y,
            10.0,
            z_top,
            SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_OPAQUE,
        )
    }

    fn area(zone: &HiddenZone) -> f32 {
        zone.polygon.unsigned_area()
    }

    fn keys(codes: &[KeyCode]) -> BTreeSet<KeyCode> {
        codes.iter().copied().collect()
    }

    #[test]
    fn zone_is_the_footprint_to_top_projection_sweep_minus_the_footprint() {
        let obstacles = [building(1, 0.0, 0.0, 5.0)];
        let list = ObstacleList::from_slice_all_active(&obstacles);
        let zones = derive_hidden_zones(&list);
        assert_eq!(zones.len(), 1, "one opaque building yields one zone");
        assert!(
            (area(&zones[0]) - 50.0).abs() < 0.01,
            "10x10 footprint projected 5 units north hides 50 map units of ground: {}",
            area(&zones[0])
        );
        assert!(zones[0].contains_feet(GroundPoint::new(5.0, -2.0)));
        assert!(!zones[0].contains_feet(GroundPoint::new(5.0, 5.0)));
    }

    #[test]
    fn zone_derivation_uses_only_obstacle_data_and_active_opaque_volumes() {
        let flat = building(1, 0.0, 0.0, 0.0);
        let transparent = prism(2, 20.0, 0.0, 10.0, 5.0, SIGHTOBSTACLE_SOLID);
        let mut projection_area = building(3, 40.0, 0.0, 5.0);
        projection_area.obstacle_type |=
            robin_engine::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA;
        let obstacles = [flat, transparent, projection_area];
        let list = ObstacleList {
            static_obstacles: &obstacles,
            dynamic_obstacles: &[],
            static_active: &[true, true, false],
        };
        assert!(
            derive_hidden_zones(&list).is_empty(),
            "flat, transparent and inactive obstacles must not invent a zone"
        );
    }

    #[test]
    fn reveal_all_is_superset_of_character_occupancy() {
        let obstacles = [building(1, 0.0, 0.0, 5.0), building(2, 40.0, 0.0, 5.0)];
        let list = ObstacleList::from_slice_all_active(&obstacles);
        let zones = derive_hidden_zones(&list);
        assert_eq!(zones.len(), 2);

        assert_eq!(
            visible_zone_indices(&zones, &[], false),
            Vec::<usize>::new()
        );
        assert_eq!(visible_zone_indices(&zones, &[], true), vec![0, 1]);
        assert_eq!(
            visible_zone_indices(&zones, &[GroundPoint::new(5.0, -2.0)], false),
            vec![0]
        );
        assert_eq!(
            visible_zone_indices(
                &zones,
                &[GroundPoint::new(45.0, -2.0), GroundPoint::new(5.0, -2.0)],
                false,
            ),
            vec![0, 1]
        );
        assert_eq!(
            visible_zone_indices(&zones, &[GroundPoint::new(5.0, 5.0)], false),
            Vec::<usize>::new(),
            "feet on the footprint are not feet in the hidden zone"
        );
    }

    #[test]
    fn held_modifier_and_debug_override_are_frame_local() {
        assert!(!reveal_all_requested(&keys(&[]), false));
        assert!(reveal_all_requested(&keys(&[KeyCode::SuperLeft]), false));
        assert!(reveal_all_requested(&keys(&[KeyCode::SuperRight]), false));
        assert!(reveal_all_requested(&keys(&[]), true));
        assert!(!reveal_all_requested(&keys(&[KeyCode::ShiftLeft]), false));
        assert_eq!(
            HIDDEN_AREAS_MODIFIER_KEYS,
            [KeyCode::SuperLeft, KeyCode::SuperRight]
        );
    }

    #[test]
    fn lines_vanish_immediately_when_modifier_or_occupancy_is_gone() {
        let obstacles = [building(1, 0.0, 0.0, 5.0)];
        let list = ObstacleList::from_slice_all_active(&obstacles);
        let zones = derive_hidden_zones(&list);
        let feet = [GroundPoint::new(5.0, -2.0)];

        assert_eq!(visible_zone_indices(&zones, &feet, false), vec![0]);
        assert!(reveal_all_requested(
            &keys(&[HIDDEN_AREAS_MODIFIER_KEYS[0]]),
            false
        ));
        // With the modifier released and no character inside, the outline
        // is gone on that same frame — nothing is latched.
        assert_eq!(
            visible_zone_indices(&zones, &[], reveal_all_requested(&keys(&[]), false)),
            Vec::<usize>::new(),
            "release hides the outline on the next frame without a latch"
        );
        assert_eq!(
            visible_zone_indices(
                &zones,
                &[GroundPoint::new(5.0, 5.0)],
                reveal_all_requested(&keys(&[]), false),
            ),
            Vec::<usize>::new(),
            "leaving the zone hides the outline on the next frame"
        );
    }
}
