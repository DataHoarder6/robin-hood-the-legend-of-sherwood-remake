# Campaign Flow Case Coverage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every decision point in the campaign-flow case inventory a named test, and fix every defect those tests expose, so all cases are green before the full-campaign proof lands.

**Architecture:** TDD per case. Each task writes a failing test against the current behavior, files a DEFECTS row when the test exposes a defect, applies the minimal fix, then gates. Tests live beside the code they own, in the crate's existing test modules, so `scripts/push-gate.sh` runs them.

**Tech Stack:** Rust (nightly-2026-10-21), `cargo test`, `bash scripts/check-quality.sh SUITE`, git worktree on `fork/main`.

**Spec:** `docs/superpowers/specs/2026-11-22-full-campaign-coop-proof-design.md`

**Inventory:** `docs/slop/campaign-flow-case-inventory.md`

## Global Constraints

- Worktree `.worktrees/full-campaign-proof` off `fork/main`; the shared `main` is a read mirror and is never committed to.
- One diagnosed defect per `docs/DEFECTS.md` row; the row carries class, reproduction, and the check that fails without the fix. Row before fix.
- A row is `fixed` only when its check is in a suite the gate runs.
- Tests are written red, run to confirm they fail, then fixed. Nothing lands red.
- `bash scripts/check-quality.sh SUITE` is the only test entry point. Bare `cargo test` is not a gate.
- Run cargo unfiltered; no `| head`, no shared log path.
- Never set `CARGO_TARGET_DIR`.
- Commit by pathspec, `git commit -- <files>`; never `git add -A`.
- Use the repo's terms: Debriefing, campaign map, presentation defect, generation defect.

## Review Focus

- A Restart button that is shown and enabled while no restart snapshot exists silently advances the page instead of restarting — Task 1 pins it.
- A "Watch replay" button enabled in a browser can never play a recording there — Task 6 pins it.
- An aborted mission result must reach the aborted Debriefing and the `Interrupted` leaderboard, not the win page — Task 8 pins it.
- A debriefing decision must not be recorded unless the mission actually entered next matches it — Task 8 pins the named test; the process-layer binding is Plan 2.
- A seat that stays silent on a page must settle on a bounded wait — Task 2 and Task 3 pin the terminal and Sherwood cases.

---

### Task 1: Restart button tells the truth

**Files:**
- Modify: `crates/robin_rs/src/ingame_menu/debriefing.rs:674-692` (draw gate and `BTN_RESTART` handler)
- Test: `crates/robin_rs/src/ingame_menu/debriefing.rs` test module
- Modify: `docs/DEFECTS.md` (new row)

**Interfaces:**
- Consumes: `DebriefingContent.restart_allowed: bool` (:128), `DebriefingContent.restart_snapshot_exists: bool` (:135); `PageOutcome::{Ok,Restart}` (:102).
- Produces: Restart control offered only when both flags hold; activating it yields `PageOutcome::Restart`.

- [ ] **Step 1: Write the failing test**

`restart_button_without_snapshot_does_not_offer_restart`: build a `DebriefingModalState` with `restart_allowed: true`, `restart_snapshot_exists: false`; assert the Restart control is not drawn or is disabled, and that activating its id does not produce `PageOutcome::Restart`.

- [ ] **Step 2: Add the DEFECTS row**

New row, class `presentation`, with reproduction "open a Debriefing with `restart_allowed=true` and no restart save, click Restart", check = the test from Step 1, status `open`.

- [ ] **Step 3: Confirm RED**

Run the one-test recipe from `docs/TESTING.md:90-95`. Expected: FAIL.

- [ ] **Step 4: Fix**

Gate drawing and handling of `BTN_RESTART` on `restart_snapshot_exists`, not only `restart_allowed`. Keep the existing warning only as an unreachable guard.

- [ ] **Step 5: Confirm GREEN and gate**

Run the same test, then `bash scripts/check-quality.sh format` and `client-release`.

- [ ] **Step 6: Mark the row `fixed` and commit**

```bash
git add crates/robin_rs/src/ingame_menu/debriefing.rs docs/DEFECTS.md
git commit -m "fix(ui): do not offer restart without a restart snapshot" -- crates/robin_rs/src/ingame_menu/debriefing.rs docs/DEFECTS.md
```

### Task 2: Live debriefing phase chain

**Files:**
- Modify: `crates/robin_rs/src/game_session/terminal_debriefing.rs` (`TerminalDebriefingPhase` drive)
- Test: same file

**Interfaces:**
- Consumes: `TerminalDebriefingPhase::{Debriefing,LoadPicker,AwaitingFinalAuthority,LeaderboardPendingStart,AwaitingLeaderboard,AwaitingSnapshot}` (:227); `TerminalDebriefingProgress::{Complete,EmergencyExit}` (:300); `MissionLeaderboardRuntime` rearm methods.
- Produces: a terminal debriefing with a leaderboard endpoint reaches `Complete` with `AwaitingSnapshot` observed on the way.

- [ ] **Step 1: Write the failing test**

`terminal_phase_chain_reaches_complete_through_leaderboard_and_snapshot`: drive a host terminal flow with a board endpoint; assert the observed phase sequence contains `Debriefing`, `LeaderboardPendingStart`, `AwaitingLeaderboard`, `AwaitingSnapshot`, then `Complete`.

- [ ] **Step 2: File a DEFECTS row if it fails**, class `presentation` or `generation` per the observed mechanism, with the test as check.

- [ ] **Step 3: Confirm RED** with the one-test recipe.

- [ ] **Step 4: Fix** the phase transition that stalls or skips; rearm one-shot leaderboard preparation for the new attempt if that is the cause.

- [ ] **Step 5: Confirm GREEN**, then `format` and `client-release`.

- [ ] **Step 6: Commit** by pathspec with the DEFECTS row if one was filed.

### Task 3: Sherwood outcome arms and non-host refusal

**Files:**
- Modify: `crates/robin_rs/src/game_session/mouse_input.rs:2362-2560` (`drive_sherwood_modal_flow`)
- Test: same file's test module
- Modify: `docs/DEFECTS.md` if a case fails

**Interfaces:**
- Consumes: `SherwoodConfirmationAction::{ReturnToMap,LeaveWonMission,StartMission{men_to_blazon}}` (`sherwood_flow.rs:34`); `MissionChoice::{StartMission,ShowPendingMissions,None}` (`ui_screens.rs:91`); `ExternalAction`; non-host refusal message at `mouse_input.rs:2621`.
- Produces: each arm dispatches its documented command; a non-host cannot drive the overlay.

- [ ] **Step 1: Write failing tests**

`sherwood_confirmation_arms_dispatch_their_commands`: one case per `SherwoodConfirmationAction` and per `MissionChoice`, asserting the command or `ExternalAction` produced.
`non_host_cannot_drive_the_sherwood_overlay`: assert the refusal message and no command dispatched.

- [ ] **Step 2: File DEFECTS rows for failing arms**, class `presentation`, with the test names as checks.

- [ ] **Step 3: Confirm RED** with the one-test recipe.

- [ ] **Step 4: Fix** the arm that does not dispatch or the refusal that leaks.

- [ ] **Step 5: Confirm GREEN**, then `format` and `client-release`.

- [ ] **Step 6: Commit** by pathspec.

### Task 4: Campaign map cases

**Files:**
- Modify: `crates/robin_rs/src/campaign_map.rs` (`handle_sherwood_campaign_map_overlay` callers, detail actions)
- Test: same file

**Interfaces:**
- Consumes: `CampaignMapChoice::{SelectMission,PseudoDebriefTimer,Quit,Redisplay}` (:113); `MissionAvailability::{Ready,Completed,Locked}` (`campaign_progress.rs:114`); "Inspect" region and `l`/`f`/`s` detail actions.
- Produces: `Redisplay` re-enters the map without launching; Inspect is inert for a locked pin; leaderboard actions report a status string; selection cancelled at the description modal commits no mission.

- [ ] **Step 1: Write failing tests**

`redisplay_reenters_the_map_without_launching`, `inspect_is_inert_for_a_locked_pin`, `detail_leaderboard_actions_report_status`, `cancelling_after_selection_commits_no_mission`.

- [ ] **Step 2: File a DEFECTS row per failure**, class `presentation`.

- [ ] **Step 3: Confirm RED.**

- [ ] **Step 4: Fix** the failing paths.

- [ ] **Step 5: Confirm GREEN**, then `format`, `client-release`, and `engine`.

- [ ] **Step 6: Commit** by pathspec.

### Task 5: Leaderboard widget cases

**Files:**
- Modify: `crates/robin_rs/src/ingame_menu/leaderboard.rs:189-245`, `src/leaderboard/mission_end.rs:252-268`
- Test: those files' test modules

**Interfaces:**
- Consumes: `MissionEndLeaderboardAction::{SelectTab,RetryBoard,SubmitThisRun,RetrySubmission,SetAlwaysSubmitRuns,Close}` (`mission_end.rs:252`); `BoardLoadState::{Loading,Ready,Failed}` (:228); `MissionSubmissionState::{WonMissionRequired,Unavailable,AwaitingConsent,Submitting,Queued,Failed}` (:235).
- Produces: enablement and action mapping for each state; `Retry` dispatches `RetryBoard` on board failure and `RetrySubmission` on submission failure.

- [ ] **Step 1: Write failing tests**

`submit_enabled_only_awaiting_consent`, `retry_dispatches_board_or_submission_by_state`, `close_always_available`, `unavailable_notice_requires_acknowledgement`.

- [ ] **Step 2: File DEFECTS rows for failures**, class `presentation`.

- [ ] **Step 3: Confirm RED.**

- [ ] **Step 4: Fix** `sync_widget_enabled` / `widget_action`.

- [ ] **Step 5: Confirm GREEN**, then `format` and `client-release`.

- [ ] **Step 6: Commit** by pathspec.

### Task 6: Replay buttons tell the truth

**Files:**
- Modify: `crates/robin_rs/src/mission_replays/browser.rs:42-47`, `crates/robin_rs/src/campaign_map.rs:2776-2790`, `crates/robin_rs/src/mission_replays/native.rs`
- Test: those files

**Interfaces:**
- Consumes: `mission_replays::watch` native/browser implementations; `CampaignMapModalState::watch_selected_play` (`campaign_map.rs:772`); `MissionAvailability::Completed` / `is_mission_replayable` (`campaign_progress.rs:114/129`).
- Produces: the Watch control is not offered where playback cannot work; native watch succeeds; a completed-mission replay leaves campaign rewards unchanged.

- [ ] **Step 1: Write failing tests**

`watch_replay_not_offered_in_browser`, `native_watch_success_opens_viewer`, `completed_mission_replay_leaves_rewards_unchanged`.

- [ ] **Step 2: File DEFECTS rows**, class `presentation`, for the browser button and any native gap.

- [ ] **Step 3: Confirm RED.**

- [ ] **Step 4: Fix** the browser enablement and any native success path.

- [ ] **Step 5: Confirm GREEN**, then `format` and `client-release`.

- [ ] **Step 6: Commit** by pathspec.

### Task 7: Load and quick-load cases

**Files:**
- Modify: `crates/robin_rs/src/game_session/terminal_debriefing.rs:961-1014`, `src/game_session/frame_simulate.rs:866`
- Test: those files

**Interfaces:**
- Consumes: `TerminalDebriefingAction::{LoadRestart,Load{slot,mission_id}}` (:76-82); `SaveLoadRequest::{Load,LoadRestart,QuickLoad}` (`callbacks.rs:120`); `quick_load_key` gate in `debriefing.rs:200`.
- Produces: `Load` restores the campaign position end to end; picker cancel returns to Debriefing; non-host quick load reports the host-only refusal.

- [ ] **Step 1: Write failing tests**

`debriefing_load_restores_campaign_position`, `load_picker_cancel_returns_to_debriefing`, `quick_load_refused_for_non_host`.

- [ ] **Step 2: File DEFECTS rows for failures**, class `generation` where authority is involved, otherwise `presentation`.

- [ ] **Step 3: Confirm RED.**

- [ ] **Step 4: Fix** the mapped action or the authority gate.

- [ ] **Step 5: Confirm GREEN**, then `format` and `client-release`.

- [ ] **Step 6: Commit** by pathspec.

### Task 8: Results, dialogs, and decision-to-mission link

**Files:**
- Modify: `crates/robin_rs/src/game_session/mod.rs`, `src/game_session/frame_simulate.rs:766-790`, `crates/robin_engine/src/game_operation.rs`
- Test: those files

**Interfaces:**
- Consumes: `MissionResult::{Won,Lost,Aborted}` and `game_code()` (`game_operation.rs:74/86`); `leave_prompt()` / `leave_prompt_buttons()` (`frame_simulate.rs:766/781`); `Campaign.current_mission_idx` / `next_mission_idx` (`campaign.rs:567/568`).
- Produces: aborted reaches the `Interrupted` leaderboard and lose-table page; the `YesOnly` first-mission prompt confirms; a dismissed debriefing decision is reflected in the mission entered next.

- [ ] **Step 1: Write failing tests**

`aborted_result_reaches_interrupted_leaderboard`, `yes_only_leave_prompt_confirms_first_mission`, `debriefing_outcome_matches_next_mission_entered`.

- [ ] **Step 2: File DEFECTS rows for failures.**

- [ ] **Step 3: Confirm RED.**

- [ ] **Step 4: Fix** the result mapping or the decision handoff.

- [ ] **Step 5: Confirm GREEN**, then `format`, `engine`, `client-release`.

- [ ] **Step 6: Commit** by pathspec.

### Task 9: Full case gate

- [ ] **Step 1: Run** `bash scripts/check-quality.sh format lint check engine client client-release tooling docs`.
- [ ] **Step 2: Verify** every row in `docs/slop/campaign-flow-case-inventory.md` names a test that passes.
- [ ] **Step 3: Commit** any remaining DEFECTS status updates by pathspec.
