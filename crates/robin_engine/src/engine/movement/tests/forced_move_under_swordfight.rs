//! Characterisation: what happens to a PC movement carrying
//! [`MoveFlags::FORCE_SWORD_MOVEMENT`] when a swordfight starts around the
//! moving PC.
//!
//! `group_move_pc_simple_route` stamps that flag for the "fight through,
//! don't stop" perk, and `abort_orphaned_sword_movement` reads it when the
//! last opponent leaves. These tests pin what the engine actually does when
//! an enemy starts the fight while the movement is still walking, and
//! whether the destination is honoured once the fight ends. They assert the
//! measured behaviour; they do not ask for a particular one.
//!
//! Measured: the entry interrupts (destroys) the movement before the actor
//! ever gets a movement frame, in both the flagged and the unflagged case, and
//! the PC never resumes toward its destination. The flag is not consulted on
//! this path — it only guards `abort_orphaned_sword_movement`, which needs a
//! movement that is still `InProgress` when the last opponent leaves.

#[cfg(test)]
mod suite {
    use super::super::*;
    use crate::element::{
        ActionState, ActorData, ActorPc, Command, ElementData, ElementKind, Entity, HumanData,
        PcData, Posture,
    };
    use crate::engine::test_support::actors::TestActor;
    use crate::engine::test_support::unmapped_conversion;
    use crate::order::Order;
    use crate::sequence::{
        MoveFlags, SequenceElement, SequenceElementData, SequencePriority, SequenceState,
    };
    use crate::sprite_script::SpriteScript;

    fn assets_with_test_pc_profile() -> LevelAssets {
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles.characters.push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..crate::profiles::CharacterProfile::default()
        });
        profiles.soldiers.push(crate::profiles::SoldierProfile {
            hth_weapon_id: 1,
            ..crate::profiles::SoldierProfile::default()
        });
        profiles
            .hth_weapons
            .push(crate::profiles::HtHWeaponProfile {
                // Melee (Uber) reach must admit the adjacent opponent, otherwise
                // `enter_swordfight` rejects the pair before any of the movement
                // handling these tests characterise runs.
                distance: [10, 30, 50, 120],
                ..crate::profiles::HtHWeaponProfile::default()
            });
        LevelAssets {
            profile_manager: std::sync::Arc::new(profiles),
            ..LevelAssets::new()
        }
    }

    /// One large ordinary sector both actors stand in: swordfight evaluation
    /// reads a live sector for the owner and its principal.
    fn install_ground_sector(engine: &mut EngineInner) -> crate::position_interface::SectorHandle {
        engine.world.fast_grid_mut().size_map(128, 128);
        engine.world.fast_grid_mut().allocate_layers(1);
        let index = engine.world.fast_grid_mut().add_sector(
            crate::engine::test_support::square_sector(
                1,
                0,
                MapPoint::new(1.0, 1.0),
                MapPoint::new(2000.0, 2000.0),
            ),
            0,
        );
        crate::position_interface::SectorHandle::new(1)
            .expect("sector 1 is a valid handle")
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).expect("sector index"))
    }

    struct Fixture {
        engine: EngineInner,
        owner: EntityId,
        movement_sequence: crate::sequence::SequenceId,
        start: MapPoint,
        destination: MapPoint,
        sector: crate::position_interface::SectorHandle,
    }

    /// A PC part-way through an authored `WalkingWithSword` movement — the
    /// shape `group_move_pc_simple_route` produces for a group move.
    fn install_pc_sword_move(force: bool) -> Fixture {
        let assets = LevelAssets::new();
        let mut engine = EngineInner::new();
        let sector = install_ground_sector(&mut engine);
        let start = MapPoint::new(100.0, 100.0);
        let destination = MapPoint::new(140.0, 100.0);

        let walking_script = SpriteScript {
            action_id: OrderType::WalkingSword as u16,
            action_done: 0,
            average_speed: 10.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 10,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![10],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut conversion = unmapped_conversion();
        conversion[OrderType::WalkingSword as usize] = 0;
        let mut element = ElementData::from_initial_posture(Posture::Upright);
        element.kind = ElementKind::ActorPc;
        element.active = true;
        element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![walking_script; 16]),
            std::sync::Arc::new(conversion),
        );
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        element.sprite.position_iface.set_anti_collision_on(false);
        element.set_position_map(start);
        element.set_sector(Some(sector));

        let owner = engine.add_test_entity(Entity::Pc(ActorPc {
            element,
            actor: ActorData {
                action_state: ActionState::MovingSword,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData {
                life_points: 50,
                keep_moving_in_combat: true,
                ..PcData::default()
            },
        }));

        let order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(owner),
            OrderType::WalkingWithSword,
        );
        // `determine_priority` resolves an ordinary `Move` to `Normal`; the
        // fixture states it explicitly instead of leaving the sentinel.
        movement.priority = SequencePriority::Normal;
        movement.orders.push_back(Order::new(
            OrderType::WalkingWithSword,
            destination.x,
            destination.y,
            order_id,
        ));
        let SequenceElementData::Movement {
            destination: stored_destination,
            flags,
            ..
        } = &mut movement.data
        else {
            unreachable!("new_movement must create movement data")
        };
        *stored_destination = destination;
        if force {
            *flags |= MoveFlags::FORCE_SWORD_MOVEMENT;
        }
        let movement_sequence = engine.t_launch_element(&LevelAssets::new(), movement);
        engine.t_element_in_progress(&assets, movement_sequence, 0);
        engine.select_sequence_element(
            engine
                .orders
                .sequence_manager
                .get_element(movement_sequence, 0)
                .unwrap()
                .owner
                .unwrap(),
            Some((movement_sequence, 0)),
        );

        Fixture {
            engine,
            owner,
            movement_sequence,
            start,
            destination,
            sector,
        }
    }

    /// An enemy soldier in melee range starts the fight: the moving PC is the
    /// *opponent* of the entry, which is how a fight begins around a PC that
    /// is already walking.
    fn enemy_engages(fixture: &mut Fixture) -> EntityId {
        let assets = assets_with_test_pc_profile();
        let sim = crate::sim_rng::test_context();
        let mut enemy_ai = crate::ai_enemy::EnemyAi::new(0);
        enemy_ai.hth_weapon_id = 1;
        let mut enemy = TestActor::soldier(Posture::Upright)
            .life_points(50)
            .enemy_ai(enemy_ai)
            .build();
        enemy
            .element_data_mut()
            .set_position_map(MapPoint::new(110.0, fixture.start.y));
        enemy.position_iface_mut().set_anti_collision_on(false);
        enemy.element_data_mut().set_sector(Some(fixture.sector));
        let enemy = fixture.engine.add_test_entity(enemy);
        assert!(
            fixture.engine.enter_swordfight(
                TickCtx::new(&sim, &assets),
                enemy,
                fixture.owner,
                false,
            ),
            "fixture must establish a live swordfight between the enemy and the moving PC"
        );
        assert_eq!(
            fixture.engine.human(fixture.owner).opponents.len(),
            1,
            "the PC must be in the fight it was pulled into"
        );
        enemy
    }

    fn movement_element(fixture: &Fixture) -> &SequenceElement {
        fixture
            .engine
            .orders
            .sequence_manager
            .get_element(fixture.movement_sequence, 0)
            .expect("the movement element must stay registered")
    }

    fn tick(fixture: &mut Fixture, frames: usize) {
        let assets = assets_with_test_pc_profile();
        for _ in 0..frames {
            fixture.engine.t_tick_actor_owner_envelopes(&assets);
        }
    }

    fn position(fixture: &Fixture) -> MapPoint {
        fixture
            .engine
            .ent(fixture.owner)
            .element_data()
            .position_map()
    }

    /// Everything these tests assert about a movement that a fight started
    /// around: the element survives only as a record of what it was.
    fn assert_destroyed_at_entry(fixture: &Fixture, force: bool) {
        let element = movement_element(fixture);
        assert_eq!(
            element.state,
            SequenceState::Interrupted,
            "enemy-initiated swordfight enters through `stop_owner_current(Preference)`; a \
             WalkingWithSword order matches none of the waiting-transition rewrites in \
             `stop_movement_for_owner`, so the movement is interrupted, not postponed"
        );
        assert!(
            element.orders.is_empty(),
            "an interrupted movement keeps no orders to resume from"
        );
        let SequenceElementData::Movement {
            destination, flags, ..
        } = &element.data
        else {
            panic!("movement fixture changed data kind")
        };
        assert_eq!(
            *destination, fixture.destination,
            "the authored destination is still recorded on the destroyed element"
        );
        assert_eq!(
            flags.contains(MoveFlags::FORCE_SWORD_MOVEMENT),
            force,
            "the fixture must carry exactly the flag under test"
        );
        assert_eq!(
            position(fixture),
            fixture.start,
            "the movement never reached motion processing"
        );
    }

    #[test]
    fn forced_pc_sword_move_is_destroyed_by_an_enemy_initiated_swordfight() {
        let mut fixture = install_pc_sword_move(true);
        enemy_engages(&mut fixture);

        assert_destroyed_at_entry(&fixture, true);

        tick(&mut fixture, 2);
        assert_eq!(
            movement_element(&fixture).state,
            SequenceState::Interrupted,
            "the fight does not revive the movement"
        );
        assert_eq!(
            position(&fixture),
            fixture.start,
            "the PC stands still for the whole fight"
        );

        // The fight ends: every opponent is gone (knocked out / cleared).
        fixture.engine.human_mut(fixture.owner).opponents.clear();
        tick(&mut fixture, 6);

        assert_eq!(
            movement_element(&fixture).state,
            SequenceState::Interrupted,
            "FORCE_SWORD_MOVEMENT does not resurrect an entry-interrupted movement after \
             the fight ends; the flag is only read by `abort_orphaned_sword_movement`, \
             which needs an element that is still InProgress"
        );
        assert_eq!(
            fixture.engine.human(fixture.owner).opponents.len(),
            0,
            "the fight is over for the PC"
        );
        assert_eq!(
            position(&fixture),
            fixture.start,
            "the PC does not resume toward its destination, and never reaches {:?}",
            fixture.destination
        );
    }

    #[test]
    fn unforced_pc_sword_move_is_destroyed_by_an_enemy_initiated_swordfight() {
        let mut fixture = install_pc_sword_move(false);
        enemy_engages(&mut fixture);

        assert_destroyed_at_entry(&fixture, false);

        tick(&mut fixture, 2);
        assert_eq!(
            movement_element(&fixture).state,
            SequenceState::Interrupted,
            "the fight does not revive the movement"
        );
        assert_eq!(
            position(&fixture),
            fixture.start,
            "the PC stands still for the whole fight"
        );

        fixture.engine.human_mut(fixture.owner).opponents.clear();
        tick(&mut fixture, 6);

        assert_eq!(
            movement_element(&fixture).state,
            SequenceState::Interrupted,
            "without the flag the outcome is identical, so the flag makes no difference \
             to a movement destroyed at fight entry"
        );
        assert_eq!(
            position(&fixture),
            fixture.start,
            "the PC does not resume toward its destination, and never reaches {:?}",
            fixture.destination
        );
    }
}
