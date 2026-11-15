//! The "no friendly-fire damage" perk.
//!
//! The perk gates one thing: the life-point assignment a flagged PC's strike
//! performs on a victim the mission does not treat as hostile.  Everything the
//! hit does besides costing life points — protection draws, the impact sound,
//! the victim's hit reaction, any push — must stay byte-identical, because a
//! second perk will suppress friendly-fire knockback on the same hit.

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
