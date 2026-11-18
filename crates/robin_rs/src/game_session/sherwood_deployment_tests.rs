//! The speed a script records reaches the actor as its movement order.
//!
//! `sherwood.scb` records two movement orders for a returning PC:
//!
//! * `StartUp.SendToDeploymentZone` (class-relative address 51) answers the
//!   rail's start-up message 1000 with `ICONST tmp4 = 1` at byte offset 118 and
//!   `RecordMove(pc, point, tmp4)` at 119 — the run into the deployment zone.
//! * `Zone_de_deployement_8000002d.SendAway` (address 0), reached from the
//!   deployment zone's `EnterZone` while the mission team already fills the
//!   next mission's beam-me quota, records the same move with
//!   `ICONST tmp0 = 0` at byte offset 19 and `RecordMove(...)` at 20 — the walk
//!   back out.
//!
//! Replaying the rail's message against the retail mission pins both
//! directions at the actor instead of only inside the native: a recorded
//! style of 1 must install `RunningUpright` and a recorded style of 0 must
//! install `WalkingUpright`.
#![cfg(test)]

use std::sync::Arc;

use robin_engine::element::{ActionState, EntityId};
use robin_engine::engine::{
    Engine, EngineArgs, LevelAssets, LevelLoadArgs, SimConfig, SimulationFrameInput,
};
use robin_engine::order::OrderType;
use robin_engine::player_command::{PlayerCommand, PlayerInput};

/// Start-up message the Sherwood `GoToExit` control sends the script.
const GO_TO_EXIT_STARTUP_MESSAGE: i32 = 1000;
/// Frames the PC idles before the press; the run must not start earlier.
const IDLE_FRAMES: usize = 40;
/// Frames observed after the press: the run, the zone's answer, and the stop.
const OBSERVED_FRAMES: usize = 400;

/// One frame of the observed PC's movement state.
#[derive(Clone, Copy, Debug)]
struct FrameState {
    order: Option<OrderType>,
    action_state: ActionState,
}

fn sherwood_engine() -> (Engine, LevelAssets) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let data = robin_test_support::original_data::data_directory("");
    let (_campaign, profiles, context) =
        crate::main_entry::rust_init_with_roots(Some(&data), Some(root)).unwrap();
    let files = context.preparation_files().unwrap().clone();
    let mut bank_host = crate::Host::scratch(640.0, 480.0);
    bank_host
        .frontend
        .resources
        .frame_holder_before_publication_mut()
        .initialize_sprite_bank_with_files(".", &files)
        .unwrap();

    let mission = profiles
        .missions
        .iter()
        .position(|profile| profile.mission_filename.eq_ignore_ascii_case("Sherwood"))
        .expect("the retail profile catalog must name the Sherwood mission");
    let mut campaign = robin_engine::campaign::Campaign::from_profiles(
        &profiles,
        robin_engine::player_profile::DifficultyLevel::Medium,
    );
    campaign.current_mission_idx = Some(mission);
    // `GetNumberOfBeamMes` answers the retail default of 5 while no next
    // mission is chosen, so a five-man team already fills the quota: the
    // deployment zone walks the arriving PC back out instead of recruiting it.
    let team: Vec<usize> = (0..5).collect();
    campaign.gang_indices = team.clone();
    campaign.mission_team_indices = team;

    let mut assets = LevelAssets::new();
    assets.profile_manager = profiles.clone();
    assets.sprite_scriptor = Arc::new(robin_engine::sprite_script::SpriteScriptor::with_resources(
        Arc::new(robin_engine::sprite_script::MissionResourceEnvironment::from_files(&files)),
    ));
    assets.bank_signature = bank_host.frontend.resources.frame_holder().signature();
    let path = files.resolve_data_path("Data/Levels/Sherwood.scb").unwrap();
    let program = robin_engine::script_manager::ScriptProgram::from_scb(
        robin_assets::scb::parse_file(&path).unwrap(),
    )
    .unwrap();
    assets.scripts.mission_programs = Arc::new(std::collections::BTreeMap::from([(
        "Sherwood".to_owned(),
        Arc::new(program),
    )]));
    let mut text = robin_assets::resource_manager::ResourceManager::with_files(files.clone());
    text.attach_resource_file("Data/Text/Level.res").unwrap();
    (assets.peasant_firstnames, assets.peasant_surnames) =
        crate::game_session::load_peasant_name_pool(&mut text).unwrap();
    assets.fixed_vip_names = crate::game_session::load_fixed_vip_name_map(&mut text).unwrap();
    let loaded = robin_engine::engine::level_loading::load_mission_for_campaign_with_files(
        &campaign,
        &profiles,
        "Data/Levels",
        &mut |_| {},
        &files,
    )
    .unwrap();
    let engine = Engine::new(EngineArgs {
        campaign,
        level: LevelLoadArgs {
            assets: &mut assets,
            level_directory: "Data/Levels",
            progress: &mut |_| {},
            loaded,
            bg_pixel_dims: (4096.0, 4096.0),
        },
        ground_mark_sprite: None,
        titbit_row_frame_counts: Vec::new(),
        rng_seed: 0x5eed,
        original_rng_replay: None,
        sim_config: SimConfig {
            script_enabled: true,
            ..SimConfig::default()
        },
    })
    .unwrap();
    (engine, assets)
}

/// Select the PC the rail's message will move and report its movement state
/// for every frame before and after the press.
fn press_go_to_exit() -> (EntityId, Vec<FrameState>, Vec<FrameState>) {
    let (mut engine, assets) = sherwood_engine();
    let pc: EntityId = engine
        .pc_ids()
        .first()
        .copied()
        .expect("Sherwood must construct its PCs");
    engine
        .advance_frame_without_hash(
            &assets,
            SimulationFrameInput::from_player_inputs(vec![PlayerInput::host(
                PlayerCommand::SelectPc {
                    pc_id: pc,
                    append: false,
                },
            )])
            .with_post_initialize(true),
        )
        .unwrap();

    let observe = |engine: &Engine| FrameState {
        order: engine.actor_order_type(pc),
        action_state: engine
            .get_entity(pc)
            .and_then(|entity| entity.actor_data())
            .expect("the selected PC keeps actor data")
            .action_state,
    };
    let mut before = Vec::with_capacity(IDLE_FRAMES);
    for _ in 0..IDLE_FRAMES {
        before.push(observe(&engine));
        engine
            .advance_frame_without_hash(&assets, SimulationFrameInput::default())
            .unwrap();
    }

    engine
        .advance_frame_without_hash(
            &assets,
            SimulationFrameInput::from_player_inputs(vec![PlayerInput::host(
                PlayerCommand::DispatchStartupMessage {
                    msg: GO_TO_EXIT_STARTUP_MESSAGE,
                    arg1: 0,
                    arg2: 0,
                },
            )]),
        )
        .unwrap();
    let mut after = Vec::with_capacity(OBSERVED_FRAMES);
    for _ in 0..OBSERVED_FRAMES {
        after.push(observe(&engine));
        engine
            .advance_frame_without_hash(&assets, SimulationFrameInput::default())
            .unwrap();
    }
    (pc, before, after)
}

fn assert_never_moves(frames: &[FrameState], pc: EntityId, what: &str) {
    let moved: Vec<_> = frames
        .iter()
        .filter(|frame| {
            matches!(
                frame.order,
                Some(OrderType::WalkingUpright | OrderType::RunningUpright)
            )
        })
        .collect();
    assert!(
        moved.is_empty(),
        "{pc:?} must stand still {what}, observed {moved:?}"
    );
}

#[test]
#[ignore = "requires full game data via ROBINHOOD_DATA_DIR"]
fn the_deployment_message_records_a_running_move() {
    let (pc, before, after) = press_go_to_exit();
    assert_never_moves(&before, pc, "before the deployment press");

    let running: Vec<_> = after
        .iter()
        .filter(|frame| frame.order == Some(OrderType::RunningUpright))
        .collect();
    assert!(
        !running.is_empty(),
        "the script recorded RecordMove(..., 1); the actor must run, observed {:?}",
        after.iter().map(|frame| frame.order).collect::<Vec<_>>()
    );
    for frame in &running {
        assert_eq!(
            frame.action_state,
            ActionState::MovingFast,
            "a recorded speed 1 must move the actor fast, not walk"
        );
    }
    assert!(
        after
            .iter()
            .position(|frame| frame.order == Some(OrderType::RunningUpright))
            < after
                .iter()
                .position(|frame| frame.order == Some(OrderType::WalkingUpright)),
        "the deployment run must precede the zone's walk-back, observed {:?}",
        after.iter().map(|frame| frame.order).collect::<Vec<_>>()
    );
}

#[test]
#[ignore = "requires full game data via ROBINHOOD_DATA_DIR"]
fn the_full_deployment_zone_records_a_walking_move() {
    let (pc, before, after) = press_go_to_exit();
    assert_never_moves(&before, pc, "before the deployment press");

    let walked_out = after
        .iter()
        .position(|frame| frame.order == Some(OrderType::WalkingUpright))
        .unwrap_or_else(|| {
            panic!(
                "the deployment zone records RecordMove(..., 0) for an arriving PC; \
                 no walk followed the run, observed {:?}",
                after.iter().map(|frame| frame.order).collect::<Vec<_>>()
            )
        });
    assert!(
        after[..walked_out]
            .iter()
            .any(|frame| frame.order == Some(OrderType::RunningUpright)),
        "{pc:?} must run into the zone before the zone walks it out"
    );
    assert_eq!(
        after[walked_out].action_state,
        ActionState::Moving,
        "a recorded speed 0 must walk the actor, not run it"
    );
    assert!(
        after[walked_out..]
            .iter()
            .take_while(|frame| frame.order != Some(OrderType::WaitingUpright))
            .all(|frame| frame.action_state != ActionState::MovingFast),
        "the walk back out must not be re-accelerated to a run"
    );
}
