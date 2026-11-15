//! A PC movement carrying [`MoveFlags::FORCE_SWORD_MOVEMENT`] when a
//! swordfight starts around the moving PC.
//!
//! `group_move_pc_simple_route` stamps that flag for the "fight through,
//! don't stop" perk. Fight entry prepares the opponent with
//! `stop_owner_current(Preference)`, whose `stop_movement_for_owner` phase
//! either rewrites the movement into a waiting transition or interrupts it.
//!
//! Measured: a flagged movement survives that stop, keeps its
//! `WalkingWithSword` order and destination, advances on every frame it is in
//! progress while the opponent stays engaged, and reaches its destination.
//! The single non-advancing frame is the PC's own reciprocal
//! `EnterSwordfight` at `PostponeEverythingButInjuries`, which queues the
//! movement behind the raise-sword animation. Precedence pins both
//! directions: an `Injury`-level stop (a hit or knockout) still interrupts the
//! flagged movement, and an unflagged movement is still destroyed at entry.
//!
//! The same holds when the PC is the entry's *initiator*: swordfight
//! preparation opens its scope for the opponent, so the entry never stops the
//! PC's own movement at all, and the flagged movement simply walks on.

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
        element
            .sprite
            .position_iface
            .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
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

    /// An enemy soldier standing in melee range beside the walk. Beside, not
    /// on it: a swordfighter inside the mover's move box would block the step
    /// itself (world collision), which is a different cancellation than the
    /// fight entry.
    fn enemy_beside_path(fixture: &mut Fixture) -> EntityId {
        let mut enemy_ai = crate::ai_enemy::EnemyAi::new(0);
        enemy_ai.hth_weapon_id = 1;
        let mut enemy = TestActor::soldier(Posture::Upright)
            .life_points(50)
            .enemy_ai(enemy_ai)
            .build();
        enemy.element_data_mut().set_position_map(MapPoint::new(
            fixture.start.x + 10.0,
            fixture.start.y + 18.0,
        ));
        enemy.position_iface_mut().set_anti_collision_on(false);
        enemy.element_data_mut().set_sector(Some(fixture.sector));
        fixture.engine.add_test_entity(enemy)
    }

    /// An enemy soldier in melee range starts the fight: the moving PC is the
    /// *opponent* of the entry, which is how a fight begins around a PC that
    /// is already walking.
    fn enemy_engages(fixture: &mut Fixture) -> EntityId {
        let assets = assets_with_test_pc_profile();
        let sim = crate::sim_rng::test_context();
        let enemy = enemy_beside_path(fixture);
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

    /// The mirror image of [`enemy_engages`]: the walking PC is the entry's
    /// *initiator*. `enter_swordfight_impl` opens the preparation scope for
    /// the *opponent*, so the `stop_owner_current(Preference)` it runs — and
    /// the priority ranking that lets `FORCE_SWORD_MOVEMENT` outrank it —
    /// never reaches the PC's own movement on this side.
    fn pc_initiates(fixture: &mut Fixture) -> EntityId {
        let assets = assets_with_test_pc_profile();
        let sim = crate::sim_rng::test_context();
        let enemy = enemy_beside_path(fixture);
        assert!(
            fixture.engine.enter_swordfight(
                TickCtx::new(&sim, &assets),
                fixture.owner,
                enemy,
                false,
            ),
            "the PC must be able to start the fight with the enemy beside its path"
        );
        assert_eq!(
            fixture.engine.human(fixture.owner).opponents.len(),
            1,
            "the initiating PC is in the fight it started"
        );
        assert_eq!(
            fixture.engine.human(enemy).opponents.len(),
            1,
            "the enemy is engaged by the initiating PC"
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

    /// One production-ordered frame: the owner systems tick, then the
    /// sequence phase dispatches what those ticks queued. The fight entry
    /// launches the PC's reciprocal `EnterSwordfight` behind the owner phase,
    /// so a tick that runs only owner envelopes would never see it.
    fn frame(fixture: &mut Fixture) {
        let assets = assets_with_test_pc_profile();
        fixture.engine.t_tick_actor_owner_envelopes(&assets);
        fixture.engine.t_hourglass_phase_sequences(&assets);
    }

    fn position(fixture: &Fixture) -> MapPoint {
        fixture
            .engine
            .ent(fixture.owner)
            .element_data()
            .position_map()
    }

    fn distance(from: MapPoint, to: MapPoint) -> f32 {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        (dx * dx + dy * dy).sqrt()
    }

    fn distance_to_destination(fixture: &Fixture) -> f32 {
        distance(position(fixture), fixture.destination)
    }

    fn assert_forced_flags(fixture: &Fixture, force: bool) {
        let SequenceElementData::Movement {
            destination, flags, ..
        } = &movement_element(fixture).data
        else {
            panic!("movement fixture changed data kind")
        };
        assert_eq!(
            *destination, fixture.destination,
            "the authored destination must stay recorded on the element"
        );
        assert_eq!(
            flags.contains(MoveFlags::FORCE_SWORD_MOVEMENT),
            force,
            "the fixture must carry exactly the flag under test"
        );
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
        assert_forced_flags(fixture, force);
        assert_eq!(
            position(fixture),
            fixture.start,
            "the movement never reached motion processing"
        );
    }

    /// The perk: a movement carrying [`MoveFlags::FORCE_SWORD_MOVEMENT`]
    /// outranks the fight-entry stop, so the PC walks on through the fight
    /// and reaches the destination it was given.
    #[test]
    fn forced_pc_sword_move_keeps_advancing_through_an_enemy_initiated_swordfight() {
        let mut fixture = install_pc_sword_move(true);
        enemy_engages(&mut fixture);

        {
            let element = movement_element(&fixture);
            assert_eq!(
                element.state,
                SequenceState::InProgress,
                "the fight-entry stop leaves a forced sword movement running"
            );
            assert_eq!(
                element.orders.len(),
                1,
                "a kept movement keeps its WalkingWithSword order"
            );
        }
        assert_forced_flags(&fixture, true);

        let mut last_distance = distance_to_destination(&fixture);
        let mut advancing_frames = 0;
        let mut postponed_frames = 0;
        for _ in 0..64 {
            let state_before = movement_element(&fixture).state;
            frame(&mut fixture);
            assert_eq!(
                fixture.engine.human(fixture.owner).opponents.len(),
                1,
                "the enemy stays engaged for the whole walk"
            );
            let distance = distance_to_destination(&fixture);
            assert!(
                distance <= last_distance,
                "the forced movement never backs away from its destination \
                 ({last_distance} -> {distance})"
            );
            match state_before {
                SequenceState::InProgress => {
                    assert!(
                        distance < last_distance,
                        "a forced movement that is in progress advances on every frame \
                         while an opponent is engaged ({last_distance} -> {distance})"
                    );
                    advancing_frames += 1;
                }
                SequenceState::Postponed => {
                    // The same fight entry launches the PC's reciprocal
                    // `EnterSwordfight` at `PostponeEverythingButInjuries`, which
                    // queues the movement behind the raise-sword sequence for
                    // one frame. That is the priority ladder, not the stop.
                    postponed_frames += 1;
                }
                other => {
                    panic!("unexpected movement state {other:?} while walking through a fight")
                }
            }
            last_distance = distance;
            if distance == 0.0 {
                break;
            }
        }

        assert!(
            advancing_frames >= 5,
            "the movement must have walked on for several frames, got {advancing_frames}"
        );
        assert_eq!(
            postponed_frames, 1,
            "only the PC's own raise-sword entry postpones the movement, and only once"
        );
        assert_eq!(
            position(&fixture),
            fixture.destination,
            "the PC reaches the authored destination while still in the fight"
        );
        assert_eq!(
            movement_element(&fixture).state,
            SequenceState::Terminated,
            "the movement ends by arriving, not by the fight"
        );
    }

    /// The other direction of the same precedence: the forced movement ranks
    /// above `Preference` (fight entry) and below `Injury`. A hit or knockout
    /// stops an injured or unconscious PC from walking on.
    #[test]
    fn forced_pc_sword_move_is_still_interrupted_by_an_injury_stop() {
        let mut fixture = install_pc_sword_move(true);
        enemy_engages(&mut fixture);
        for _ in 0..3 {
            frame(&mut fixture);
        }
        assert_eq!(
            movement_element(&fixture).state,
            SequenceState::InProgress,
            "the forced movement is walking before the hit lands"
        );
        let before_hit = position(&fixture);
        assert!(
            distance(before_hit, fixture.destination)
                < distance(fixture.start, fixture.destination),
            "the forced movement made progress before the hit"
        );

        // `Injury` is the level a hit/knockout teardown stops the victim at
        // (`ReceiveDamage` elements carry it); the fight-entry stop is weaker.
        let assets = assets_with_test_pc_profile();
        let sim = crate::sim_rng::test_context();
        fixture.engine.stop_actor_orders(
            TickCtx::new(&sim, &assets),
            &mut Vec::new(),
            fixture.owner,
            SequencePriority::Injury,
        );

        assert_eq!(
            movement_element(&fixture).state,
            SequenceState::Interrupted,
            "FORCE_SWORD_MOVEMENT ranks below Injury: a hit or knockout still \
             interrupts the forced movement"
        );
        assert!(
            movement_element(&fixture).orders.is_empty(),
            "an interrupted forced movement keeps no orders to resume from"
        );

        for _ in 0..4 {
            frame(&mut fixture);
        }
        assert_eq!(
            position(&fixture),
            before_hit,
            "an injured PC never advances again, however the fight ends"
        );
        assert_eq!(
            fixture.engine.human(fixture.owner).opponents.len(),
            1,
            "the fight is still live, so only the injury interrupted the move"
        );
    }

    #[test]
    fn unforced_pc_sword_move_is_destroyed_by_an_enemy_initiated_swordfight() {
        let mut fixture = install_pc_sword_move(false);
        enemy_engages(&mut fixture);

        assert_destroyed_at_entry(&fixture, false);

        frame(&mut fixture);
        frame(&mut fixture);
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
        for _ in 0..6 {
            frame(&mut fixture);
        }

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

    /// The perk measured from the other side: the PC starts the fight. The
    /// enemy-initiated case pins the ranking that lets the flag outrank
    /// `stop_owner_current(Preference)`; here there is no stop to outrank,
    /// because swordfight preparation and its stop are scoped to the
    /// opponent. The forced movement must advance on every `InProgress`
    /// frame, never be postponed, and arrive.
    #[test]
    fn forced_pc_sword_move_advances_through_a_swordfight_the_pc_initiated() {
        let mut fixture = install_pc_sword_move(true);
        let enemy = pc_initiates(&mut fixture);

        {
            let element = movement_element(&fixture);
            assert_eq!(
                element.state,
                SequenceState::InProgress,
                "no step of the PC-initiated entry touches the initiator's own movement"
            );
            assert_eq!(
                element.orders.len(),
                1,
                "the movement keeps its WalkingWithSword order"
            );
        }
        assert_forced_flags(&fixture, true);

        let mut last_distance = distance_to_destination(&fixture);
        let mut advancing_frames = 0;
        let mut postponed_frames = 0;
        for _ in 0..64 {
            let state_before = movement_element(&fixture).state;
            frame(&mut fixture);
            assert_eq!(
                fixture.engine.human(fixture.owner).opponents.len(),
                1,
                "the initiating PC stays in the fight for the whole walk"
            );
            assert_eq!(
                fixture.engine.human(enemy).opponents.len(),
                1,
                "the engaged enemy stays in the fight while the PC walks on"
            );
            let distance = distance_to_destination(&fixture);
            assert!(
                distance <= last_distance,
                "the movement never backs away from its destination \
                 ({last_distance} -> {distance})"
            );
            match state_before {
                SequenceState::InProgress => {
                    assert!(
                        distance < last_distance,
                        "an initiating PC's movement advances on every frame it is in \
                         progress while the enemy is engaged ({last_distance} -> {distance})"
                    );
                    advancing_frames += 1;
                }
                SequenceState::Postponed => {
                    // Nothing on the initiator's own owner queue competes with
                    // its movement: the reciprocal `EnterSwordfight` this entry
                    // launches belongs to the enemy.
                    postponed_frames += 1;
                }
                other => {
                    panic!("unexpected movement state {other:?} while walking through a fight")
                }
            }
            last_distance = distance;
            if distance == 0.0 {
                break;
            }
        }

        assert!(
            advancing_frames >= 5,
            "the movement must walk on for several frames, got {advancing_frames}"
        );
        assert_eq!(
            postponed_frames, 0,
            "an initiating PC's movement is never postponed by the fight it starts"
        );
        assert_eq!(
            position(&fixture),
            fixture.destination,
            "the PC reaches the authored destination while still in the fight it started"
        );
        assert_eq!(
            movement_element(&fixture).state,
            SequenceState::Terminated,
            "the movement ends by arriving, not by the fight"
        );
    }
}
