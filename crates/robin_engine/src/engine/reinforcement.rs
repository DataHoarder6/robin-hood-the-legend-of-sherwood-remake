//! Runtime PC spawn for the `ALARM` console cheat / Robin's on-mission
//! death reinforcement.
//!
//! Picks a random reinforcement door, pulls an un-instanced non-VIP
//! peasant from the gang, spawns a new PC, registers it as an
//! enemy-detectable for every NPC, and launches a
//! `PASS_DOOR → MOVE(jitter)` sequence.
//!
//! Creation runs at the command or arrival boundary. The reinforcement
//! PC's sprite is preloaded at level-load time by
//! `preload_campaign_peasant_sprites` so the cache-only
//! `Sprite::load_frame_info_cached` call here always hits.

use super::EngineInner;
use crate::coordinates::{MapPoint, MapVec};
use crate::element::{
    ActorData, ActorPc, Command, Detectable, DetectableType, ElementData, ElementKind, Entity,
    EntityId, HULK_LENGTH, HumanData, PcData,
};
#[cfg(test)]
use crate::engine::LevelAssets;
use crate::engine::TickCtx;
use crate::order::OrderType;
use crate::sequence::{MoveFlags, Sequence, SequenceElement, SequenceElementData};

/// A request to bring a garrison body into the level from outside it.
///
/// `tier` and the per-mission budget belong to the caller that emits the
/// request: this only picks a body, opens a door and starts the walk.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SummonRequest {
    /// Which side's staged bodies may answer.
    pub faction: crate::element::Camp,
    /// A specific entry, or any the level offers.
    pub entry: SummonEntry,
    /// Lowest profile rank allowed to answer. Zero means any body.
    pub min_tier: u8,
}

/// Which authored entry a summon uses.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SummonEntry {
    Any,
    Door(crate::gate::DoorIndex),
    Offmap(usize),
}

/// Shared off-map placement for a scripted actor and a summoned body.
///
/// Finds the point beyond the map edge reached by walking backwards from
/// `destination` in `direction`, then puts `entity` there. The layer and
/// sector are only replaced when the caller supplies them — computed script
/// locations carry neither, while authored entry points and static script
/// points carry both.
pub(crate) fn place_outside_entity(
    map_bbox: crate::coordinates::MapBBox,
    entity: &mut Entity,
    destination: MapPoint,
    layer: Option<u16>,
    sector: Option<crate::position_interface::SectorHandle>,
    direction: i16,
) -> MapPoint {
    let (_border, (x, y)) = crate::natives::compute_border_point_bbox(
        map_bbox,
        (destination.x, destination.y),
        direction,
    );
    let outside = MapPoint::new(x, y);
    let element = entity.element_data_mut();
    if element.in_honolulu {
        element.active = true;
        element.in_honolulu = false;
    }
    element.set_position_map(outside);
    if let Some(layer) = layer {
        element.set_layer(layer);
    }
    if let Some(sector) = sector {
        element
            .sprite
            .position_iface
            .set_sector_topology(Some(sector), sector.arena_index());
    }
    element.update_grid_cell();
    outside
}

impl EngineInner {
    /// Put `actor` on the off-map spawn point opposite `destination`'s
    /// inward `direction`, and return that point. Shared with the
    /// `RecordEnterGame` native so a scripted actor and a summoned body use
    /// the same border computation.
    pub(crate) fn enter_from_outside(
        &mut self,
        actor: EntityId,
        destination: MapPoint,
        layer: u16,
        sector: Option<crate::position_interface::SectorHandle>,
        direction: i16,
    ) -> MapPoint {
        let map_bbox = self.world.fast_grid.level.map_bbox;
        let entity = self
            .get_entity_mut(actor)
            .unwrap_or_else(|| panic!("enter_from_outside: actor {actor:?} missing"));
        place_outside_entity(
            map_bbox,
            entity,
            destination,
            Some(layer),
            sector,
            direction,
        )
    }

    pub(crate) fn create_reinforcement(&mut self, tcx: TickCtx<'_>, dead_pc: Option<EntityId>) {
        // Pick a random reinforcement door.
        let door_count = self.ai.global.reinforcement_doors.len();
        if door_count == 0 {
            tracing::warn!("REINFORCEMENT: no reinforcement doors on this level.");
            return;
        }
        let pick = crate::sim_rng::usize(
            tcx.sim,
            crate::sim_rng::RngSite::ReinforcementDoor,
            0..door_count,
        );
        let door_index = self.ai.global.reinforcement_doors[pick].door_index;

        // Snapshot canonical door geometry before touching entities or the
        // campaign.
        let door_snap = {
            let door = self
                .script_domains
                .interactables
                .doors
                .get(usize::from(door_index))
                .unwrap_or_else(|| {
                    panic!("reinforcement references missing map door {door_index}")
                });
            let mut sector_out =
                crate::position_interface::SectorHandle::from_number(door.sector_out);
            if let Some(index) = door.sector_out_index {
                sector_out = sector_out.with_arena_index(index);
            }
            DoorSnapshot {
                point_out: door.point_out,
                point_in: door.point_in,
                layer_out: door.layer_out,
                layer_in: door.layer_in,
                sector_out,
            }
        };

        // Preferred profile: the dead PC's profile index (if any).
        let preferred_profile_idx = dead_pc
            .and_then(|id| self.get_entity(id))
            .and_then(|e| e.pc_data())
            .map(|pc| pc.profile_index);

        // Pick the peasant via the campaign helper.
        let Some(campaign) = Some(&mut self.mission_domain.campaign) else {
            return;
        };
        let Some(char_idx) = campaign.get_random_peasant_from_gang(
            tcx.sim,
            preferred_profile_idx,
            &tcx.assets.profile_manager,
        ) else {
            tracing::info!("REINFORCEMENT: no eligible peasant in gang.");
            return;
        };
        let profile_idx = match campaign
            .characters
            .get(char_idx)
            .and_then(|d| d.character_profile_idx)
        {
            Some(p) => p,
            None => return,
        };

        // Mark the PcDescription as instanced.
        if let Some(desc) = campaign.characters.get_mut(char_idx) {
            desc.instanced = true;
        }

        // Resolve the CharacterProfile (clone what we need so we can
        // drop the campaign borrow before mutating entities).
        let (profile_filename, profile_name, kind, has_lockpick, has_climb, has_jump) = {
            let Some(profile) = tcx.assets.profile_manager.get_character(profile_idx) else {
                return;
            };
            let kind = crate::character_kind::CharacterKind::from_profile(
                &profile.filename,
                &profile.profile_name,
            );
            let (has_lockpick, has_climb, has_jump) = PcData::movement_auth_from_profile(profile);
            (
                profile.filename.clone(),
                profile.profile_name.clone(),
                kind,
                has_lockpick,
                has_climb,
                has_jump,
            )
        };

        // Hide the dead PC's interface.  Reinforcement only fires
        // mid-mission so the Sherwood-display guard is a no-op anyway.
        // The replacement PC's portrait re-display is implicit: the new
        // entity below is built with `PcData::default()`, which leaves
        // `interface_hidden = false`.
        if let Some(dead_id) = dead_pc
            && let Some(pc) = self.get_entity_mut(dead_id).and_then(|e| e.pc_data_mut())
        {
            pc.interface_hidden = true;
        }

        // Resolve the sprite from the preloaded scriptor cache. The
        // gang-peasant sprites are all loaded at mission start by
        // `preload_campaign_peasant_sprites`, so a miss here means
        // preload missed a profile — treat as a bug rather than a
        // silent fallback.
        let mut sprite = crate::sprite::Sprite::default();
        if let Err(e) = sprite.load_frame_info_cached(
            &tcx.assets.sprite_scriptor,
            crate::sprite_script::FrameKind::Character,
            &profile_filename,
            &profile_name,
        ) {
            tracing::error!(
                "REINFORCEMENT: sprite cache lookup failed for '{}' (profile {}): {e}",
                profile_name,
                profile_idx,
            );
            return;
        }

        let pathfinder_index = tcx
            .assets
            .profile_manager
            .get_character(profile_idx)
            .expect("reinforcement character profile remains available")
            .pathfinder_index;
        let half_diagonal = self.world.fast_grid.try_move_box_half_diagonal(pathfinder_index as usize)
            .unwrap_or_else(|| panic!("reinforcement profile {profile_idx} references missing pathfinder slot {pathfinder_index}"));
        sprite.position_iface.configure_for_actor(
            crate::position_interface::PathfinderIndex::new(u16::from(pathfinder_index))
                .expect("character pathfinder index cannot equal 0xffff"),
            half_diagonal,
            door_snap.point_out,
        );

        // Build the new PC entity.  Uses door outer-point / outer-layer
        // as the spawn site — i.e. the new PC appears *outside* the map
        // and then walks in via the `PASS_DOOR` element below.
        //
        let obstacle_index = self.get_projection_area_index(
            tcx.assets,
            door_snap.sector_out,
            door_snap.layer_out,
            door_snap.point_out,
        );
        let spawn_sector = Some(door_snap.sector_out);

        let mut element = {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ActorPc;
            initial_element.sprite = sprite;
            initial_element
        };
        element.set_position_map(door_snap.point_out);
        element.set_layer(door_snap.layer_out);
        element.set_sector(spawn_sector);
        if let Some(obs) = obstacle_index {
            // `set_obstacle` always pulls the top-plane from the
            // obstacle whenever the obstacle is non-null — pre-resolve
            // the plane here.
            let plane = crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                Some(obs),
                tcx.assets.environment.static_sight_obstacles.as_slice(),
            );
            element.sprite.position_iface.set_obstacle(Some(obs), plane);
        }
        let entity = Entity::Pc(ActorPc {
            element,
            actor: ActorData::default(),
            human: HumanData {
                time_hulk: HULK_LENGTH,
                ..Default::default()
            },
            pc: PcData {
                robin: false,
                profile_index: profile_idx,
                campaign_description_index: Some(
                    u32::try_from(char_idx)
                        .expect("campaign character index must fit original-game index"),
                ),
                kind,
                has_lockpick,
                has_climb,
                has_jump,
                beam_me_index: -1,
                ..Default::default()
            },
        });
        let new_id = self.add_entity(entity);
        if let Some(dead_id) = dead_pc {
            self.retire_replaced_pc(dead_id);
        }

        // Register the new PC as a visible enemy for every NPC.
        self.add_detectable_for_all_npc(new_id, DetectableType::Enemy);

        // Build the PASS_DOOR → MOVE sequence: a movement element
        // through the gate (DOOR | MAP flags), followed by a straight
        // movement to a jitter point inside.
        let action = OrderType::WalkingUpright;
        let mut seq = Sequence::new();

        let mut pass = SequenceElement::new_movement(1, Command::PassDoor, Some(new_id), action);
        pass.data = SequenceElementData::Movement {
            destination: door_snap.point_in,
            layer: door_snap.layer_in,
            sector: None,
            gate_id: Some(door_index),
            line_id: None,
            element: None,
            flags: MoveFlags::DOOR | MoveFlags::MAP,
            tolerance: 0.0,
            direction: 0,
            action,
            speed_factor: 1.0,
            linked_seek: None,
            post_seek_sequence: None,
        };
        seq.append_element(pass);

        // Find a jitter point near the inside of the door: up to ten
        // tries, ±50 in each axis.
        let pin = door_snap.point_in;
        let hd = self
            .expect_entity(new_id, "reinforcement entry movement")
            .position_iface()
            .get_half_diagonal();
        let mut jitter: Option<MapPoint> = None;
        for _ in 0..10 {
            let dx = -50.0
                + 100.0
                    * crate::sim_rng::c_rand_unit_inclusive(
                        tcx.sim,
                        crate::sim_rng::RngSite::ReinforcementJitter,
                    );
            let dy = -50.0
                + 100.0
                    * crate::sim_rng::c_rand_unit_inclusive(
                        tcx.sim,
                        crate::sim_rng::RngSite::ReinforcementJitter,
                    );
            let candidate = pin + MapVec::new(dx, dy);
            if self
                .world
                .fast_grid
                .is_reachable_thick(pin, candidate, door_snap.layer_in, hd)
            {
                jitter = Some(candidate);
                break;
            }
        }
        if let Some(target) = jitter {
            let mut mv = SequenceElement::new_movement(2, Command::Move, Some(new_id), action);
            mv.data = SequenceElementData::Movement {
                destination: target,
                layer: door_snap.layer_in,
                sector: None,
                gate_id: None,
                line_id: None,
                element: None,
                flags: MoveFlags::STRAIGHT,
                tolerance: 0.0,
                direction: 0,
                action,
                speed_factor: 1.0,
                linked_seek: None,
                post_seek_sequence: None,
            };
            seq.append_element(mv);
        }

        let seq_id = self.launch_sequence(tcx, seq);
        tracing::info!(
            new_pc = ?new_id,
            ?seq_id,
            profile = %profile_idx,
            door = %door_index,
            "REINFORCEMENT: spawned reinforcement PC."
        );
    }

    /// Wake one staged body of `request.faction`, publish it into the
    /// level's counters, and start the `PASS_DOOR` → `MOVE` walk that
    /// carries it in. Returns the body it woke, or `None` when the feature
    /// is off, no body of that faction is staged, or the level has no
    /// reinforcement door to enter through.
    pub(crate) fn summon_garrison(
        &mut self,
        tcx: TickCtx<'_>,
        request: SummonRequest,
    ) -> Option<EntityId> {
        let frame = self.control.frame_counter;
        if !self.ai.global.offmap_budget.authorize(frame) {
            return None;
        }
        let result = self.summon_garrison_authorized(tcx, request);
        if result.is_none() {
            self.ai.global.offmap_budget.release();
        }
        result
    }

    fn summon_garrison_authorized(
        &mut self,
        tcx: TickCtx<'_>,
        request: SummonRequest,
    ) -> Option<EntityId> {
        if !self.control.sim_config.enable_offmap_reinforcements {
            return None;
        }
        if self.ai.global.reinforcement_doors.is_empty() && self.ai.global.offmap_entries.is_empty()
        {
            tracing::warn!("SUMMON: no reinforcement entries on this level.");
            return None;
        }

        // Weakest first: rank order is Soldier, Officer, Knight, None. A
        // failed profile lookup sorts last rather than inventing a rank.
        let mut best: Option<(EntityId, u8)> = None;
        for id in self.world.npc_registry_ids.iter().copied() {
            let Some(entity) = self.get_entity(id) else {
                continue;
            };
            let Some(soldier) = entity.soldier_data() else {
                continue;
            };
            if !soldier.staged
                || soldier.cached_camp != request.faction
                || entity.element_data().active
            {
                continue;
            }
            let rank = tcx
                .assets
                .profile_manager
                .get_soldier(soldier.soldier_profile_index)
                .map(|profile| profile.rank as u8)
                .unwrap_or(u8::MAX);
            if rank < request.min_tier {
                continue;
            }
            if best.is_none_or(|(_, prior)| rank < prior) {
                best = Some((id, rank));
            }
        }
        let body = best.map(|(id, _)| id)?;

        enum Entry {
            Door {
                door_index: crate::gate::DoorIndex,
                point_out: MapPoint,
                layer_out: u16,
                sector_out: Option<crate::position_interface::SectorHandle>,
                point_in: MapPoint,
                layer_in: u16,
                sector_in: Option<crate::position_interface::SectorHandle>,
            },
            Offmap(usize),
        }

        let door_entry = |index: crate::gate::DoorIndex| {
            self.ai
                .global
                .reinforcement_doors
                .iter()
                .find(|door| door.door_index == index)
                .map(|door| Entry::Door {
                    door_index: door.door_index,
                    point_out: door.point_out,
                    layer_out: door.layer_out,
                    sector_out: door.sector_out,
                    point_in: door.point_in,
                    layer_in: door.position_in.level,
                    sector_in: door.position_in.sector,
                })
        };
        let entry = match request.entry {
            SummonEntry::Door(index) => door_entry(index)?,
            SummonEntry::Offmap(index) => {
                self.ai.global.offmap_entries.get(index)?;
                Entry::Offmap(index)
            }
            SummonEntry::Any => {
                if !self.ai.global.reinforcement_doors.is_empty() {
                    let pick = crate::sim_rng::usize(
                        tcx.sim,
                        crate::sim_rng::RngSite::ReinforcementDoor,
                        0..self.ai.global.reinforcement_doors.len(),
                    );
                    door_entry(self.ai.global.reinforcement_doors[pick].door_index)?
                } else {
                    let pick = crate::sim_rng::usize(
                        tcx.sim,
                        crate::sim_rng::RngSite::ReinforcementDoor,
                        0..self.ai.global.offmap_entries.len(),
                    );
                    Entry::Offmap(pick)
                }
            }
        };

        let (alert, camp, money) = match entry {
            Entry::Door {
                point_out,
                layer_out,
                sector_out,
                ..
            } => {
                let Some(Entity::Soldier(actor)) = self.get_entity_mut(body) else {
                    return None;
                };
                let alert = actor
                    .npc
                    .ai
                    .ai_brain
                    .base()
                    .map(|base| base.current_music_alert_status)
                    .unwrap_or(crate::ai::AlertLevel::Green);
                let camp = actor.soldier.cached_camp;
                let money = actor.npc.ai.money;
                actor.soldier.staged = false;
                actor.soldier.summoned = true;
                actor.element.active = true;
                actor.element.set_position_map(point_out);
                actor.element.set_layer(layer_out);
                actor.element.set_sector(sector_out);
                actor.element.update_grid_cell();
                (alert, camp, money)
            }
            Entry::Offmap(index) => {
                let offmap = &self.ai.global.offmap_entries[index];
                let (point_out, layer, sector, direction) = (
                    offmap.point_out,
                    offmap.layer,
                    offmap.sector,
                    offmap.direction,
                );
                let Some(Entity::Soldier(actor)) = self.get_entity_mut(body) else {
                    return None;
                };
                actor.element.set_position_map(point_out);
                actor.element.set_layer(layer);
                actor.element.set_sector(sector);
                actor.element.set_direction_instantly(direction);
                actor.element.update_grid_cell();
                let alert = actor
                    .npc
                    .ai
                    .ai_brain
                    .base()
                    .map(|base| base.current_music_alert_status)
                    .unwrap_or(crate::ai::AlertLevel::Green);
                let camp = actor.soldier.cached_camp;
                let money = actor.npc.ai.money;
                actor.soldier.staged = false;
                actor.soldier.summoned = true;
                actor.element.active = true;
                (alert, camp, money)
            }
        };
        self.publish_soldier_alert_count(alert);
        self.record_soldier_in_level(camp, money);

        // TODO: register the arriving body in every NPC's detectable list
        // the way `create_reinforcement` does for a new PC, once the
        // summon path owns detection wiring for soldier targets.
        let action = OrderType::WalkingUpright;
        let mut seq = Sequence::new();
        match entry {
            Entry::Door {
                door_index,
                point_in,
                layer_in,
                sector_in,
                ..
            } => {
                let mut pass =
                    SequenceElement::new_movement(1, Command::PassDoor, Some(body), action);
                pass.data = SequenceElementData::Movement {
                    destination: point_in,
                    layer: layer_in,
                    sector: sector_in,
                    gate_id: Some(door_index),
                    line_id: None,
                    element: None,
                    flags: MoveFlags::DOOR | MoveFlags::MAP,
                    tolerance: 0.0,
                    direction: 0,
                    action,
                    speed_factor: 1.0,
                    linked_seek: None,
                    post_seek_sequence: None,
                };
                seq.append_element(pass);

                let half_diagonal = self
                    .expect_entity(body, "summoned body entry movement")
                    .position_iface()
                    .get_half_diagonal();
                let mut jitter: Option<MapPoint> = None;
                for _ in 0..10 {
                    let dx = -50.0
                        + 100.0
                            * crate::sim_rng::c_rand_unit_inclusive(
                                tcx.sim,
                                crate::sim_rng::RngSite::ReinforcementJitter,
                            );
                    let dy = -50.0
                        + 100.0
                            * crate::sim_rng::c_rand_unit_inclusive(
                                tcx.sim,
                                crate::sim_rng::RngSite::ReinforcementJitter,
                            );
                    let candidate = point_in + MapVec::new(dx, dy);
                    if self.world.fast_grid.is_reachable_thick(
                        point_in,
                        candidate,
                        layer_in,
                        half_diagonal,
                    ) {
                        jitter = Some(candidate);
                        break;
                    }
                }
                if let Some(target) = jitter {
                    let mut mv =
                        SequenceElement::new_movement(2, Command::Move, Some(body), action);
                    mv.data = SequenceElementData::Movement {
                        destination: target,
                        layer: layer_in,
                        sector: None,
                        gate_id: None,
                        line_id: None,
                        element: None,
                        flags: MoveFlags::STRAIGHT,
                        tolerance: 0.0,
                        direction: 0,
                        action,
                        speed_factor: 1.0,
                        linked_seek: None,
                        post_seek_sequence: None,
                    };
                    seq.append_element(mv);
                }
            }
            Entry::Offmap(index) => {
                let offmap = self.ai.global.offmap_entries[index].clone();
                let mut mv = SequenceElement::new_movement(1, Command::Move, Some(body), action);
                mv.data = SequenceElementData::Movement {
                    destination: offmap.target,
                    layer: offmap.layer,
                    sector: offmap.sector,
                    gate_id: None,
                    line_id: None,
                    element: None,
                    flags: MoveFlags::STRAIGHT,
                    tolerance: 0.0,
                    direction: 0,
                    action,
                    speed_factor: 1.0,
                    linked_seek: None,
                    post_seek_sequence: None,
                };
                seq.append_element(mv);
            }
        }

        let seq_id = self.launch_sequence(tcx, seq);
        tracing::info!(
            body = ?body,
            ?seq_id,
            "SUMMON: garrison body entered from outside the map."
        );
        Some(body)
    }

    /// Appends a `Detectable` pointing at `element_id` to every NPC's
    /// list slot for `det_type`.  Skips duplicates.  For
    /// `DetectableType::Enemy`, also applies the per-NPC camp/rank
    /// filter so only NPCs eligible to detect the target
    /// are populated.
    pub(crate) fn add_detectable_for_all_npc(
        &mut self,
        element_id: EntityId,
        det_type: DetectableType,
    ) {
        if det_type == DetectableType::Body {
            for index in 0..self.world.npc_registry_ids.len() {
                let owner = self.world.npc_registry_ids[index];
                self.execute_ai_add_detectable(owner, element_id, det_type);
            }
            self.expect_entity_mut(element_id, "body publication target")
                .human_data_mut()
                .expect("body publication requires a human")
                .already_detectable_body = true;
            return;
        }
        let idx = det_type as usize;
        if idx >= DetectableType::COUNT {
            return;
        }
        // Pre-resolve target classification once for the per-NPC filter.
        let target_info = self.get_entity(element_id).map(|e| {
            (
                e.is_pc(),
                e.is_soldier(),
                e.is_civilian(),
                e.camp(),
                e.is_human(),
            )
        });
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for npc_id in npc_ids {
            // For DETECTABLE_ENEMY, only push if the NPC's camp/rank
            // arm accepts the target.  Other arms unconditionally push.
            if det_type == DetectableType::Enemy {
                let Some((tgt_pc, tgt_soldier, _tgt_civilian, tgt_camp, tgt_human)) = target_info
                else {
                    continue;
                };
                if !tgt_human {
                    // ENEMY targets must be human.
                    continue;
                }
                let Some(npc_entity) = self.get_entity(npc_id) else {
                    continue;
                };
                if !crate::ai_detectable_filter::should_add_enemy_detectable_with(
                    &self.mission_domain.diplomacy,
                    npc_entity.camp(),
                    npc_entity.is_soldier(),
                    tgt_pc,
                    tgt_soldier,
                    tgt_camp,
                ) {
                    continue;
                }
            }
            if let Some(entity) = self.get_entity_mut(npc_id) {
                let npc = match entity {
                    Entity::Soldier(s) => Some(&mut s.npc),
                    Entity::Civilian(c) => Some(&mut c.npc),
                    _ => None,
                };
                let Some(npc) = npc else { continue };
                if idx >= npc.detectable_lists.len() {
                    continue;
                }
                let already = npc.detectable_lists[idx]
                    .iter()
                    .any(|d| d.element == Some(element_id));
                if !already {
                    npc.detectable_lists[idx].push(Detectable {
                        element: Some(element_id),
                        detectable_type: det_type,
                        ..Default::default()
                    });
                }
            }
        }
    }
}

struct DoorSnapshot {
    point_out: MapPoint,
    point_in: MapPoint,
    layer_out: u16,
    layer_in: u16,
    sector_out: crate::position_interface::SectorHandle,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_reinforcement_publishes_pc_and_entry_sequence_before_returning() {
        let sim = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        assert!(engine.scripts.mission.is_none());
        let profile = crate::profiles::CharacterProfile {
            filename: "Peasant".into(),
            profile_name: "Paysan A".into(),
            ..Default::default()
        };
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .push(profile);
        std::sync::Arc::make_mut(&mut assets.sprite_scriptor).insert(
            "Peasant/Paysan A",
            crate::sprite_script::SpriteInfo {
                scripts: std::sync::Arc::new(Vec::new()),
                conversion: std::sync::Arc::new(crate::engine::test_support::unmapped_conversion()),
                size: crate::coordinates::SpriteSize::new(12.0, 8.0),
                center: crate::coordinates::SpriteAnchor::new(2.0, 3.0),
            },
        );
        engine
            .mission_domain
            .campaign
            .characters
            .push(crate::campaign::PcDescription {
                character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
                instanced: false,
                status: Default::default(),
            });
        engine.mission_domain.campaign.gang_indices.push(0);
        engine
            .world
            .fast_grid_mut()
            .add_move_box_half_diagonal(crate::coordinates::MoveBoxHalfDiagonal::new(4.0, 6.0));
        crate::engine::test_support::ensure_ordinary_sector(&mut engine, 0, 0);
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                sector_out: crate::sector::SectorNumber::new(0),
                sector_out_index: crate::fast_find_grid::SectorIndex::new(0),
                ..Default::default()
            });
        engine
            .ai
            .global
            .reinforcement_doors
            .push(crate::ai::ReinforcementDoorInfo {
                position_in: crate::ai::Position {
                    x: 0.0,
                    y: 0.0,
                    sector: None,
                    level: 0,
                },
                door_index: crate::gate::DoorIndex::new(0).unwrap(),
                point_out: MapPoint::new(0.0, 0.0),
                point_mid: MapPoint::new(0.0, 0.0),
                layer_out: 0,
                sector_out: None,
                point_in: MapPoint::new(0.0, 0.0),
            });

        engine.dispatch_sim_console_command(
            TickCtx::new(&sim, &assets),
            &mut None,
            &crate::console::ConsoleCommand::Reinforcement,
        );

        assert!(engine.mission_domain.campaign.characters[0].instanced);
        let (id, pc) = engine
            .world
            .entities
            .occupied()
            .find(|(_, entity)| entity.is_pc())
            .expect("reinforcement is already published");
        assert_eq!(pc.pc_data().unwrap().campaign_description_index, Some(0));
        assert_eq!(
            pc.position_iface().get_half_diagonal(),
            crate::coordinates::MoveBoxHalfDiagonal::new(4.0, 6.0)
        );
        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("entry sequence is already registered");
        assert_eq!(sequence.elements[0].owner, Some(id));
        assert_eq!(sequence.elements[0].command, Command::PassDoor);
    }

    #[test]
    #[should_panic(expected = "reinforcement references missing map door 0")]
    fn reinforcement_rejects_missing_map_door() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = EngineInner::new();
        engine
            .ai
            .global
            .reinforcement_doors
            .push(crate::ai::ReinforcementDoorInfo {
                position_in: crate::ai::Position {
                    x: 0.0,
                    y: 0.0,
                    sector: None,
                    level: 0,
                },
                door_index: crate::gate::DoorIndex::new(0).expect("valid door index"),
                point_out: MapPoint::new(0.0, 0.0),
                point_mid: MapPoint::new(0.0, 0.0),
                layer_out: 0,
                sector_out: None,
                point_in: MapPoint::new(0.0, 0.0),
            });

        engine.create_reinforcement(TickCtx::new(sim, &LevelAssets::new()), None);
    }
}
