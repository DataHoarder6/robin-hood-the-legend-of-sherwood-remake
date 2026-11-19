//! The game-state half of a voluntary bug report.
//!
//! A report is requested from a surface a player can reach — the pause menu,
//! `/bug`, `POST /bug-report` — and is written once per frame boundary here,
//! where the whole live mission is borrowable: the engine that just ticked, the
//! host's modal stack, the timeline and the transport. Nothing here mutates the
//! mission.
//!
//! What could not be gathered is named in `notes` (or as a fact with
//! `"available": false`) instead of being left out: a report that quietly omits
//! a section reads as "there was nothing there". The bundle layout is
//! documented in `docs/BUG_REPORTS.md`.

use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::player_command::PlayerId;
use serde_json::{Value, json};

use super::interactive::InteractiveFrontend;
use super::runtime::{MissionFrame, MissionMutation, MissionWorld, TimelineRuntime};

/// Write every queued report against one snapshot of the current frame.
pub(super) fn drain_requested(
    timeline: &TimelineRuntime,
    world: &mut MissionWorld,
    frontend: &mut InteractiveFrontend,
    frame: &MissionFrame,
    save_manager: &crate::savegame::SaveGameManager,
) {
    let requests = crate::bug_bundle::take_requests();
    if requests.is_empty() {
        return;
    }
    let snapshot = snapshot(timeline, world, frontend, frame, save_manager);
    for request in requests {
        crate::bug_bundle::write(request, snapshot.clone());
    }
}

/// The game-state document one bundle carries.
fn snapshot(
    timeline: &TimelineRuntime,
    world: &mut MissionWorld,
    frontend: &mut InteractiveFrontend,
    frame: &MissionFrame,
    save_manager: &crate::savegame::SaveGameManager,
) -> Value {
    let MissionMutation {
        host,
        game,
        manager,
        assets,
        ..
    } = world.mutation();
    let engine = &manager.engine;
    let ui = &frontend.ui;

    json!({
        "schema_version": 1,
        "fingerprint": fingerprint(engine, assets),
        "build": build(),
        "machine": machine(),
        "session": session(timeline, host, engine, assets, save_manager),
        "game": {
            "operation": game.operation.get_current().wire_name(),
            "operation_previous": game.operation.get_previous().wire_name(),
            "message": {"text": game.message_text, "delay": game.message_delay},
            "mission": serde_json::to_value(engine.mission()).ok(),
            "script": engine
                .mission_script()
                .map(robin_engine::engine::MissionScript::diagnostic_state),
            "presentation": presentation(ui),
            "terminal": terminal(ui),
            "staged": {
                "modal_dismissals": frame.modal_dismissals,
                "post_commands": frame.post_commands(),
                "pending_modal_kinds": host.effects.pending_modal_kinds(),
                "pending_post_external_actions": frame.post_external_actions().len(),
            },
        },
        "world": world_state(engine),
        "paths": {
            "save_directory": save_directory(host),
            "replay": crate::bug_report::replay_path().map(|path| path.display().to_string()),
            "install_home": crate::install_home::root().map(|root| root.display().to_string()),
        },
        "notes": Vec::<String>::new(),
    })
}

fn build() -> Value {
    let contract = crate::runtime_contract::RuntimeContract::current();
    let contract_json = serde_json::to_vec(&contract).expect("the runtime contract serializes");
    json!({
        "version": crate::crash::version(),
        "git_rev": crate::replay_format::ENGINE_SOURCE_COMMIT,
        "target": env!("ROBIN_BUILD_TARGET"),
        "profile": env!("ROBIN_BUILD_PROFILE"),
        "release": env!("ROBIN_BUILD_PROFILE") == "release",
        "features": env!("ROBIN_BUILD_FEATURES"),
        "replay_schema_version": robin_engine::replay::REPLAY_SCHEMA_VERSION,
        "net_protocol_version": robin_engine::multiplayer::NET_PROTOCOL_VERSION,
        "runtime_contract": contract,
        "runtime_contract_hash": sha256_hex(&contract_json),
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// Facts about the machine only as far as they explain a symptom: which GPU,
/// driver and OS rendered the frame, and how many threads the simulation had.
fn machine() -> Value {
    let process = crate::diagnostic_context::attachment()
        .ok()
        .and_then(|attachment| serde_json::from_str::<Value>(&attachment.content).ok());
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "cpu_count": std::thread::available_parallelism().map(|count| count.get()).ok(),
        "process": process,
    })
}

fn session(
    timeline: &TimelineRuntime,
    host: &crate::host::Host,
    engine: &Engine,
    assets: &LevelAssets,
    save_manager: &crate::savegame::SaveGameManager,
) -> Value {
    let net = host.transport.net();
    let local_seat = host.transport.local_seat();
    let kind = match net {
        None => "single",
        Some(_) if local_seat == PlayerId::HOST => "host",
        Some(_) => "client",
    };
    let session_id = net
        .and_then(|net| net.session_id().ok())
        .map(|id| hex::encode(id.0));
    let profile = mission_profile(engine, assets);
    let campaign_slot = save_manager.find_resume_target().and_then(|index| {
        save_manager
            .slot_name(index)
            .ok()
            .map(|name| name.as_str().to_owned())
    });
    json!({
        "kind": kind,
        "session_id": session_id,
        "transport": {
            "local_seat": local_seat.0,
            "reconnecting": host.transport.reconnecting(),
            "expected_players": host
                .transport
                .mission_sim_config()
                .map(|config| config.coop.players),
            "mission_id": host.transport.mission_id(),
        },
        "mission": {
            "profile_id": profile.map(|profile| profile.id),
            "title": profile.map(|profile| profile.mission_name.clone()),
            "filename": profile.map(|profile| profile.mission_filename.clone()),
            "map": engine.mission_map_name(),
            "campaign_slot": campaign_slot,
            "campaign_slot_source": "the save index's resume target; the whole save directory is in the bundle",
        },
        "tick": {
            "timeline_frame": timeline.frame_number(),
            "simulation_tick": engine.simulation_tick().number(),
            "engine_frame_counter": engine.frame_counter(),
        },
        "players": players(engine),
        "desync": {
            "current_state_hash": format!("{:016x}", robin_engine::replay::state_hash(engine)),
            "last_compared_hash_frame": timeline.network().last_compared_hash_frame(),
            "desync_count": timeline.network().desync_count(),
            "replay_playback": timeline.replay().playback().is_some(),
        },
    })
}

/// One entry per seat, including an empty seat, so a reader can tell "the
/// fourth player never joined" from "there are three players".
fn players(engine: &Engine) -> Vec<Value> {
    engine
        .seats()
        .iter()
        .enumerate()
        .map(|(index, seat)| {
            json!({
                "seat": index,
                "connected": seat.is_active(index),
                "nickname": seat.nickname,
                "assigned_character": seat.assigned_character,
                "selection": seat.selection,
                "build_rev": {
                    "available": false,
                    "reason": "the wire cannot carry it: NetMsg::Hello reports protocol_version and nickname only, and NetMsg::Welcome adds no build identity; adding a field is a wire change",
                },
            })
        })
        .collect()
}

fn mission_profile<'a>(
    engine: &Engine,
    assets: &'a LevelAssets,
) -> Option<&'a robin_engine::profiles::MissionProfile> {
    let campaign = engine.campaign();
    campaign
        .current_mission_idx
        .and_then(|index| campaign.missions.get(index))
        .map(|mission| mission.profile(&assets.profile_manager))
}

fn save_directory(host: &crate::host::Host) -> Option<String> {
    host.application_context()
        .with_active_profile(|profile| {
            crate::save_file::save_directory_for_profile(profile.id)
                .map(|directory| directory.display().to_string())
        })
        .ok()
        .flatten()
}

/// What the player is looking at, and everything queued to change it.
fn presentation(ui: &super::interactive::MissionUi) -> Value {
    let modal = ui.active_modal.as_ref().map(|modal| {
        json!({
            "kind": modal_kind(modal),
            "pending_story_kinds": modal.pending_story_kinds(),
        })
    });
    json!({
        "pause_menu": ui.pause_menu.is_some(),
        "ui_task": ui.active_ui_task.as_ref().map(|task| format!("{:?}", task.kind())),
        "active_modal": modal,
        "console_visible": ui.console_overlay.is_visible(),
    })
}

fn modal_kind(modal: &super::modal_state::ActiveModal) -> &'static str {
    use super::modal_state::ActiveModal;
    match modal {
        ActiveModal::Dialogue(_) => "dialogue",
        ActiveModal::PopupScroll(_) => "popup_scroll",
        ActiveModal::Debriefing(_) => "debriefing",
        ActiveModal::MissionState { .. } => "mission_state",
        ActiveModal::Trading(_) => "trading",
    }
}

/// The mission-end page, the decision order it validates against, and the one
/// fact this module is not allowed to read.
fn terminal(ui: &super::interactive::MissionUi) -> Value {
    let page = ui.terminal_debriefing.as_ref();
    let decision_order = page.map(|page| {
        let current = page.current_kind();
        json!({
            "current_kind": current,
            "stage": if current.is_some() { "FinalDebriefing" } else { "Settled" },
        })
    });
    json!({
        "active": page.is_some(),
        "presentation": page.and_then(|page| page.presentation()),
        "decision_order": decision_order,
        "awaiting_snapshot": page.map(|page| page.awaiting_snapshot()),
        "pending_campaign_update": ui.pending_terminal_debriefing.is_some(),
        "phase": {
            "available": false,
            "reason": "TerminalDebriefingState::phase and its enum are private to game_session/terminal_debriefing.rs, which this change may not edit; decision_order and presentation are the public halves",
        },
    })
}

/// Every player character with the position the "standing on the sign" class
/// of report needs, and the counts a reader compares against.
fn world_state(engine: &Engine) -> Value {
    let characters: Vec<Value> = engine
        .pc_ids()
        .iter()
        .filter_map(|id| engine.get_entity(*id).map(|entity| (*id, entity)))
        .map(|(id, entity)| {
            let element = entity.element_data();
            json!({
                "entity": id,
                "position_map": element.position_map(),
                "position_3d": element.position(),
                "layer": element.layer(),
                "direction": element.direction(),
                "active": entity.is_active(),
                "pc": entity.pc_data().map(|pc| json!({
                    "life_points": pc.life_points,
                    "immortal": pc.immortal,
                    "playable": pc.playable,
                    "current_action": pc.current_action,
                    "list_index": pc.list_index,
                })),
                "actor": entity.actor_data().map(|actor| json!({
                    "action_state": actor.action_state,
                })),
            })
        })
        .collect();
    let alive = characters
        .iter()
        .filter(|character| {
            character["pc"]["life_points"]
                .as_i64()
                .is_some_and(|life| life > 0)
        })
        .count();
    let riders = engine
        .entities_iter()
        .filter(|entity| entity.soldier_data().is_some_and(|soldier| soldier.rider))
        .count();
    json!({
        "player_characters": characters,
        "alive": {
            "count": alive,
            "predicate": "player characters with life_points > 0",
        },
        "companions": "every player character is listed above; a companion is one more entry",
        "horses": {
            "count": riders,
            "note": "a horse is a flag on a mounted soldier, not a separate entity",
        },
        "selected": {
            "host": engine.hero_selection(PlayerId::HOST),
        },
    })
}

/// One issue per (mission, script state, build): the receiver's dedupe key for
/// repeated reports. The tick is deliberately excluded — two seats stuck in the
/// same state at different ticks are one defect, and the session id ties them.
fn fingerprint(engine: &Engine, assets: &LevelAssets) -> String {
    let mut material = String::new();
    if let Some(profile) = mission_profile(engine, assets) {
        material.push_str(&profile.mission_filename);
    }
    material.push('\0');
    if let Some(script) = engine.mission_script() {
        let state = script.diagnostic_state();
        material.push_str(&serde_json::to_string(&state).unwrap_or_default());
    }
    material.push('\0');
    material.push_str(crate::crash::version());
    sha256_hex(material.as_bytes())[..16].to_owned()
}
