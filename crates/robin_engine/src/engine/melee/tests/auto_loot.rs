use super::*;

/// Run one knock-out tail as if `looter` had put `victim` down.
fn knock_out(engine: &mut EngineInner, assets: &LevelAssets, victim: EntityId, looter: EntityId) {
    engine.apply_knockout_side_effects(
        TickCtx::new(&crate::sim_rng::test_context(), assets),
        victim,
        true,
        Some(looter),
        true,
    );
}

/// The party's mission income: the campaign purse the HUD shows and the
/// per-mission collected total it feeds.  A walking PC's pickup moves both.
fn mission_money(engine: &EngineInner) -> (i32, u32) {
    (
        engine
            .mission_domain
            .campaign
            .get_value(crate::campaign::CampaignValue::Ransom),
        engine.mission_domain.mission_stat.collected_money,
    )
}

fn loot_fixture() -> (EngineInner, LevelAssets, EntityId, EntityId) {
    let mut engine = make_engine();
    let looter = engine.add_test_entity(make_pc(wp(0.0, 0.0), None));
    let victim = engine.add_test_entity(make_soldier(wp(10.0, 0.0), None));
    (
        engine,
        action_test_assets([crate::profiles::Action::NoAction; 3]),
        looter,
        victim,
    )
}

#[test]
fn auto_loot_moves_a_knocked_out_victims_money_to_the_looter() {
    const CARRIED: u32 = 37;
    let (mut engine, assets, looter, victim) = loot_fixture();
    engine
        .ent_mut(victim)
        .npc_data_mut()
        .expect("soldier fixture")
        .money = CARRIED;
    engine.set_auto_loot(&[looter], true);
    let (purse_before, collected_before) = mission_money(&engine);

    knock_out(&mut engine, &assets, victim, looter);

    assert_eq!(
        engine
            .ent(victim)
            .npc_data()
            .expect("soldier fixture")
            .money,
        0,
        "the downed soldier is stripped"
    );
    assert_eq!(
        mission_money(&engine),
        (purse_before + CARRIED as i32, collected_before + CARRIED),
        "the money joins the same purse and mission total a walking pickup reaches"
    );
}

#[test]
fn auto_loot_off_leaves_the_victims_money_alone() {
    const CARRIED: u32 = 37;
    let (mut engine, assets, looter, victim) = loot_fixture();
    engine
        .ent_mut(victim)
        .npc_data_mut()
        .expect("soldier fixture")
        .money = CARRIED;
    let (purse_before, collected_before) = mission_money(&engine);

    knock_out(&mut engine, &assets, victim, looter);

    assert!(!engine.ent(looter).pc_data().expect("PC fixture").auto_loot);
    assert_eq!(
        engine
            .ent(victim)
            .npc_data()
            .expect("soldier fixture")
            .money,
        CARRIED,
        "without the perk the body still carries its purse"
    );
    assert_eq!(
        mission_money(&engine),
        (purse_before, collected_before),
        "without the perk mission income does not move"
    );
}

#[test]
fn a_repeated_knockout_tail_does_not_loot_the_same_victim_twice() {
    const CARRIED: u32 = 37;
    let (mut engine, assets, looter, victim) = loot_fixture();
    engine
        .ent_mut(victim)
        .npc_data_mut()
        .expect("soldier fixture")
        .money = CARRIED;
    engine.set_auto_loot(&[looter], true);
    let (purse_before, collected_before) = mission_money(&engine);

    knock_out(&mut engine, &assets, victim, looter);
    let after_first = mission_money(&engine);
    assert_eq!(
        after_first,
        (purse_before + CARRIED as i32, collected_before + CARRIED)
    );

    knock_out(&mut engine, &assets, victim, looter);

    assert_eq!(
        mission_money(&engine),
        after_first,
        "a re-entered tail must not credit the same purse again"
    );
    assert_eq!(
        engine
            .ent(victim)
            .npc_data()
            .expect("soldier fixture")
            .money,
        0
    );
}
