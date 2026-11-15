# Campaign purchases and per-PC state

What exists today for "spend campaign money on a PC", and where each piece
lives. Anchors are symbols, not line numbers, so they survive edits; every one
resolves in the tree.

Last verified: 2026-11-15 against `b324e09f0`.

## Money

One pool: `CampaignValue::Ransom`, read and written through
`Campaign::get_value` / `add_value` / `set_value` (`crates/robin_engine/src/campaign.rs`).
Mission income and every picked-up coin land in the same counter —
`MissionDomain::add_campaign_value` credits `Ransom` and
`mission_stat.collected_money` together (`crates/robin_engine/src/engine/state/mission.rs`).

There is exactly one money-out purchase: `Campaign::buy_blazon`, priced by
`Mission::get_blazon_price` and inflated per profile by
`increase_blazon_price` (`crates/robin_engine/src/mission.rs`). It mutates only
`Campaign` and **does not check affordability** — the check lives in the UI
predicate `can_convert_money_to_blazons`, read from
`crates/robin_rs/src/ui_screens.rs`. New purchase code must check the balance in
the authoritative handler instead of copying that split.

Plumbing for a paused-campaign purchase: `ExternalAction::CampaignBuyBlazon`,
dispatched in `crates/robin_engine/src/engine/rollback_safe.rs`, surfaced by the
confirmation modal `crates/robin_rs/src/ingame_menu/buy_blazons.rs`.

The only money-*in* panel is Sherwood trading: `PlayerCommand::CampaignSellProductionItem`
→ `sell_sherwood_production_item` (`engine/trading.rs`), priced from the static
`TRADE_ITEMS` table and joined to `Campaign::production_sectors`. It is
sell-only and the wrong vehicle for a purchase node.

## Per-PC campaign state

`Campaign::characters: Vec<PcDescription>` is the per-PC record; the index is
stable character identity. `PcDescription::status: PcStatus` holds life points,
coma, the two combat skills, the ammo counters and the Sherwood beam slot.
Anything else per-PC has to be a new `#[serde(default)]` field on `PcStatus`
(plus the hand-written `Default`), because the neighbouring stores do not fit:
`collected_relics` is a campaign-global list, `production_sectors` is per
production type, `CampaignDeeds` is global, and `CampaignValue::Custom1..20` is
20 global script scratch slots used by authored missions.

Campaign state reaches a mission in one place: `PcData::from_spawn_status`
(`crates/robin_engine/src/element/pc.rs`) copies life points and the ammo
counters at spawn, called from `add_beam_me_pc_entity`
(`engine/level_loading/pcs.rs`). Skills are read live from the campaign copy
instead (`apply_production_sector_data` in `engine/level_loading.rs`). No
`PlayerCommand` is constructed at mission start and there is no first-frame
command stage.

The engine-side perk flags already exist and are mission-scoped:
`PcData::keep_moving_in_combat` / `PcData::auto_loot`, set by
`PlayerCommand::SetKeepMovingInCombat` / `SetAutoLoot` and applied by
`set_keep_moving_in_combat` / `set_auto_loot` in `engine/commands.rs`. Nothing
connects them to campaign state.

Hazard: missions with an `authored_spawn_roster` replace the campaign
description wholesale — `authored_beam_me_spawn_plan` (`engine/level_loading/pcs.rs`)
assigns a fresh `PcDescription` seeded from `PcStatus::from_profile`, which
erases experience, ammo and any new per-PC field.

## Save compatibility

- Old saves: `#[serde(default)]` on the new field plus its `Default` entry.
  `SAVE_FORMAT_VERSION` does not move for an additive field.
- The two exhaustive legacy-adoption literals must be updated (the compiler
  forces both): `LegacyPcStatusAdoption::to_rust`
  (`engine/legacy_save/campaign.rs`) and `pc_status`
  (`engine/legacy_save/adopt_pc_human.rs`).
- `PcStatus` is state-hashed and bitcode-snapshotted inside `Campaign`, so a new
  field bumps `REPLAY_SCHEMA_VERSION`, `NET_PROTOCOL_VERSION` and
  `NATIVE_SNAPSHOT_VERSION` (`engine/replay.rs`, `engine/multiplayer.rs`,
  `engine/snapshot.rs`).
- Never add a `CampaignValue` variant: `CampaignValues::from_wire` asserts the
  wire length matches the enum, and the legacy decoder hardcodes
  `CAMPAIGN_VALUE_COUNT` (`crates/robin_legacy_save/src/campaign.rs`).

## Not present, would have to be built

- A per-PC perk or unlock field in campaign state.
- A campaign→mission command seed.
- A screen that lists purchasable nodes: outside a mission only the campaign map
  and the mission-description modal exist, and the Sherwood HUD has five fixed
  buttons (`crates/robin_rs/src/sherwood_hud.rs`).
- A generic campaign purchase action, and a price table other than blazons /
  `TRADE_ITEMS`.
- Preservation of per-PC campaign fields across authored-roster missions.

See also: [`SAVE_FORMAT.md`](SAVE_FORMAT.md),
[`SHERWOOD_TRADING.md`](SHERWOOD_TRADING.md).
