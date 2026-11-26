# Campaign co-op: one host-owned session

Issue #44: a campaign host never opened a session an ambush host would. This
records what was measured, what the fix changed, and what is still unproved.

Last verified: 2026-11-26 against `fork/main` `6a86371f3930` — H30's evidence
is `build-host:~/campaign-e2e-H30-20261119-165539` and the rows below are read
from the corpus contract, not from a new gate run.

## What could be measured pre-fix

A process-level reproduction of the reported path is not available on a
developer host: "Create lobby" is gated on a live matchmaking client
(`crates/robin_rs/src/main_menu/multiplayer_menu.rs`, `update_buttons`:
`valid && … && (self.local || connected)`), and `docs/multiplayer/HARNESS.md`
section 6 excludes matchmaking from the harness on purpose. So the measurement
is the value the menu published, which `CoopRules::validate` — unchanged by this
work — refuses:

```text
$ git show 4ff153ba6:crates/robin_rs/src/main_menu/multiplayer_menu.rs
664:    fn publish_rules(&mut self) {
665-        if self.coop.validate().is_ok()
666-            && !self.local
667-            && let Some(session) = &self.matchmaking_client
668-        {
669-            if let Err(error) = session.set_rules(self.coop) {
```

A continued campaign slot put the slot's own rules into `self.coop`
(`selected_rules`, `ID_CREATE`), and those are `campaign: true` **plus the roster
the slot played with**. `CoopRules::validate` refuses exactly that pair
("campaign co-op uses the persistent campaign roster"), so `publish_rules`
published nothing and returned no reason — while the `Created` handler still set
`"Game created. Press Start when ready."`. `update_buttons` used the same
`validate` for `valid`, so "Start mission" was disabled on the same words.

The assertion that pins the value is in
`main_menu::multiplayer_menu::tests::campaign_choices_publish_one_valid_campaign_lobby`;
`campaign_lobby_rules_drop_the_saved_party_and_keep_the_seats` pins the
conversion.

## What a campaign launch is now

- `CoopRules::campaign_lobby` (`crates/robin_engine/src/coop.rs`) is the one
  conversion from a save's rules to lobby words: `campaign`, no team, `Shared`
  control, the same seat count and health scaling. A save records those same
  words at quit, so a slot resumes with the same party size and mode.
- The menu publishes `selected_rules()` and never `self.coop`; a lobby that
  cannot be advertised says why in the status line and the `Created` handler
  leaves the game instead of sitting in it (`publish_or_report`).
- The hosted-lobby update takes `coop.players` from the lobby always; only a
  skirmish resets `assignments`. A campaign's roster is the host's, its party
  size is the lobby's.
- A campaign is one **session** across its missions, not one mission:
  `crate::game_session::run_session` owns it, so the mission-end page hands every
  seat to the next mission (`crates/robin_rs/src/game_session/mod.rs`,
  `run_session_body`; `crates/robin_rs/src/main_entry/run.rs` routes a
  `--mp-campaign` direct launch there instead of one `run_mission`).
- A resumed slot overrides the launch's simulation config with the lobby's
  campaign words, so a slot written by a solo run resumes as a campaign instead
  of ending after its first mission.
- A campaign host holding a slot plays the slot's current mission; a fresh
  campaign plays the mission its lobby advertised
  (`MultiplayerLaunch::plays_lobby_mission`). Both reach the session through
  `Campaign::next_mission_idx`, which `determine_next_mission` honours.
- A guest seat never writes the campaign slot
  (`crates/robin_rs/src/main_entry/callbacks/executor.rs`, `SaveLoadRequest::
  Continue`): one install owns the campaign.
- `--mp-campaign` starts the same session from the command line and requires
  `--server`/`--connect` and `--mp-expected-players`
  (`game_session/multiplayer.rs::validate_multiplayer_launch_args`).

## What proves it

- `a_campaign_lobby_replaces_the_slots_own_simulation_config`
  (`game_session/setup.rs`): the lobby's words replace a solo slot's config, and
  a missing party size installs nothing.
- `only_a_slot_holding_campaign_host_keeps_the_slots_mission`: which mission a
  host plays.
- `a_campaign_hands_its_next_mission_the_same_party` and
  `a_joining_seat_never_continues_the_hosts_session`
  (`main_entry/launch.rs`): the next mission is the same campaign with the same
  fixed party, and the continuation flag is the host's alone.
- `standalone_multiplayer_finished` (`game_session/mod.rs`) already refuses to
  end a session whose config is a campaign.
- **H30** (`scripts/validation/scenarios/H30.json`, process layer): a host with
  `--mp-campaign` advertises a session, a second seat from its own install joins
  with no local save, both seats are `running` and in the same mission
  (`/state.map`), the guest's `/state` reports the host's
  `expected_players: 2`, and the guest compares the host's hash against its own
  every 25 frames with no desync. Its `hash-equal` names seat 1 alone: the host
  publishes `StateHash` and a client's publication of one is refused as
  host-only (`multiplayer/client_outgoing.rs`), so a host's
  `last_compared_hash_frame` is null forever and a process row that named seat 0
  could never settle — H30 as first written asserted `peers=0,1` and hung at
  130 s with the session otherwise healthy (`…-20261119-160134`, both seats on
  `lincoln`, the guest comparing 24 frames, 0 desyncs). Evidence:
  `build-host:~/campaign-e2e-H30-20261119-165539` (`verdict: observed`).

## Found on this base, now covered by corpus rows

1. **`WIN` at mission end panics a joining seat.** `POST /console
   {"command":"WIN"}` on the host ends the mission; the host's
   `ApplyQuitMissionUpdates` reaches the client as a late replicated command, the
   client finalizes achievements inside the rollback and then panics at
   `crates/robin_engine/src/achievement.rs:1257` (`achievement effect after
   finalization`) → `fatal runtime error: failed to initiate panic`, seat lost.
   Reproduced with and without `campaign`, so it is not a campaign regression.
   Evidence: `build-host:~/campaign-e2e-H30-20261119-150616` (campaign) and
   `…-151409` (the same row with `campaign` removed), each a client panic right
   after `Post-mission warcrime recruitment`.
   Needs the rollback-safe mission-end command the terminal-matrix rows own
   (`/mission-result`, H21-H27); the console cheat is not rollback-safe.
2. **A same-identity client restart cannot re-handshake at the process layer.**
   After `SIGKILL` + relaunch on the same install, the client logs
   `initial multiplayer handshake failed: read frame header: connection lost`
   every ~0.5 s and gives up (`failed to connect to …`), so the seat is never
   reclaimed. Evidence: `build-host:~/campaign-e2e-H30-20261119-152232` (row with
   `restart`/`identity: same`). H14 proves the fresh-identity refusal; nothing
   proves the same-identity reclaim end to end.
3. **A lost mission does not carry both seats past the debriefing here.** With
   `LOOSE` on the host both seats reach the debriefing, the host records its vote
   (`host vote recorded; peers still owe an acknowledgement`) and waits; the
   client meanwhile reconnects after the host evicted its seat
   (`incoming peer handler ended: read frame header: connection lost`,
   `seat evicted: peer1`) under ~100 ms rollbacks, and never acks. Evidence:
   `build-host:~/campaign-e2e-H30-20261119-151723`. The acknowledgement budget and
   the eviction live in the terminal matrix (#25, H21-H27).
   Until those land, the mission-end handoff has no process row; H30 stops at
   "both seats are in the campaign's current mission".

The three conditions above were reproduced on the 2026-11-19 base; they are no
longer "found, not fixed" at this ref. H33 covers the `WIN` command path (the
cheat is staged as the replicated mission-result command), H35 the
same-identity reclaim, and H34 the `LOOSE` debriefing path. H33/H34 assert no
post-finalization achievement effect; H35 asserts the seat returns to `running`
without exhausting its reconnect budget. H36 and H38 then carry the handoff to
mission 2 through the game's own debriefing. The checks are the corpus rows in
[the harness](../multiplayer/HARNESS.md); this paragraph does not claim a gate
was run for them here.
