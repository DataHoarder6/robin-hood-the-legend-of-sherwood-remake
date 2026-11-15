//! The two friendly-fire perks.
//!
//! Each gates one thing on a hit the mission does not treat as hostile, and the
//! two are independent flags:
//!
//! - "no friendly-fire damage" (the first perk) gates the life-point assignment
//!   a flagged PC's strike performs.  Everything else the hit does — protection
//!   draws, the impact sound, the victim's hit reaction, any push — stays
//!   byte-identical.
//! - "no friendly-fire knockback" (this pair's second half) gates the push a
//!   flagged PC's push strike performs: the victim is not displaced, while the
//!   damage still lands unless the damage perk also suppressed it, and the
//!   reaction, its sound, the speech and the knock-out path run untouched.

use super::*;

/// Land one non-lethal sword hit (`SwordStrike::A`, cutting 1) and return the
/// damage element, so a test can inspect what the hit registered.
fn sword_hit(
    engine: &mut EngineInner,
    assets: &LevelAssets,
    attacker: EntityId,
    victim: EntityId,
) -> (crate::sequence::SequenceId, usize) {
    let sim = crate::sim_rng::test_context();
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence = engine.orders.sequence_manager.insert_element(damage);
    engine
        .orders
        .sequence_manager
        .start_sequence_level(sequence);
    engine.select_sequence_element(victim, Some((sequence, 0)));
    engine.t_element_in_progress(assets, sequence, 0);

    engine.apply_sword_damage(
        TickCtx::new(&sim, assets),
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence, 0),
    );
    (sequence, 0)
}

fn life_points(engine: &EngineInner, victim: EntityId) -> i16 {
    match engine.ent(victim) {
        Entity::Pc(pc) => pc.pc.life_points,
        Entity::Soldier(soldier) => soldier.npc.life_points,
        Entity::Civilian(civilian) => civilian.npc.life_points,
        other => panic!("fixture victim is not human: {:?}", other.kind()),
    }
}

/// Five starting life points would make the assertions read like arithmetic;
/// the fixtures start at 50 and one cutting point per hit.
#[test]
fn flagged_pc_sword_hit_costs_a_friendly_no_life_points() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(wp(0.0, 0.0), None));
    // A second party PC: same camp, so the mission treats the two as allied.
    let victim = engine.add_test_entity(make_pc(wp(10.0, 0.0), None));
    engine.set_no_friendly_fire_damage(&[attacker], true);
    let assets = assets_with_sword_profile_effects(1, 50, 1, 0);

    let (sequence, index) = sword_hit(&mut engine, &assets, attacker, victim);

    assert_eq!(
        life_points(&engine, victim),
        50,
        "a flagged PC's strike must cost a friendly victim no life points"
    );
    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence, index)
        .expect("the hit must not be dropped: the damage element stays registered");
    assert_eq!(
        element.orders.front().map(|order| order.order_type),
        Some(crate::order::OrderType::FallingBackUpright),
        "the hit still lands: the victim's hit reaction is authored on the element"
    );
}

#[test]
fn flagged_pc_sword_hit_still_wounds_an_enemy() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(wp(0.0, 0.0), None));
    let victim = engine.add_test_entity(make_soldier(wp(10.0, 0.0), None));
    engine.set_no_friendly_fire_damage(&[attacker], true);
    let assets = assets_with_sword_profile_effects(1, 50, 1, 0);

    sword_hit(&mut engine, &assets, attacker, victim);

    assert_eq!(
        life_points(&engine, victim),
        49,
        "an enemy is damaged exactly as before the perk existed"
    );
}

/// Characterisation of today's behaviour, kept so the perk cannot quietly
/// become unconditional friendly-fire immunity.
#[test]
fn unflagged_pc_sword_hit_still_wounds_a_friendly() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(wp(0.0, 0.0), None));
    let victim = engine.add_test_entity(make_pc(wp(10.0, 0.0), None));
    let assets = assets_with_sword_profile_effects(1, 50, 1, 0);

    sword_hit(&mut engine, &assets, attacker, victim);

    assert_eq!(
        life_points(&engine, victim),
        49,
        "without the perk a PC's strike still wounds its allies"
    );
}

/// The flag belongs to the attacker.  A non-PC attacker that strikes a flagged
/// PC is not suppressed, and neither is scripted damage that names no attacker.
#[test]
fn the_flag_only_suppresses_hits_the_flagged_pc_lands() {
    let mut engine = make_engine();
    let victim = engine.add_test_entity(make_pc(wp(10.0, 0.0), None));
    engine.set_no_friendly_fire_damage(&[victim], true);
    let ally_soldier = engine.add_test_entity(make_soldier(wp(0.0, 0.0), None));
    {
        // Same camp as the victim: an allied soldier, not an enemy.
        let Entity::Soldier(soldier) = engine.ent_mut(ally_soldier) else {
            unreachable!()
        };
        soldier.soldier.cached_camp = crate::element::Camp::Royalists;
    }
    let assets = assets_with_sword_profile_effects(1, 50, 1, 0);

    sword_hit(&mut engine, &assets, ally_soldier, victim);

    assert_eq!(
        life_points(&engine, victim),
        49,
        "the perk rides on the attacker, so a non-PC attacker is untouched"
    );
}

#[test]
fn scripted_sword_damage_is_untouched_by_the_perk() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(wp(0.0, 0.0), None));
    let victim = engine.add_test_entity(make_pc(wp(10.0, 0.0), None));
    engine.set_no_friendly_fire_damage(&[attacker], true);
    let assets = assets_with_sword_profile_effects(1, 50, 1, 0);

    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence = engine.orders.sequence_manager.insert_element(damage);
    engine
        .orders
        .sequence_manager
        .start_sequence_level(sequence);
    engine.select_sequence_element(victim, Some((sequence, 0)));
    engine.t_element_in_progress(&assets, sequence, 0);

    engine.apply_sword_damage(
        TickCtx::new(&sim, &assets),
        victim,
        // No attacker: scripted damage.  The flagged PC is only in the world.
        None,
        Some(SwordStrike::A),
        Some(1),
        (sequence, 0),
    );

    assert_eq!(
        life_points(&engine, victim),
        49,
        "damage that names no attacker must not be read as friendly fire"
    );
}

/// Land one non-lethal push strike (`SwordStrike::H`, true circle) and return
/// the damage element, so a test can inspect both the reaction it authored and
/// the flight it prepared.
fn push_hit(
    engine: &mut EngineInner,
    assets: &LevelAssets,
    attacker: EntityId,
    victim: EntityId,
) -> (crate::sequence::SequenceId, usize) {
    let sim = crate::sim_rng::test_context();
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::H, 1);
    let sequence = engine.orders.sequence_manager.insert_element(damage);
    engine
        .orders
        .sequence_manager
        .start_sequence_level(sequence);
    engine.select_sequence_element(victim, Some((sequence, 0)));
    engine.t_element_in_progress(assets, sequence, 0);

    engine.apply_sword_damage(
        TickCtx::new(&sim, assets),
        victim,
        Some(attacker),
        Some(SwordStrike::H),
        Some(1),
        (sequence, 0),
    );
    (sequence, 0)
}

/// The shared non-straight fixture is a lethal hundred-cutting true circle;
/// these perks decide what a *surviving* hit does, so make it a short,
/// non-lethal push that lands inside the authorized test grid.
fn assets_with_short_push_strike() -> LevelAssets {
    let mut assets = assets_with_nonstraight_profile(
        SwordStrike::H,
        crate::profiles::WeaponThrustKind::TrueCircle,
    );
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    let thrust = &mut profiles.hth_weapons[0].thrusts[SwordStrike::H as usize];
    thrust.energy = 1;
    thrust.cutting = 1;
    thrust.maximal_distance = 20;
    assets
}

/// The reaction the hit authored, asserted to be a pushed fall so a test that
/// only cares about displacement cannot pass on a dropped hit.
fn pushed_reaction(
    engine: &EngineInner,
    element: (crate::sequence::SequenceId, usize),
) -> crate::order::OrderType {
    let order = engine
        .orders
        .sequence_manager
        .get_element(element.0, element.1)
        .expect("the hit must not be dropped: the damage element stays registered")
        .orders
        .back()
        .expect("a push strike authors a falling reaction")
        .order_type;
    assert!(
        matches!(
            order,
            crate::order::OrderType::FallingPushedUpright
                | crate::order::OrderType::FallingPushedWithBow
                | crate::order::OrderType::FallingPushedWithSword
                | crate::order::OrderType::FallingPushedCrouched
        ),
        "the hit's reaction must be a pushed fall, got {order:?}"
    );
    order
}

/// Prepare takeoff for the reaction the hit authored and run the flight's
/// starting tick, returning the victim's position afterwards.
///
/// `melee/tests/state.rs` measures pushed displacement the same way: the
/// flight applies its first increment on the starting execution, so the
/// position after this tick is the displacement the blow is worth.
fn position_after_push(
    engine: &mut EngineInner,
    assets: &LevelAssets,
    victim: EntityId,
    element: (crate::sequence::SequenceId, usize),
) -> crate::coordinates::MapPoint {
    let reaction = pushed_reaction(engine, element);
    engine.elem_mut(victim).sprite.scripts =
        std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
            frame_ids: vec![0, 1],
            ..Default::default()
        }]);
    engine.initialize_push_flight(assets, victim, element, reaction);
    engine.perform_combat_flight_position(victim, crate::sprite::MotionState::InProgress);
    engine.map_pos_of(victim)
}

/// Knockback perk only: the blow costs the ally life points, but leaves them
/// exactly where they stood.
#[test]
fn knockback_perk_leaves_a_friendly_in_place_but_still_wounds_them() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(wp(0.0, 100.0), None));
    let victim = engine.add_test_entity(make_pc(wp(10.0, 100.0), None));
    authorize_test_hit_flight(&mut engine, victim);
    engine.set_no_friendly_fire_knockback(&[attacker], true);
    let assets = assets_with_short_push_strike();
    let position_before = engine.map_pos_of(victim);

    let element = push_hit(&mut engine, &assets, attacker, victim);
    let position_after = position_after_push(&mut engine, &assets, victim, element);

    assert_eq!(
        position_after, position_before,
        "a flagged PC's push strike must not displace a friendly victim"
    );
    assert_eq!(
        engine.ent(victim).position_iface().get_increment(),
        crate::coordinates::WorldVec3D::default(),
        "and the blow installs no flight velocity on that victim"
    );
    assert_eq!(
        life_points(&engine, victim),
        49,
        "the knockback perk must leave the damage of the same hit untouched"
    );
}

/// Both perks: no life points and no displacement, but the hit is still a hit.
#[test]
fn both_friendly_fire_perks_leave_a_friendly_unwounded_and_in_place() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(wp(0.0, 100.0), None));
    let victim = engine.add_test_entity(make_pc(wp(10.0, 100.0), None));
    authorize_test_hit_flight(&mut engine, victim);
    engine.set_no_friendly_fire_damage(&[attacker], true);
    engine.set_no_friendly_fire_knockback(&[attacker], true);
    let assets = assets_with_short_push_strike();
    let position_before = engine.map_pos_of(victim);

    let element = push_hit(&mut engine, &assets, attacker, victim);
    let position_after = position_after_push(&mut engine, &assets, victim, element);

    assert_eq!(
        life_points(&engine, victim),
        50,
        "the damage perk still owns the life points of this hit"
    );
    assert_eq!(
        position_after, position_before,
        "the knockback perk owns the displacement of the same hit"
    );
}

/// Damage perk only: no life points, but the ally is still knocked back.
#[test]
fn damage_perk_alone_still_knocks_a_friendly_back() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(wp(0.0, 100.0), None));
    let victim = engine.add_test_entity(make_pc(wp(10.0, 100.0), None));
    authorize_test_hit_flight(&mut engine, victim);
    engine.set_no_friendly_fire_damage(&[attacker], true);
    let assets = assets_with_short_push_strike();
    let position_before = engine.map_pos_of(victim);

    let element = push_hit(&mut engine, &assets, attacker, victim);
    let position_after = position_after_push(&mut engine, &assets, victim, element);

    assert_eq!(
        life_points(&engine, victim),
        50,
        "the damage perk must not need the knockback perk"
    );
    assert_ne!(
        position_after, position_before,
        "without the knockback perk the ally is still pushed"
    );
}

/// The perk is about friendliness, not about the push: an enemy is displaced
/// exactly as before.
#[test]
fn knockback_perk_still_knocks_an_enemy_back() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(wp(0.0, 100.0), None));
    let victim = engine.add_test_entity(make_soldier(wp(10.0, 100.0), None));
    authorize_test_hit_flight(&mut engine, victim);
    engine.set_no_friendly_fire_knockback(&[attacker], true);
    let assets = assets_with_short_push_strike();
    let position_before = engine.map_pos_of(victim);

    let element = push_hit(&mut engine, &assets, attacker, victim);
    let position_after = position_after_push(&mut engine, &assets, victim, element);

    assert_ne!(
        position_after, position_before,
        "a hostile victim is still knocked back by a flagged PC"
    );
    assert_eq!(
        life_points(&engine, victim),
        49,
        "and is still wounded by the same hit"
    );
}

/// Characterisation of today's behaviour, kept so the perk cannot quietly
/// become unconditional push immunity.
#[test]
fn unflagged_pc_still_knocks_a_friendly_back() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(wp(0.0, 100.0), None));
    let victim = engine.add_test_entity(make_pc(wp(10.0, 100.0), None));
    authorize_test_hit_flight(&mut engine, victim);
    let assets = assets_with_short_push_strike();
    let position_before = engine.map_pos_of(victim);

    let element = push_hit(&mut engine, &assets, attacker, victim);
    let position_after = position_after_push(&mut engine, &assets, victim, element);

    assert_ne!(
        position_after, position_before,
        "without the perk a PC still pushes its allies"
    );
    assert_eq!(life_points(&engine, victim), 49, "and still wounds them");
}
