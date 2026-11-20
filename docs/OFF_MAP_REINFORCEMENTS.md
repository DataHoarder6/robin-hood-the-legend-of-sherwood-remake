# Off-map reinforcements

Status: **all eight work items landed.** `SimConfig::enable_offmap_reinforcements`,
staged garrison bodies, `EngineInner::summon_garrison`, authored entry kinds,
the shared `place_outside_entity`, the per-mission budget, the `AlertSoldiers`
emitter, weakest-first rank selection, withdrawal on alarm decay and the
`SummonReinforcements` native exist. Every decision was settled in a grilling
session on 2026-11-19.

## What this is

Actors arrive in a level from outside it: an officer whose alarm finds no
soldiers within reach calls for the garrison, a mission script sends the
sheriff's men through the gate, the player calls a replacement from the gang.
One event, three callers, two factions.

This is original-game behaviour, not an invention. The `S02_Lei_MP` walkthrough
records a trio of guards appearing at the main gate once Will Scarlett is freed,
and the review corpus records the general rule that an alarm brings low-ranking
soldiers first and higher ranks after.

## What already exists

- Reinforcement points are authored mission data: `RawReinforcementPoint`
  (`crates/robin_level_data/src/level_data.rs`) inside `RawTacticData`, each
  carrying position, direction, action, obstacle, sector and layer.
  `install_reinforcement_doors_stage`
  (`crates/robin_engine/src/engine/level_loading/finish.rs`) turns them into
  `DoorType::Reinforcement` doors (`crates/robin_engine/src/gate.rs`) and caches
  `ReinforcementDoorInfo` in `ai.global.reinforcement_doors` (`ai/contexts.rs`).
- `create_reinforcement` (`crates/robin_engine/src/engine/reinforcement.rs`)
  draws an un-instanced peasant from the gang, places it at the door's outer
  point — outside the map — and walks it in with `PassDoor` then `MOVE`. It is
  PC-only and crate-private. Callers: the `ALARM` console command and
  `pc.time_till_reinforcement` expiry.
- `MSG_SEND_REINFORCEMENT` is the player-facing path: jingle, a cooldown on the
  PC, then the same draw.
- `RecordEnterGame` (`crates/robin_engine/src/natives/dispatch/sequences.rs`)
  teleports an actor to a point just outside the map edge opposite its facing
  and walks it to a destination. This is the only existing way an actor enters
  from outside without a door, and it lives inside the mission-script native.
- Alert escalation: Green/Yellow/Red, `Decision::AlertSoldiers` /
  `RunAndAlertSoldiers`, radius and count constants in
  `crates/robin_engine/src/ai_enemy/util.rs`, recruitment in
  `crates/robin_engine/src/engine/ai/alert_execution.rs`.
- A merry man who leaves through a reinforcement door is frozen and marked
  inactive (`crates/robin_engine/src/engine/ai/panic_execution.rs`); the slot
  survives, the simulation does not touch it.

What does not exist: any spawn that is not a PC from the gang, any entry that is
not a door, any budget, any staging that is free of per-tick cost, and any
script access (no spawn native among the original 265 or the port's 8
extensions).

## Mechanism

One event: `Summon { faction, tier, count, entry_hint }`.

Emitters:

- The NPC alert path. `AlertSoldiers` recruits within radius first and emits the
  summon only when it comes up short. One escalation ladder, not two.
- The mission script, through one extension native
  `SummonReinforcements(faction, tier, count, entry)` registered beside the
  existing eight.
- The player command, by generalising `MSG_SEND_REINFORCEMENT` from
  `dead_pc: Option<EntityId>` to `(faction, tier, entry)`.

The player's call is their own gang. There is no UI path that summons soldiers;
the faction field exists so both sides share one implementation.

## Entry

`RawReinforcementPoint` has a `kind` (door, road, stairs, jetty) and an
optional in-map `target`. `ReinforcementEntryKind::Door` is the default for
legacy REIN data and the only kind that requires the out-of-map motion area;
the other kinds install as `ai.global.offmap_entries` instead of gates.

- Door entries use the existing `PassDoor` then `MOVE` path.
- Edge entries place the body at the authored outside point and walk straight
  to the target. `place_outside_entity` computes that point from the map
  bounds and is shared with the `RecordEnterGame` native through
  `EngineInner::enter_from_outside`. Pathing cannot cover this: `find_path`
  refuses a goal outside the grid and cannot link a source outside it, so
  entering from outside is a transition, never a route.

## Bodies and cost

Roster-based, per faction. Staged bodies are parked with `active = false` — the
state persists, the slot is kept, and the tick cost is zero, because the engine
has no culling and no simulation distance: every active entity is simulated
every frame. `RawSoldier::staged` authors one; a staged body is left out of the
alert population and the mission statistics until a summon wakes it.

Per-mission budgets: a total, a concurrent cap on live summoned actors, and
separate pools for the player's gang and the enemy garrison. Summoned actors are
marked and drop no money and do not count toward ratings or kill/KO tallies; the
budget is the difficulty dial and must not also be a reward dial.

## Behaviour

A summoned actor takes the caller as `master`, inherits its goal (last known
player position or the door rally point), sweeps, and falls back to `Default` at
the entry point. Authored alert paths remain optional mission data.

When the alarm decays to Green, summoned actors walk back to a reinforcement
point and go inactive — the exit a merry man already uses.

Tiers: the alert level sets the floor, prior arrivals escalate within the alarm,
and the caller's rank sets the ceiling. Guardrails, all in mission data: total
budget, concurrent cap, weakest-first order, summon only while a living NPC has
recently seen the player, a cooldown between summons, and a hard stop below Red.

## The flag

`SimConfig::enable_offmap_reinforcements`, `#[serde(default =
"enabled_by_default")]` — on by default, off on request. `SimConfig`
(`crates/robin_engine/src/engine/global_options.rs`) is the deterministic
simulation config: serialised, state-hashed, copied into the engine before the
first tick. The precedent is `enable_unbinding`, a post-port interaction that is
on by default and has no launch flag; `from_options` hardcodes it, and so does
this. That one also has an in-game gameplay-settings entry; this flag
deliberately does not, because the value must be fixed before the first tick. It
is a `RankedSimulationConfigField`, so a ranked board pins it like every other
simulation-affecting setting.

One flag covers the whole feature. With it off, the level behaves exactly as it
does today: no summon event, no budget accounting, alarm escalation unchanged,
and the existing PC replacement plus the `ALARM` cheat keep working, because
those are original-game behaviour. `SummonReinforcements` stays registered and
fails explicitly when the flag is off, so the native count and mod validation do
not depend on session state.

There is no in-game menu entry and no mid-mission toggle: the value is fixed
before the first tick, which is what keeps replays and co-op peers in agreement.

## Work items

1. ~~`enter_from_outside` as an engine operation; `RecordEnterGame` becomes a
   caller~~ — `place_outside_entity` is the shared operation, called by both.
2. ~~Generalise the spawn path from a PC with an optional dead body to
   `(faction, entry)`~~ — `summon_garrison` wakes an authored staged body
   instead of creating one, so the faction is a lookup and the entry is a door.
   Tier selection stays with the caller.
3. ~~Entry kinds and optional in-map target on the authored point type~~ —
   `ReinforcementEntryKind` and `RawReinforcementPoint::target`.
4. ~~Budget accounting~~ — `RawTacticData::reinforcement_budget` authors the
   total, concurrent cap and cooldown; `OffmapReinforcementBudget::authorize`
   is the single gate, and a failed summon refunds its slot. The counter/stat
   publication a woken body needs is `publish_soldier_alert_count` and
   `record_soldier_in_level`. The summoned mark is `SoldierData::staged`,
   cleared on wake.
5. ~~`Decision` emission from `AlertSoldiers` when the in-radius recruit falls
   short~~ — `try_summon_offmap` calls when `alerted_us` is empty and the
   officer is not Green; the arriving body takes the officer as `master` and
   its last known position as its goal.
6. ~~Withdrawal on alarm decay~~ — `tick_offmap_withdrawals` sends calm-level
   bodies out through the nearest reinforcement door and frees their budget
   slot when they actually leave.
7. ~~The `SummonReinforcements` native~~ — extension native, dispatched through
   `EngineCommand::SummonReinforcements` to the shared summon path.
8. ~~`SimConfig` field, its `from_options` value, and its state-hash coverage.~~

## Open

- Implementation sequencing, and the first acceptance test. Whether the trio in
  `S02_Lei_MP` is a scripted spawn in the original mission data or an engine
  event decides whether that mission is the test or a synthetic level is.
- The legendary placement generator's reinforcement budgets
  (`crates/robin_engine/src/engine/level_loading/legendary_reinforcements.rs`)
  are a different thing: they size the authored roster at load time. Nothing
  here revives that disabled spawner.
