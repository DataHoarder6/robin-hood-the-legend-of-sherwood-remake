# Demo mission loses at game second 0 (`Dem_Lei_MP`)

Evidence for [browser-seat.md §10.5](browser-seat.md#105-what-it-does-not-prove-the-mission-is-lost-on-its-first-tick):
a single native host, no browser, logs

```
DEBUG victory: CheckVictoryCondition game_seconds=0 result=Ok(2) mission_won=false
  blocked_by_combat=false pc_positions=[(Pc(PcId(198)), (209.0, 1881.0)),
  (Pc(PcId(81)), (184.0, 1039.0))]
INFO robin_engine::engine: EngineInner: mission quit
```

and `Engine tick returned: LevelFailed`. The session is from
`/home/admin/solo-victory-20261119-221832/host.log` on `minipc`.

## Verdict

**Bad test setup, not an engine defect and not a browser defect.** The host
launched the Demo mission with a fresh campaign whose gang is Robin alone
(`1 in gang`). The Demo's authored `StartUp.CheckVictoryCondition` requires a
spawned PC carrying the **Lockpick** action (script action `28`) while global
`200 == 0` — i.e. before Robin reaches the prison and frees Will. With a
Robin-only party no PC has Lockpick, so the script's own bytecode returns `2`
("mission lost") and the engine faithfully returns `LevelFailed`. The Demo
ships an auto-start path that builds the gang `RJMT` (Robin, Little John,
Marian, **Friar Tuck**); Friar Tuck has contextual Lockpick, so the script's
check passes there. The harness bypassed that path by forcing `--mission`.

## 1. Which script/native returns 2, and why at tick 0

`Dem_Lei_MP.scb`, class `StartUp`, function `CheckVictoryCondition`
(`addr=1438`, `params=2`, `ret=4`). Raw quads from `disasm_scb`:

```
1441  ICONST  tmp0 = 200
1442  tmp0 = GetGlobal(tmp0)                         // native #2
1445  ICONST  tmp4 = 0
1446  IEQ     tmp8 = tmp0 == tmp4
1447  IFNZ    tmp8 goto @1449
1449  ICONST  tmp0 = 28
1450  tmp0 = HasAnyPCActionWhoIsInThisLevelOrCouldMaybeComeFromSherwood(tmp0) // native #253
1453  ICONST  tmp8 = 0
1454  IEQ     tmp4 = tmp0 == tmp8
1455  IFNZ    tmp4 goto @1457
1457  ICONST  tmp0 = 2
1458  IMOV    tmp4 = tmp0
1459  RETURNVAL tmp4
```

Decompiled equivalent:

```js
v0 = 0;
if (GetGlobal(200) == 0) {
    if (HasAnyPCActionWhoIsInThisLevelOrCouldMaybeComeFromSherwood(28) == 0)
        return 2;
}
// global 200 == 1 branch: all PCs safe → DoneShortBriefing(1) → return 1
return v0;
```

- `28` is `ScriptAction::Lockpick` in
  [`crates/robin_level_data/src/profiles.rs`](../../crates/robin_level_data/src/profiles.rs)
  (`ScriptAction` enum, `Lockpick = 28`).
- Global `200` is set to `1` only by `Prison_800001ba.EnterZone` (when Robin
  enters the prison); at tick 0 it is `0`, so the loss branch is live.
- `HasAnyPCActionWhoIsInThisLevelOrCouldMaybeComeFromSherwood` is implemented at
  [`crates/robin_engine/src/natives/dispatch/script_core.rs`](../../crates/robin_engine/src/natives/dispatch/script_core.rs)
  (the `HasAnyPCAction…` arm): it iterates the spawned PCs, maps each through
  `pc_profile_index`, and returns `1` only if a live PC's profile has the action
  in `actions` or `contextual_actions` (or a dead non-VIP PC has a Sherwood
  replacement). It returned `0` because neither spawned PC has Lockpick.
- Tick 0 runs the check because `frame_counter = 0`, `0 % FRAMES_PER_SECOND(25)
  == 0`, and `0 % VICTORY_CHECK_INTERVAL(3) == 0` in
  [`crates/robin_engine/src/engine/tick/mission.rs`](../../crates/robin_engine/src/engine/tick/mission.rs).
  The `Ok(2)` arm calls `quit_mission()` and returns `GameCode::LevelFailed`.

The native's return value is correct; `Ok(2)` is genuine authored bytecode, not
a native bug and not wrong mission state.

## 2. Why the native returns 0 at tick 0 — the setup

Evidence from `host.log`:

```
INFO  robin_engine::campaign: Rust campaign: 2 missions, 10 characters, 1 in gang
ERROR robin_engine::engine::level_loading::pcs: Beam-me 1 requires character type 3 but no match in mission team
ERROR robin_engine::engine::level_loading::pcs: Beam-me 2 requires character type 2 but no match in mission team
ERROR robin_engine::engine::level_loading::pcs: Beam-me 3 requires character type 1 but no match in mission team
INFO  robin_engine::engine::level_loading::pcs: Spawned 1 PCs at beam-me positions
DEBUG robin_engine::engine::selection: Sorted pc_ids by priority: [Pc(PcId(198)), Pc(PcId(81))]
```

Beam-me types `3/2/1` are Little John / Marian / Friar Tuck
([`crates/robin_engine/src/engine/level_loading/pcs.rs`](../../crates/robin_engine/src/engine/level_loading/pcs.rs),
`assign_required_beam_me_pcs`). None were in the mission team, so only Robin
was seated from a beam-me; the two PCs at tick 0 are Robin (`PcId(198)`) and
Will (`PcId(81)`, script actor `81`, `AddPCToGang(GetActorScript(81))` in the
prison-zone script).

Profile actions from the shipped `Data/Configuration/profile.cpf` (converted
with `cpf_to_json`):

| Profile | actions | contextual actions |
| --- | --- | --- |
| `RobinHood` | Bow, Hit, Purse | Search, Climb, Jump, NoAction |
| `WillScarlet` | Stone, Strangle, BigShield | Climb, Jump, Execute, NoAction |
| `Friar Tuck` | WaspNest, Ale, Guzzle | Tie, **Lockpick**, NoAction, NoAction |
| `Stuteley` | Net, Apple, Beggar | Climb, **Lockpick**, Tie, NoAction |
| `MerryManB` | Apple, Heal, NoAction | Climb, Resuscitate, **Lockpick**, Execute |

Neither Robin nor Will has Lockpick, so the native's loop finds nothing and
returns `0`. The authored mission is "Save Scarlett" and the walkthroughs are
explicit that it needs a lockpicker (`docs/third-party/guides/*`); the demo's
auto-start gang exists precisely to supply one.

The campaign was built by the fresh-campaign path
([`crates/robin_engine/src/campaign.rs`](../../crates/robin_engine/src/campaign.rs),
`from_profiles` → `seed_initial_gang`: Robin alone), not by the demo gang path.
`--no-default-loose` does not help: the `Ok(2)` arm is before the
`ignore_default_loose` guard in `hourglass_phase_mission_and_messages`, and
`blocked_by_combat=false` rules out the combat-victory gate.

### Comparison: `Emb01_FoA_EC`

The ambush's `StartUp.CheckVictoryCondition` starts with `return 1` (after a
bonus-item branch that falls through) and never returns `2`, so it cannot fail
at tick 0 regardless of party composition. The Demo is the outlier: its first
branch is an explicit "no lockpicker in the party → lost" guard.

## 3. Host invocation that starts the Demo normally

Drop `--mission Dem_Lei_MP`. Keep the Demo datadir and the rest of the harness
flags:

```
ROBINHOOD_INSTALL_HOME=/tmp/robin-host-test \
ROBINHOOD_DATA_DIR=~/Games/robin-hood/demo-leicester/robinhood_demo \
ROBINHOOD_MODS_DIR=/tmp/robin-mods-empty \
ROBINHOOD_BROWSER_URL=https://robinhood.phiresky.xyz/ \
  DISPLAY=:98 robin --server --mp-browser-join-links true \
    --mp-expected-players 1 --mp-nickname host --http-server 18641 \
    --no-sound
```

Without `--mission`, `run_game` reaches the demo-detection block in
[`crates/robin_rs/src/main_entry/run.rs`](../../crates/robin_rs/src/main_entry/run.rs):
`detect_demo_mode_with_context` returns
`("Dem_Lei_MP", "Leicester", "RJMT", Leicester)` for the Leicester demo datadir,
and the block calls `create_gang_from_pcs("RJMT")` + `add_all_to_mission_team()`
before launching mission index 1. That seats Little John, Marian and Friar
Tuck (matching beam-mes 1–3) and gives the party a Lockpick PC, so
`CheckVictoryCondition` returns `0` at tick 0 instead of `2`. The multiplayer
session's mission id is the authoritative loaded mission id, so the browser
ticket remains `Dem_Lei_MP`.

By contrast, `--mission Dem_Lei_MP` enters `force_mission_launch` in
[`crates/robin_rs/src/main_entry/callbacks.rs`](../../crates/robin_rs/src/main_entry/callbacks.rs),
which calls `campaign.reset(...)` and does **not** apply the demo PC string
(except in map-export/hackable-level cases). That is the Robin-only gang seen
in the log. There is no CLI gang-string option; a saved campaign with the RJMT
gang would also work, but the supported route is the demo auto-start.

**Result:** with this setup the `victory` debug line reads `result=Ok(0)` and
the mission keeps running past game second 0; the browser seat then has a
playable mission to join.

## 4. Engine defect / one-line game fix

None. The engine's mapping `Ok(2) → LevelFailed` is correct, the native's
`pc_handles`-based check matches its documented scope, and the script's loss is
authored. There is no one-line game-code change to apply — the change belongs
in the harness invocation (remove `--mission`, let the Demo auto-start build
`RJMT`). No production code was modified for this investigation.

## Evidence sources

- `ssh minipc`: `/home/admin/solo-victory-20261119-221832/host.log`
  (`victory` debug line, campaign/beam-me/spawn lines).
- `disasm_scb --decompile` and `disasm_scb` over
  `/home/admin/Games/robin-hood/demo-leicester/robinhood_demo/Data/Levels/Dem_Lei_MP.scb`
  (quads 1438–1459 above).
- `cpf_to_json` over `Data/Configuration/profile.cpf` (profile actions,
  mission `required_character_indices: [0]`, `required_actions: []`).
- `disasm_scb --decompile` over the full-game `Emb01_FoA_EC.scb` (comparison).
