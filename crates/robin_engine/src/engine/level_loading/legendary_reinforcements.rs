use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::*;

// TODO: Re-enable after reinforcement patches identify compatible mission content
// across supported editions and overlays, rather than relying on roster counts.
const REINFORCEMENTS_ENABLED: bool = false;

/// Issue #61 content pilot: the one mission whose placement additions are
/// staged garrison bodies with a nonzero budget. Every other mission keeps its
/// original, non-staged content.
const PILOT_MISSION: &str = "Emb01_FoA_EC";

const PATCH_DATA: &str = include_str!("legendary_enemy_patches.json");

#[derive(Debug, Serialize, Deserialize)]
struct LegendaryPatchManifest {
    version: u32,
    preset: String,
    missions: Vec<LegendaryMissionPatch>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LegendaryMissionPatch {
    mission: String,
    authored_soldier_count: usize,
    selected_proposal_ids: Vec<String>,
    additions: Vec<LegendarySoldierAddition>,
    existing_officer_links: Vec<ExistingOfficerLink>,
    new_patrol_groups: Vec<NewPatrolGroup>,
    /// Off-map garrison budget for this mission. Zero means no summons, which
    /// is the default for every mission outside the issue #61 content pilot.
    #[serde(default)]
    reinforcement_budget: crate::level_data::RawReinforcementBudget,
}

#[derive(Debug, Serialize, Deserialize)]
struct LegendarySoldierAddition {
    source_index: usize,
    position_x: u16,
    position_y: u16,
    layer: u16,
    direction: u32,
    path_behavior: AddedPathBehavior,
    proposal_id: String,
    /// Wait off-map until an alarm wakes the body. Default false keeps every
    /// original placement active as before.
    #[serde(default)]
    staged: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AddedPathBehavior {
    Inherit,
    Stationary,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExistingOfficerLink {
    officer_index: usize,
    addition_indices: Vec<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NewPatrolGroup {
    addition_indices: Vec<usize>,
}

fn patch_manifest() -> &'static LegendaryPatchManifest {
    static MANIFEST: OnceLock<LegendaryPatchManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        serde_json::from_str(PATCH_DATA)
            .expect("embedded Legendary enemy patch manifest must be valid JSON")
    })
}

/// Apply the content-only fields of a reviewed patch: stage the selected
/// additions and install the mission's off-map budget. Kept separate from the
/// gated loader so the pilot content can be proved through the live path in
/// tests while production stays byte-identical until the director lands.
fn install_pilot_content(
    patch: &LegendaryMissionPatch,
    additions: &mut [crate::level_data::RawSoldier],
    loaded: &mut crate::level_data::LoadedLevel,
) {
    if !patch.mission.eq_ignore_ascii_case(PILOT_MISSION) {
        return;
    }
    for (raw, addition) in additions.iter_mut().zip(&patch.additions) {
        raw.staged = addition.staged;
    }
    if patch.reinforcement_budget.total > 0 {
        match loaded.mission.tactic_data.as_mut() {
            Some(tactic) => tactic.reinforcement_budget = patch.reinforcement_budget,
            None => {
                loaded.mission.tactic_data = Some(crate::level_data::RawTacticData {
                    reinforcement_points: Vec::new(),
                    reinforcement_budget: patch.reinforcement_budget,
                    ambush_points: Vec::new(),
                    seek_points: Vec::new(),
                    archery_sectors: Vec::new(),
                });
            }
        }
    }
}

impl EngineInner {
    pub(super) fn apply_legendary_reinforcements(
        &mut self,
        assets: &LevelAssets,
        loaded: &mut crate::level_data::LoadedLevel,
        mission_name: &str,
        config: SimConfig,
    ) -> Result<Vec<crate::level_data::RawSoldier>, EngineError> {
        if !REINFORCEMENTS_ENABLED
            || config.difficulty != crate::player_profile::DifficultyLevel::Legendary
        {
            return Ok(Vec::new());
        }
        let manifest = patch_manifest();
        if manifest.version != 1 || manifest.preset != "legendary" {
            return Err(EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!(
                    "unsupported embedded patch manifest version/preset: {}/{}",
                    manifest.version, manifest.preset
                ),
            });
        }
        let Some(patch) = manifest
            .missions
            .iter()
            .find(|patch| patch.mission.eq_ignore_ascii_case(mission_name))
        else {
            tracing::debug!(mission = mission_name, "no reviewed Legendary enemy patch");
            return Ok(Vec::new());
        };
        if loaded.mission.soldiers.len() != patch.authored_soldier_count {
            return Err(EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!(
                    "mission {mission_name} has {} authored soldiers, but reviewed patch expects {}",
                    loaded.mission.soldiers.len(),
                    patch.authored_soldier_count
                ),
            });
        }
        let authored_count = loaded.mission.soldiers.len();
        let final_count = authored_count
            .checked_add(patch.additions.len())
            .ok_or_else(|| EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!("mission {mission_name} soldier count overflow"),
            })?;
        if final_count > usize::from(u16::MAX) {
            return Err(EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!("mission {mission_name} would exceed the u16 soldier-index limit"),
            });
        }

        let authored = loaded.mission.soldiers.clone();
        let mut additions = Vec::with_capacity(patch.additions.len());
        for addition in &patch.additions {
            let mut raw = authored
                .get(addition.source_index)
                .cloned()
                .ok_or_else(|| EngineError::MissionLevelStage {
                    stage: "Legendary enemy reinforcements",
                    reason: format!(
                        "mission {mission_name} proposal {} references missing source soldier {}",
                        addition.proposal_id, addition.source_index
                    ),
                })?;
            let point = MapPoint::new(
                f32::from(addition.position_x),
                f32::from(addition.position_y),
            );
            raw.position_x = addition.position_x;
            raw.position_y = addition.position_y;
            raw.layer = addition.layer;
            raw.direction = addition.direction & 15;
            raw.sector = self.legendary_sparse_sector_for_point(
                assets,
                point,
                addition.layer,
                mission_name,
                &addition.proposal_id,
            )?;
            raw.obstacle_index = u16::MAX;
            raw.money = 0;
            raw.script_class = None;
            raw.subordinate_ids.clear();
            if matches!(addition.path_behavior, AddedPathBehavior::Stationary) {
                raw.path_id = crate::level_data::NO_HIKING_PATH_ID;
                raw.alert_path_id = crate::level_data::NO_HIKING_PATH_ID;
            }
            additions.push(raw);
        }

        let global_index = |local_index: usize| -> Result<u16, EngineError> {
            let index = authored_count.checked_add(local_index).ok_or_else(|| {
                EngineError::MissionLevelStage {
                    stage: "Legendary enemy reinforcements",
                    reason: format!("mission {mission_name} addition index overflow"),
                }
            })?;
            u16::try_from(index).map_err(|_| EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!("mission {mission_name} addition index {index} exceeds u16"),
            })
        };
        for link in &patch.existing_officer_links {
            if link.officer_index >= authored_count {
                return Err(EngineError::MissionLevelStage {
                    stage: "Legendary enemy reinforcements",
                    reason: format!(
                        "mission {mission_name} references missing officer {}",
                        link.officer_index
                    ),
                });
            }
            let subordinate_ids = link
                .addition_indices
                .iter()
                .map(|&index| global_index(index))
                .collect::<Result<Vec<_>, _>>()?;
            loaded.mission.soldiers[link.officer_index]
                .subordinate_ids
                .extend(subordinate_ids);
        }
        for group in &patch.new_patrol_groups {
            let (&commander_local, followers) =
                group.addition_indices.split_first().ok_or_else(|| {
                    EngineError::MissionLevelStage {
                        stage: "Legendary enemy reinforcements",
                        reason: format!(
                            "mission {mission_name} contains an empty added patrol group"
                        ),
                    }
                })?;
            let commander_global = usize::from(global_index(commander_local)?);
            let subordinate_ids = followers
                .iter()
                .map(|&index| global_index(index))
                .collect::<Result<Vec<_>, _>>()?;
            additions[commander_global - authored_count].subordinate_ids = subordinate_ids;
        }

        install_pilot_content(patch, &mut additions, loaded);

        tracing::info!(
            mission = mission_name,
            additions = patch.additions.len(),
            proposals = ?patch.selected_proposal_ids,
            "applied reviewed Legendary enemy reinforcements"
        );
        Ok(additions)
    }

    fn legendary_sparse_sector_for_point(
        &self,
        assets: &LevelAssets,
        point: MapPoint,
        layer: u16,
        mission_name: &str,
        proposal_id: &str,
    ) -> Result<u16, EngineError> {
        if !self.world.fast_grid.is_inside_grid_point(point) {
            return Err(EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!(
                    "mission {mission_name} proposal {proposal_id} places a soldier outside walkable motion geometry at ({}, {}) layer {layer}",
                    point.x, point.y
                ),
            });
        }
        // `get_sector` deliberately prioritizes doors, lifts, patches and jump
        // sectors for cursor/movement queries. A spawn position instead needs
        // the underlying motion AREA identity that Original serialized on the
        // soldier, even when a special sector overlaps it.
        let block = self.world.fast_grid.get_block_index(point, layer);
        let mut motion_area = None;
        let mut blocked = false;
        for (index, sector) in self
            .world
            .fast_grid
            .get_sectors_at_block(block, crate::sector::SectorType::MOTION)
        {
            if !sector.contains_point(point) {
                continue;
            }
            if sector.sector_type.is_area() {
                motion_area = crate::fast_find_grid::SectorIndex::new(index);
            } else {
                blocked = true;
            }
        }
        let Some(sector_idx) = motion_area.filter(|_| !blocked) else {
            return Err(EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!(
                    "mission {mission_name} proposal {proposal_id} places a soldier outside walkable motion geometry at ({}, {}) layer {layer}",
                    point.x, point.y
                ),
            });
        };
        let topology = assets
            .navigation
            .legacy_grid_topology
            .as_ref()
            .ok_or_else(|| EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!("mission {mission_name} has no retained sparse sector topology"),
            })?;
        topology
            .position_sector_indices
            .iter()
            .position(|candidate| *candidate == Some(sector_idx))
            .and_then(|slot| u16::try_from(slot).ok())
            .ok_or_else(|| EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!(
                    "mission {mission_name} proposal {proposal_id} resolved to an unmappable runtime sector"
                ),
            })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn embedded_manifest_has_valid_unique_indices() {
        let manifest = patch_manifest();
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.preset, "legendary");
        let mut missions = BTreeSet::new();
        let mut patrol_groups = 0;
        for patch in &manifest.missions {
            assert!(missions.insert(patch.mission.to_ascii_lowercase()));
            assert!(!patch.additions.is_empty());
            assert_eq!(
                patch
                    .selected_proposal_ids
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len(),
                patch.selected_proposal_ids.len()
            );
            for addition in &patch.additions {
                assert!(addition.source_index < patch.authored_soldier_count);
                assert!(addition.direction < 16);
            }
            for link in &patch.existing_officer_links {
                assert!(link.officer_index < patch.authored_soldier_count);
                assert!(!link.addition_indices.is_empty());
                assert!(
                    link.addition_indices
                        .iter()
                        .all(|&index| index < patch.additions.len())
                );
            }
            for group in &patch.new_patrol_groups {
                patrol_groups += 1;
                assert!(group.addition_indices.len() >= 2);
                assert!(
                    group
                        .addition_indices
                        .iter()
                        .all(|&index| index < patch.additions.len())
                );
            }
        }
        assert!(
            patrol_groups >= 2,
            "reviewed manifest should exercise cloned patrol ownership"
        );
    }

    fn pilot_patch() -> &'static LegendaryMissionPatch {
        patch_manifest()
            .missions
            .iter()
            .find(|patch| patch.mission.eq_ignore_ascii_case(PILOT_MISSION))
            .expect("the pilot mission must be in the embedded manifest")
    }

    fn raw_soldier() -> crate::level_data::RawSoldier {
        crate::level_data::RawSoldier {
            position_x: 0,
            position_y: 0,
            direction: 0,
            action: 0,
            obstacle_index: 0,
            sector: 0,
            layer: 0,
            material: 0,
            profile_number: 0,
            profile_id: None,
            allegiance: None,
            command_interface: crate::human_control::CommandInterface::None,
            mission_role: crate::human_control::MissionRole::Combatant,
            combat_stance: crate::human_control::CombatStance::Aggressive,
            revealed: false,
            staged: false,
            tower_guard: false,
            company_number: 0,
            drunk_level: 0,
            money: 0,
            subordinate_ids: Vec::new(),
            path_id: 0,
            alert_path_id: 0,
            script_class: None,
        }
    }

    #[test]
    fn pilot_is_the_only_mission_with_staged_content() {
        let manifest = patch_manifest();
        assert_eq!(
            manifest.missions.len(),
            34,
            "the reviewed placement set still has 34 missions"
        );
        let pilot = pilot_patch();
        assert_eq!(pilot.additions.len(), 6);
        assert!(pilot.additions.iter().all(|addition| addition.staged));
        assert_eq!(pilot.reinforcement_budget.total, 2);
        assert_eq!(pilot.reinforcement_budget.concurrent, 1);
        assert_eq!(pilot.reinforcement_budget.cooldown_ticks, 30);
        for patch in &manifest.missions {
            if patch.mission.eq_ignore_ascii_case(PILOT_MISSION) {
                continue;
            }
            assert!(
                patch.additions.iter().all(|addition| !addition.staged),
                "{} must keep its original non-staged placements",
                patch.mission
            );
            assert_eq!(
                patch.reinforcement_budget.total, 0,
                "{} must keep a zero reinforcement budget",
                patch.mission
            );
        }
    }

    #[test]
    fn pilot_content_stages_bodies_and_installs_budget() {
        let patch = pilot_patch();
        let mut additions = vec![raw_soldier(); patch.additions.len()];
        let mut loaded = crate::level_data::LoadedLevel::empty();

        install_pilot_content(patch, &mut additions, &mut loaded);

        assert!(additions.iter().all(|raw| raw.staged));
        let budget = loaded
            .mission
            .tactic_data
            .as_ref()
            .expect("the pilot installs tactic data")
            .reinforcement_budget;
        assert_eq!(budget.total, 2);
        assert_eq!(budget.concurrent, 1);
        assert_eq!(budget.cooldown_ticks, 30);

        // finish.rs installs that authored budget into the live AI state.
        let mut engine = EngineInner::new();
        engine.install_reinforcement_doors_stage(&LevelAssets::new(), &loaded);
        let live = engine.ai.global.offmap_budget;
        assert_eq!(live.total, 2);
        assert_eq!(live.remaining, 2);
        assert_eq!(live.concurrent, 1);
        assert_eq!(live.cooldown_ticks, 30);
    }

    fn pilot_garrison_engine() -> (EngineInner, Vec<crate::element::EntityId>) {
        use crate::ai::{Position, ReinforcementDoorInfo};
        use crate::coordinates::MapBBox;
        use crate::coordinates::MapPoint;
        use crate::element::{Camp, Entity};
        use crate::gate::DoorIndex;

        let patch = pilot_patch();
        let mut engine = EngineInner::new();
        let mut bbox = MapBBox::new();
        bbox.expand_point(MapPoint::new(0.0, 0.0));
        bbox.expand_point(MapPoint::new(1000.0, 1000.0));
        engine.world.fast_grid_mut().level_mut().map_bbox = bbox;

        let bodies = (0..patch.additions.len())
            .map(|_| {
                let mut actor =
                    crate::engine::test_support::actors::make_test_ai_soldier(Camp::Lacklandists);
                actor.element_data_mut().active = false;
                actor.position_iface_mut().configure_for_actor(
                    crate::position_interface::PathfinderIndex::new(0).expect("pathfinder index"),
                    crate::coordinates::MoveBoxHalfDiagonal::new(20.0, 20.0),
                    MapPoint::new(40.0, 100.0),
                );
                let Entity::Soldier(soldier) = &mut actor else {
                    unreachable!("fixture is a soldier");
                };
                soldier.soldier.staged = true;
                engine.add_test_entity(actor)
            })
            .collect();

        engine
            .ai
            .global
            .reinforcement_doors
            .push(ReinforcementDoorInfo {
                position_in: Position {
                    x: 100.0,
                    y: 100.0,
                    sector: None,
                    level: 0,
                },
                door_index: DoorIndex::new(0).expect("door index"),
                point_out: MapPoint::new(40.0, 100.0),
                point_mid: MapPoint::new(70.0, 100.0),
                point_in: MapPoint::new(100.0, 100.0),
                layer_out: 0,
                sector_out: None,
            });

        let mut loaded = crate::level_data::LoadedLevel::empty();
        let mut additions = vec![raw_soldier(); patch.additions.len()];
        install_pilot_content(patch, &mut additions, &mut loaded);
        engine.install_reinforcement_doors_stage(&LevelAssets::new(), &loaded);

        (engine, bodies)
    }

    #[test]
    fn pilot_content_wakes_a_guard_under_alert_without_new_rng_draws() {
        use crate::element::Camp;
        use crate::engine::reinforcement::{SummonEntry, SummonRequest};
        use crate::gate::DoorIndex;

        let (mut engine, bodies) = pilot_garrison_engine();
        assert!(
            bodies.iter().all(|id| !engine
                .get_entity(*id)
                .expect("staged body")
                .element_data()
                .active),
            "staged bodies start inactive at level load (entities.rs)"
        );
        for id in &bodies {
            engine.enemy_mut(*id).base.current_music_alert_status = crate::ai::AlertLevel::Yellow;
        }

        // The pilot adds no new RNG site. The existing summon consumes only
        // two already-reviewed sites: ReinforcementJitter for the arrival
        // walk (bounded 10-step loop) and ReinforcementDoor on the Any path.
        // Seed enough draws for that loop so the replay does not panic.
        let replay = std::sync::Arc::new(std::sync::Mutex::new(
            crate::sim_rng::OriginalRngReplay::new(vec![0; 24]),
        ));
        #[allow(clippy::disallowed_methods)]
        let rng_state = std::sync::Arc::new(std::sync::Mutex::new(fastrand::Rng::with_seed(1)));
        let sim = crate::sim_rng::SimulationContext::new(
            std::sync::Arc::clone(&rng_state),
            Some(std::sync::Arc::clone(&replay)),
            crate::engine::SimConfig::default(),
        );
        let assets = engine.test_runtime_assets();
        let woken = engine
            .summon_garrison(
                TickCtx::new(&sim, &assets),
                SummonRequest {
                    faction: Camp::Lacklandists,
                    entry: SummonEntry::Door(DoorIndex::new(0).expect("door index")),
                    min_tier: 0,
                },
            )
            .expect("a Yellow alert wakes one staged pilot body");

        assert!(bodies.contains(&woken));
        let entity = engine.get_entity(woken).expect("woken body");
        assert!(entity.element_data().active);
        assert!(!entity.soldier_data().expect("soldier data").staged);
        assert_eq!(engine.ai.global.offmap_budget.remaining, 1);
        assert_eq!(engine.ai.global.offmap_budget.live, 1);
        let replay = replay.lock().expect("replay lock");
        let used_sites = replay.sites(0..replay.cursor());
        assert!(
            used_sites.iter().all(|site| matches!(
                site,
                crate::sim_rng::RngSite::ReinforcementJitter
                    | crate::sim_rng::RngSite::ReinforcementDoor
            )),
            "the pilot summon must use only pre-reviewed reinforcement RNG sites, got {used_sites:?}"
        );
    }

    #[test]
    fn pilot_content_is_inert_when_the_option_is_off() {
        use crate::element::Camp;
        use crate::engine::reinforcement::{SummonEntry, SummonRequest};
        use crate::gate::DoorIndex;

        let (mut engine, bodies) = pilot_garrison_engine();
        engine.control.sim_config.enable_offmap_reinforcements = false;
        for id in &bodies {
            engine.enemy_mut(*id).base.current_music_alert_status = crate::ai::AlertLevel::Red;
        }
        let sim = crate::sim_rng::test_context();
        let assets = engine.test_runtime_assets();
        assert!(
            engine
                .summon_garrison(
                    TickCtx::new(&sim, &assets),
                    SummonRequest {
                        faction: Camp::Lacklandists,
                        entry: SummonEntry::Door(DoorIndex::new(0).expect("door index")),
                        min_tier: 0,
                    },
                )
                .is_none(),
            "with the option off the staged garrison stays asleep"
        );
        assert!(bodies.iter().all(|id| {
            !engine
                .get_entity(*id)
                .expect("staged body")
                .element_data()
                .active
        }));
        // Existing budget semantics: authorize() spends the slot before the
        // flag check and release() never refunds `remaining`. The woken-body
        // assertion above is the option-off proof; no new draw was added.
        assert_eq!(engine.ai.global.offmap_budget.remaining, 1);
        assert_eq!(engine.ai.global.offmap_budget.live, 0);
    }
}
