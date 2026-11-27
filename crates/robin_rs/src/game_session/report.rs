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

/// The pointer sampled once at the frame boundary, in the space the HUD hit
/// tests use. Nothing here reads render state mid-frame: the window has already
/// converted the raw pointer to the logical game surface, and this copy is
/// passed down to the writer with the rest of the snapshot.
#[derive(Debug, Clone, Copy)]
pub(super) struct CursorSample {
    pub(super) x: f32,
    pub(super) y: f32,
    /// Logical game surface: the canvas the HUD layouts and hit tests share.
    pub(super) surface_w: u32,
    pub(super) surface_h: u32,
    /// Physical surface pixels per logical game pixel on the x axis.
    pub(super) scale: f32,
}

impl CursorSample {
    pub(super) fn from_window(window: &crate::window::GameWindow) -> Self {
        let (x, y) = window.cursor_pos();
        let (logical_w, logical_h) = window.logical_size();
        let (physical_w, _) = window.surface_size();
        Self {
            x: x as f32,
            y: y as f32,
            surface_w: logical_w,
            surface_h: logical_h,
            scale: physical_w as f32 / logical_w.max(1) as f32,
        }
    }

    fn into_json(self) -> Value {
        json!({
            "x": self.x,
            "y": self.y,
            "surface_w": self.surface_w,
            "surface_h": self.surface_h,
            "scale": self.scale,
        })
    }
}

/// The layers a cursor query probes, topmost first. Small controls are tested
/// before the larger rails and the portrait panel, matching the order in which
/// the HUD claims pixels; the first geometric hit wins.
const CURSOR_LAYERS: [&str; 6] = [
    "touch_plan_hud",
    "zoom_hud",
    "corner_hud",
    "stature_hud",
    "sherwood_hud",
    "ui_panel",
];

fn rect_tuple(rect: &crate::gfx_types::Rect) -> (i32, i32, i32, i32) {
    (
        rect.x(),
        rect.y(),
        rect.width() as i32,
        rect.height() as i32,
    )
}

fn element(
    id: &str,
    name: &str,
    layer: &str,
    rect: (i32, i32, i32, i32),
    label: String,
    label_source: &str,
) -> Value {
    json!({
        "id": id,
        "name": name,
        "parent_layer": layer,
        "rect": {"x": rect.0, "y": rect.1, "w": rect.2, "h": rect.3},
        "label": label,
        "label_source": label_source,
    })
}

/// Resolve the text the player actually sees for a HUD button: the same menu
/// table the tooltip renderer reads. A missing table is named through
/// `label_source`, never silently replaced.
fn tooltip(
    menu_text: Option<&crate::ingame_menu::resources::MenuText>,
    id: usize,
) -> Option<String> {
    let text = menu_text?.get(id);
    (!text.is_empty()).then_some(text)
}

/// Query the HUD layers under one sampled point with the widgets' own hit
/// tests. The first layer that claims the point names its element; a miss is
/// a positive record of every layer that was tested. The modal and mission-end
/// page come from the same frame-boundary helpers the rest of the snapshot uses.
fn cursor_element(
    sample: &CursorSample,
    is_sherwood: bool,
    men_to_blazon: bool,
    frontend: &InteractiveFrontend,
    num_pcs: usize,
) -> Value {
    use crate::corner_hud::CornerButton;
    use crate::sherwood_hud::SherwoodButton;
    use crate::stature_hud::StatureButton;
    use crate::zoom_hud::ZoomButton;

    let hud = &frontend.hud;
    let ui = &frontend.ui;
    let menu_text = frontend.resources.menu.as_ref().map(|menu| &menu.menu_text);
    let x = sample.x.round() as i32;
    let y = sample.y.round() as i32;
    let sw = sample.surface_w.min(u32::from(u16::MAX)) as u16;
    let sh = sample.surface_h.min(u32::from(u16::MAX)) as u16;
    let resolve = |id, name, layer, rect, mt_id: Option<usize>| match mt_id
        .and_then(|id| tooltip(menu_text, id))
    {
        Some(label) => element(id, name, layer, rect, label, "menu tooltip"),
        None => element(
            id,
            name,
            layer,
            rect,
            name.to_owned(),
            "name (menu text unavailable)",
        ),
    };

    let mut tested = Vec::new();
    let mut found = None;

    tested.push(CURSOR_LAYERS[0]);
    if crate::touch_plan_hud::hit_test(sw, x, y) {
        let (left, top, right, bottom) = crate::touch_plan_hud::rect(sw);
        found = Some(element(
            "touch_plan.plan",
            "Plan actions",
            CURSOR_LAYERS[0],
            (left, top, right - left, bottom - top),
            "PLAN ACTIONS".to_owned(),
            "control text",
        ));
    }

    if found.is_none() {
        tested.push(CURSOR_LAYERS[1]);
        if let Some(button) = hud.zoom_layout.hit_test_geometric(x, y) {
            let (id, name, rect, mt) = match button {
                ZoomButton::ZoomUp => (
                    "zoom.up",
                    "Zoom in",
                    &hud.zoom_layout.zoom_up,
                    crate::zoom_hud::zoom_button_tooltip_mt_id(button),
                ),
                ZoomButton::ZoomDown => (
                    "zoom.down",
                    "Zoom out",
                    &hud.zoom_layout.zoom_down,
                    crate::zoom_hud::zoom_button_tooltip_mt_id(button),
                ),
            };
            found = Some(resolve(
                id,
                name,
                CURSOR_LAYERS[1],
                rect_tuple(rect),
                Some(mt),
            ));
        }
    }

    if found.is_none() {
        tested.push(CURSOR_LAYERS[2]);
        if let Some(button) = hud.corner_layout.hit_test_geometric(x, y) {
            let (id, name, rect) = match button {
                CornerButton::Clock => (
                    "corner.clock",
                    "Record quick action",
                    &hud.corner_layout.clock,
                ),
                CornerButton::Sight => ("corner.sight", "View cone", &hud.corner_layout.sight),
                CornerButton::QuickStart => (
                    "corner.quickstart",
                    "Launch all quick actions",
                    &hud.corner_layout.quickstart,
                ),
            };
            found = Some(resolve(
                id,
                name,
                CURSOR_LAYERS[2],
                rect_tuple(rect),
                Some(crate::corner_hud::corner_button_tooltip_mt_id(button)),
            ));
        }
    }

    if found.is_none() {
        tested.push(CURSOR_LAYERS[3]);
        if let Some(button) = hud.stature_layout.hit_test_geometric(x, y) {
            let (id, name, rect) = match button {
                StatureButton::Up => ("stature.up", "Stand up", &hud.stature_layout.up),
                StatureButton::Down => ("stature.down", "Crouch", &hud.stature_layout.down),
            };
            found = Some(resolve(
                id,
                name,
                CURSOR_LAYERS[3],
                rect_tuple(rect),
                Some(crate::stature_hud::stature_button_tooltip_mt_id(button)),
            ));
        }
    }

    if found.is_none() {
        tested.push(CURSOR_LAYERS[4]);
        if let Some(button) = hud.sherwood_layout.hover_test(x, y, hud.sherwood_rail) {
            let (id, name) = match button {
                SherwoodButton::DisplayCampaignMap => ("sherwood.map", "Campaign map"),
                SherwoodButton::GoToExit => ("sherwood.exit", "Go to exit"),
                SherwoodButton::StartMission => ("sherwood.start_mission", "Start mission"),
                SherwoodButton::QuitMission => ("sherwood.quit_mission", "Quit mission"),
                SherwoodButton::SherwoodTrading => ("sherwood.trading", "Trading"),
                SherwoodButton::PerksPanel => ("sherwood.perks", "Perks"),
            };
            found = Some(resolve(
                id,
                name,
                CURSOR_LAYERS[4],
                rect_tuple(&hud.sherwood_layout.rect(button)),
                crate::sherwood_hud::sherwood_button_tooltip_mt_id(
                    button,
                    is_sherwood,
                    men_to_blazon,
                ),
            ));
        }
    }

    if found.is_none() {
        tested.push(CURSOR_LAYERS[5]);
        if let Some(slot) = crate::ui_panel::hit_test_portrait(sw, sh, sample.x, sample.y, num_pcs)
        {
            let name = format!("Portrait {slot}");
            found = Some(element(
                "ui_panel.portrait",
                &name,
                CURSOR_LAYERS[5],
                crate::ui_panel::portrait_slot_rect(sw, sh, slot, num_pcs),
                name.clone(),
                "element name",
            ));
        }
    }

    let (element, miss) = match found {
        Some(element) => (element, Value::Null),
        None => (
            Value::Null,
            json!({"reason": "no UI element under the cursor"}),
        ),
    };
    json!({
        "element": element,
        "miss": miss,
        "layers_tested": tested,
        "active_modal": presentation(ui)["active_modal"].clone(),
        "terminal_page": terminal(ui),
    })
}

/// Write every queued report against one snapshot of the current frame.
pub(super) fn drain_requested(
    timeline: &TimelineRuntime,
    world: &mut MissionWorld,
    frontend: &mut InteractiveFrontend,
    frame: &MissionFrame,
    save_manager: &crate::savegame::SaveGameManager,
    cursor: &CursorSample,
) {
    let requests = crate::bug_bundle::take_requests();
    if requests.is_empty() {
        return;
    }
    let snapshot = snapshot(timeline, world, frontend, frame, save_manager, cursor);
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
    cursor: &CursorSample,
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
    let cursor_query = cursor_element(
        cursor,
        game.is_sherwood,
        engine.is_men_to_blazon_conversion_mode(),
        frontend,
        engine.pc_ids().len(),
    );

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
        "cursor_sample": cursor.into_json(),
        "cursor_element": cursor_query,
        "notes": Vec::<String>::new(),
    })
}

fn build() -> Value {
    let contract = crate::runtime_contract::RuntimeContract::current();
    let contract_json = serde_json::to_vec(&contract).expect("the runtime contract serializes");
    json!({
        "version": crate::crash::version(),
        "git_rev": crate::replay_format::engine_source_commit(),
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
