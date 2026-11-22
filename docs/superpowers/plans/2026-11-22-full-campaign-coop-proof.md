# Full Campaign Co-op Proof Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the observability and driver needed for one Linux host plus Linux guest to play a whole campaign end to end in one `--mp-campaign` session, and prove it with a gated `H37` row.

**Architecture:** The host publishes campaign facts through `/state` and a choice event through `/events`. An imperative `--campaign-walk` mode in the existing process driver loops on those facts, sending the game's own host-authoritative `CampaignSelectNextMission` command. No new schema repeat construct; the declarative H01-H36 corpus stays unchanged. H37 binds to the new mode and must pass with zero error states.

**Tech Stack:** Rust (nightly-2026-10-21), Python 3 stdlib driver, `bash scripts/check-quality.sh multiplayer-e2e`.

**Spec:** `docs/superpowers/specs/2026-11-22-full-campaign-coop-proof-design.md`

**Depends on:** `docs/superpowers/plans/2026-11-22-campaign-flow-case-coverage.md` landing green first.

## Global Constraints

- Worktree `.worktrees/full-campaign-proof` off `fork/main`; never commit to the read mirror.
- The driver reads `/state` and `/events`, never log text and never `/engine-dump`.
- The game publishes the stop condition (`complete`) and the choice (`suggested_choice`); the driver never computes the mission order.
- All four cross-play directions and packaging (issue #49 criteria 3 and 9) are BLOCKED without a Windows seat; this plan proves Linux host + Linux guest only.
- `bash scripts/check-quality.sh SUITE` is the only test entry point; process layer is `multiplayer-e2e`.
- One DEFECTS row per diagnosed defect, written before the fix; a row is `fixed` only in a gated suite.
- Commit by pathspec; no `git add -A`; no `CARGO_TARGET_DIR`; no shell redirection of cargo output.

## Review Focus

- A peer reading another seat's campaign or choosing a mission on its behalf — Task 2 and Task 3 pin host-only publication and host-authoritative commands.
- A driver that hardcodes the mission count or stops early and calls it complete — Task 1 pins `complete` from the H12 boundary and Task 5 pins the row's count from the campaign object.
- A choice event emitted once per seat rather than once per committed choice — Task 3 pins it.
- A replayable mission that satisfies the row without advancing the campaign — Task 5 asserts entered mission == committed choice == suggested choice.
- A silent peer or bounded-wait expiry being misreported as success — Task 4 pins the loop's error-state assertions.

---

### Task 2: Publish campaign facts on `/state`

**Files:**
- Modify: `crates/robin_rs/src/http_server/diagnostics.rs:126-152` (`snapshot_state`)
- Modify: `crates/robin_rs/src/http_server/types.rs:13-25` (`SessionFacts`)
- Modify: `crates/robin_rs/src/http_server/dispatch.rs:62-120` (drain call site)
- Test: `crates/robin_rs/src/http_server/diagnostics.rs` test module

**Interfaces:**
- Consumes: `snapshot_state(engine: &Engine, replay: Option<ReplayStatus>, transport: Option<&HostTransport>, session: Option<SessionFacts>) -> serde_json::Value` (`diagnostics.rs:126`); `Engine::campaign() -> &Campaign` (`host_accessors.rs:1018`); `assets.profile_manager` (`dispatch.rs:63`).
- Produces: `snapshot_state` gains a `campaign` key with `current_mission`, `accessible_mission_indices`, `next_mission_idx`, `suggested_choice`, `complete`, `mission_count`, `awaiting_choice`. Single-player peers publish `null` or an empty object; no peer reads another seat's campaign.

- [ ] **Step 1: Write the failing test**

`snapshot_state_publishes_host_campaign_facts`: build an engine with a known campaign state, call `snapshot_state`, assert the `campaign` object carries the exact current mission, accessible list, suggested choice, `complete: false`, and the catalogue mission count.

- [ ] **Step 2: Confirm RED** with the one-test recipe from `docs/TESTING.md:90-95`.

- [ ] **Step 3: Implement**

Thread `assets: &LevelAssets` into `snapshot_state` and its callers (`multiplayer.rs:181`, `diagnostics.rs:848,855,911,958`; tests). Add the key to the `json!` at `diagnostics.rs:146-151`. Derive facts from `Campaign` fields and `suggested_next_mission` / `is_complete` from Task 1.

- [ ] **Step 4: Confirm GREEN**, then `format`, `check`, `client-release`.

- [ ] **Step 5: Commit** by pathspec.

### Task 3: Emit a campaign-choice event

**Files:**
- Modify: `crates/robin_rs/src/observability/events.rs:40-74` (`EventKind`)
- Modify: `crates/robin_rs/src/game_session/mouse_input.rs:2458-2466` (commit site)
- Test: `crates/robin_rs/src/game_session/mouse_input.rs` test module

**Interfaces:**
- Consumes: `EventKind` (`events.rs:40-74`); `crate::observability::event(kind, message) -> Event` (`observability.rs:152`); `Event::field(key, impl Serialize)` and `Event::record()` (`events.rs:99-133,136-192`); commit site dispatching `PlayerCommand::CampaignSelectNextMission { mission_idx: Some(index) }`.
- Produces: `EventKind::CampaignChoice`, with `mission_idx` and the committing seat, recorded exactly once per committed choice on the host only.

- [ ] **Step 1: Write the failing test**

`campaign_choice_event_records_mission_and_seat`: drive the host commit; assert one `CampaignChoice` event with the expected `mission_idx` and seat, and none from the guest.

- [ ] **Step 2: Confirm RED.**

- [ ] **Step 3: Implement** the variant and the emission at the host-authoritative commit site; do not emit for a refused or proposed choice.

- [ ] **Step 4: Confirm GREEN**, then `format` and `client-release`.

- [ ] **Step 5: Commit** by pathspec.

### Task 1: Derive suggested choice and completion on `Campaign`

**Files:**
- Modify: `crates/robin_engine/src/campaign.rs` (near `:884`, `:633-654`)
- Test: `crates/robin_engine/src/campaign.rs` test module

**Interfaces:**
- Consumes: `Campaign.next_mission_idx` (:568), `accessible_mission_indices` (:563), `achievement_envelope_complete(&ProfileManager) -> bool` (:884), `get_sherwood_mission_idx()` (:2149), and the availability rank rule in `campaign_map.rs:2843`.
- Produces: `pub fn suggested_next_mission(&self) -> Option<usize>` returning `next_mission_idx` when set, otherwise the single accessible non-HQ mission when there is exactly one, otherwise the highest-priority accessible mission by the game's own rank; `pub fn is_complete(&self, profiles: &ProfileManager) -> bool` delegating to the H12 boundary.

- [ ] **Step 1: Write failing tests**

`suggested_choice_uses_committed_index`, `suggested_choice_uses_game_rank_for_sherwood_options`, `campaign_complete_requires_the_h12_boundary`.

- [ ] **Step 2: Confirm RED** via the `engine` suite one-test recipe.

- [ ] **Step 3: Implement** the two read-only methods; do not call the `&mut self` `determine_next_mission`.

- [ ] **Step 4: Confirm GREEN**, then `format` and `engine`.

- [ ] **Step 5: Commit** by pathspec.

### Task 4: Imperative `--campaign-walk` driver mode

**Files:**
- Modify: `scripts/validation/multiplayer_e2e.py` (`main` :1579, `Peer.argv` :835, `ScenarioRun` :928, `start` :1336, `resolve` :1240, `settle` :1098, `judge` :1375, `evaluate_expectation` :603)
- Test: `scripts/validation/tests/test_campaign_walk.py` (new)

**Interfaces:**
- Consumes: `snapshot_state` campaign object from Task 2; `CampaignChoice` events from Task 3; `dispatch_command` (`http_server/dispatch.rs:248-257`) for `PlayerCommand::CampaignSelectNextMission { mission_idx: Some(usize) }`; existing `resolve()`, `poll()`, `settle()`, `Transcript.event`.
- Produces: a pure `campaign_walk_step(state: dict, planned_result: str) -> dict` returning one of `end_mission`, `answer_page`, `choose_next`, `done`, `error`; and a `--campaign-walk` flag that loops on it until `campaign.complete`.

- [ ] **Step 1: Write failing tests** in the new Python file using `unittest`:

`walk_step_ends_then_answers`, `walk_step_chooses_only_suggested_mission`, `walk_step_stops_only_on_complete`, `walk_step_reports_error_state_as_error` (desync, session failure, budget expiry, or silence).

- [ ] **Step 2: Confirm RED** with `python3 -m unittest discover -s scripts/validation/tests`.

- [ ] **Step 3: Implement** the pure step function, then wire the flag through `options` → `run_one` → `run_scenario` → `ScenarioRun`, recording every choice in the transcript. The loop never relaunches a peer with `--mission`.

- [ ] **Step 4: Confirm GREEN** with the Python tests plus `bash scripts/check-quality.sh tooling`.

- [ ] **Step 5: Commit** by pathspec.

### Task 5: H37 scenario row and corpus binding

**Files:**
- Modify: `scripts/validation/scenario_corpus.py:150-151` (`SCENARIO_FIELDS`), `:115-131` (`EXPECT_FIELDS`), `:162` (`RESOLVE_RESULTS` if needed)
- Create: `scripts/validation/scenarios/H37.json`
- Modify: `docs/multiplayer/HARNESS.md` (stale paragraph at :70-75 and new H37 row)
- Modify: `docs/DEFECTS.md` (new row for the full-run proof)

**Interfaces:**
- Consumes: `SCHEMA = 4` (`scenario_corpus.py:62`); `H30.json` baseline; `H36.json` fault + resolve shape; assertion vocabulary at `multiplayer_e2e.py:603-622`.
- Produces: `H37` extends `H30`, sets `"walk": "full-campaign"`, asserts `campaign(complete=true)`, `mission(count_from_campaign=true)` comparing the observed distinct maps to `/state.campaign.mission_count`, `hash-equal(every=25)`, `exit(running)`, and no `Desync`/`SessionFailed` events.

- [ ] **Step 1: Write a failing validator test**

`H37_is_accepted_and_binds_the_campaign_walk_prover`: load the corpus, assert the row validates, has `walk: full-campaign`, and its expectations include the campaign-complete and hash assertions.

- [ ] **Step 2: Confirm RED.**

- [ ] **Step 3: Implement** the schema additions, the scenario, and the HARNESS correction. Add `campaign(complete)` and `mission(count_from_campaign)` to `EXPECT_FIELDS` and their evaluators in `multiplayer_e2e.py:603-622`. Remove the stale sentence claiming the handoff has no process row.

- [ ] **Step 4: Confirm GREEN** with the corpus validator and `tooling` suite.

- [ ] **Step 5: Commit** by pathspec.

### Task 6: Prove the full campaign green

- [ ] **Step 1: Build** the pinned binary with `scripts/remote-check.sh client-release`.
- [ ] **Step 2: Run** `ROBIN_E2E_BINARY=/absolute/path/to/robin ROBINHOOD_DATA_DIR=/absolute/path/to/full-game bash scripts/check-quality.sh multiplayer-e2e`.
- [ ] **Step 3: Inspect** H37's `summary.json`, `events.jsonl` and `hashes.json`: complete flag observed, mission count equals catalogue count, hash cadence held, zero desync/session-failure events, and every recorded choice matches the mission entered next.
- [ ] **Step 4: Fix** any failing assertion with a DEFECTS row and a named regression test first, then re-run. Do not mark a row green without the evidence file.
- [ ] **Step 5: Mark** the DEFECTS row `fixed` with the evidence path.
- [ ] **Step 6: Land** with `runbg run full-campaign-proof -- bash scripts/land.sh`, then remove the worktree and branch.
