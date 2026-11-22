# Campaign-flow case inventory

Evidence doc. Measured against `fork/main` at `7d4d08b00` on 2026-11-22 from
`.worktrees/full-campaign-proof`. Every decision point and outcome reachable in
the campaign flow, with the named tests that bind it and what appears untested.
Sources are the on-disk source in that worktree; paths are relative to its root.

This inventory feeds the case matrix in
`docs/superpowers/specs/2026-11-22-full-campaign-coop-proof-design.md`.

## 1. Debriefing / terminal page

Sources: `crates/robin_rs/src/game_session/terminal_debriefing.rs`,
`ingame_menu/debriefing.rs`, `game_session/debriefing.rs`,
`game_session/session_policy.rs`, `ingame_menu/modal_net.rs`.

Actions: `TerminalDebriefingAction` = `Continue`, `LoadRestart`,
`Load{slot,mission_id}`, `EmergencyExit`. `SettledDebriefingOutcome` = `Ok`,
`Restart`, `Load{slot}`, `EmergencyEnd`, mapped to `DialogResult::Completed|
Restart|Load|Aborted`. `DebriefingOutcome` = `Ok`, `Restart`,
`LoadAttempt{body,was_on_stat}`, `EmergencyEnd`. `PageOutcome` = `Ok`,
`Restart`, `LoadClicked`, `EmergencyEnd`. Published dismissals:
`["Completed", "Restart", "Aborted", "Load"]`.

Phases: `DebriefingPhase` = `Body|Stat|Done`; `TerminalDebriefingPhase` =
`Debriefing|LoadPicker|AwaitingFinalAuthority|LeaderboardPendingStart|
AwaitingLeaderboard|AwaitingSnapshot`; `TerminalDebriefingProgress` =
`Inactive|Pending|Complete|EmergencyExit|SnapshotUnconfirmed`.

Buttons/keys: `BTN_OK`→`Ok`; `BTN_RESTART` (drawn when `restart_allowed`)→
`Restart` only when `restart_snapshot_exists`, else **`Ok`** — the
enabled-but-refusing case; `BTN_LOAD`→`LoadClicked`; `ScreenKey::Quit`→
`EmergencyEnd`; `Confirm`→`Ok`; quick-load key→`LoadClicked`; scroll keys
navigate. `ModalNet::waiting_message` publishes a bounded countdown while a
seat owes a decision. Only a host-authored decision closes a surface. Client
waits for host snapshot on `Ok`/`Load`; `SNAPSHOT_CONFIRM_BUDGET = 30s`.

Tests: `terminal_debriefing_maps_to_explicit_control_actions`,
`the_presentation_is_the_decision_order_the_flow_validates`,
`terminal_http_rejects_unbound_selection_without_retaining_a_decision`,
`terminal_http_selection_is_pinned_before_authority_and_leaderboard_waits`,
`terminal_http_cannot_replace_a_retained_failed_decision`,
`terminal_picker_selection_survives_reordering_but_rejects_retirement`,
`terminal_remote_and_replay_indices_never_authorize_local_files`,
`client_terminal_completion_waits_for_the_host_snapshot`,
`an_unconfirmed_snapshot_wait_gives_up_when_its_budget_ends`,
`recorded_terminal_load_does_not_wait_for_a_live_snapshot`,
`client_terminal_load_keeps_simulation_paused_before_snapshot_prepare_arrives`,
`host_completion_echo_releases_debriefing_without_running_gameplay`,
`client_terminal_waits_for_host_update_without_sending_a_forbidden_command`,
`a_reconnecting_peer_whose_snapshot_already_carried_the_terminal_update_can_open_its_page`,
`aborted_page_dismissal_returns_to_the_campaign`,
`terminal_text_table_follows_win_loss_outcome`,
`leaderboard_outcome_is_defined_only_for_terminal_attempts`,
`debriefing_waits_for_the_terminal_campaign_command`,
`scrollable_body_advances_once_then_statistics_complete`,
`load_attempt_preserves_complete_body_and_resume_phase`,
`restart_and_emergency_end_do_not_advance_the_page`,
`terminal_decisions_have_one_boundary_and_no_fabricated_completion`,
`replay_terminal_requires_exactly_one_matching_campaign_update`,
`client_proposal_does_not_become_a_decision_locally`,
`host_proposal_is_advisory_until_host_ui_decides`,
`host_waits_for_every_client_including_late_loading_clients`,
`terminal_and_story_pages_name_the_same_await_set`,
`client_only_closes_on_host_decision`,
`driver_gate_retains_failed_outcome_and_retries_without_another_ui_action`,
`an_expired_acknowledgement_is_recorded_with_no_modal_surface_bound`.

Untested: `restart_allowed && restart_snapshot_exists` enablement gate; full
live `LeaderboardPendingStart → AwaitingLeaderboard → AwaitingSnapshot`
transition; `EmergencyExit` applying `GameCode::Quit` end to end.

## 2. Campaign map / mission selection

Source: `crates/robin_rs/src/campaign_map.rs`, `src/campaign_progress.rs`.

Choices: `CampaignMapChoice` = `SelectMission(usize)|PseudoDebriefTimer|Quit|
Redisplay`. Availability: `MissionAvailability` = `Ready|Completed|Locked`,
with `starts_mission()` false only for `Locked`; `availability_rank()` orders
Ready 0, Completed 1, Locked 2; `pin_is_interactive()` mirrors `starts_mission()`.

Paths: keys `r/d/b` details, `Escape`→`Quit`, `Tab` cycles
`ClassicMap|ProgressTree|SherwoodMuseum`, `a` achievements, `PageUp/Down`,
arrows, `Return|KpEnter|Space` selects when `starts_mission()`; mouse
double-click details, "Inspect" launch, tabs, pager, close/back; detail keys
`l` full-campaign leaderboard, `f` mission leaderboard, `s` submit/open,
`Return` watch replay, `Escape` close; "Watch replay"/"No recording",
"Submit (S)"/"Open online (S)"; `Redisplay` re-entry; `PseudoDebriefTimer` at
500 ms; `browsing` mode may only produce `Quit`.

Tests: `campaign_map_items_carry_ready_completed_and_locked_states`,
`locked_pin_cannot_dispatch_a_commit`,
`a_locked_pin_has_no_hover_highlight_selection_or_click_claim`,
`double_click_opens_details_even_for_locked_missions`,
`mission_details_cannot_launch_a_mission_and_tabs_restore_navigation`,
`details_can_select_every_play_without_launching_the_mission`,
`submission_controls_do_not_watch_or_launch_an_unavailable_recording`,
`mission_board_requires_a_real_mission_identity`,
`browser_navigates_same_views_but_cannot_launch`,
`browser_with_no_missions_still_navigates_and_closes`,
`pointer_tabs_and_back_work_without_launching_or_stale_gallery_selection`,
`pseudo_debrief_delay_uses_elapsed_time_across_clock_wrap`,
`username_modal_captures_editing_and_cancel_without_navigating`,
`classic_pointer_ignores_positions_outside_the_map_panel`,
`click_outside_the_map_panel_hits_no_pin_and_keeps_the_pointer`,
`campaign_views_scroll_independently_and_hit_test_the_visible_cards`.

Untested: `Redisplay` re-entry at overlay driver level; "Inspect" enabled
geometry; `l`/`f` status strings; submit registration-cancel path.

## 3. Sherwood overlay flow

Sources: `src/game_session/sherwood_flow.rs`, `mouse_input.rs:2000-2948`,
`src/sherwood_hud.rs`, `src/ingame_menu/mission_description.rs`,
`src/ui_screens.rs`, `src/ingame_menu/perks_panel.rs`,
`src/ingame_menu/buy_blazons.rs`.

Screens: `SherwoodCampaignFlow` = `Map|PseudoDebrief|MissionDescription|
Confirmation|PerksPanel`. Actions: `SherwoodConfirmationAction` =
`ReturnToMap|LeaveWonMission|StartMission{men_to_blazon}`. Rail buttons:
`DisplayCampaignMap|GoToExit|StartMission|QuitMission|SherwoodTrading|
PerksPanel`; rail states `PreCommit|PostCommit|PostCommitStartNotReady|
InMission`. Mission-description buttons: `Cancel|StartMission|ConvertMoney|
ConvertPeasants|ConvertMission`; `MissionChoice` =
`StartMission|ShowPendingMissions|None`; `BuyBlazonsOutcome` = `Bought|
Cancelled`. `PerksPanelOutcome` = `Close|Purchase`. Non-host is refused with
"The multiplayer host is choosing the next campaign action."

Outcomes in `drive_sherwood_modal_flow`: pseudo-debrief any outcome →
ack; ARES=0 → `Exit(Quit)`; `StartMission` → `CampaignSelectNextMission` +
blazon mode + post-commit; `ShowPendingMissions` → swap + show map;
`ReturnToMap` → show map with mission still committed; `LeaveWonMission` →
`QuitMissionRequested`; blazon-conversion → unselect + convert + reopen map;
normal start → harvest + `SaveLoadRequest::Sherwood` + `Exit(LevelInterrupted)`.

Tests: `a_mission_offers_finish_and_abandon`,
`unmet_requirements_remove_the_start_mission_button`,
`a_hidden_start_button_frees_its_pixels_but_other_controls_keep_theirs`,
`presence_follows_campaign_state`,
`perks_panel_is_a_sherwood_only_live_button`,
`every_offered_button_answers_for_its_own_centre_when_enabled`,
`absent_buttons_do_not_claim_clicks`,
`mouse_release_over_sherwood_rail_is_hud_owned`,
`the_goto_exit_press_pair_walks_then_runs`,
`a_single_goto_exit_press_never_accelerates_the_selection`,
`mission_description_window_stays_on_screen_at_every_edge_and_viewport`,
`requested_origin_near_the_right_edge_moves_the_window_outside_the_map_panel`,
`return_buys_the_selected_characters_row_exactly_once`,
`a_pointer_click_buys_the_row_under_the_cursor`,
`owned_unaffordable_and_last_clover_rows_do_not_emit_a_purchase`,
`escape_closes_without_a_command`.

Untested: driver-level outcome dispatches (all confirmation and mission-choice
arms), non-host refusal, ARES=0 → `Exit(Quit)`, buy-blazon cascade-close.

## 4. Mission results

Sources: `crates/robin_engine/src/game_operation.rs`,
`src/ingame_menu/mod.rs`, `src/game_session/terminal_debriefing.rs`.

`MissionResult::Won|Lost|Aborted` → `LevelSucceeded|LevelFailed|
LevelInterrupted`. `GameCode` full set = `LevelInProgress|LevelFailed|
LevelSucceeded|LevelInterrupted|LevelNext|LevelRestart|Quit|LevelLoad|
LevelSave`. `MissionEndKind` title ids 24/25/26, leave-prompt ids 235/238.
`leaderboard_outcome()` defined for succeeded/failed/interrupted only;
lost and aborted use the lose text table.

Tests: `mission_end_kind_comes_from_the_exit_code`,
`mission_end_titles_are_the_three_retail_ids`,
`leave_prompt_ids_are_the_retail_questions`, `mission_result_game_codes`,
`terminal_text_table_follows_win_loss_outcome`,
`aborted_page_dismissal_returns_to_the_campaign`,
`leaderboard_outcome_is_defined_only_for_terminal_attempts`,
`playback_terminal_keeps_recorded_update_and_debrief_progression`,
`live_terminal_still_stages_one_campaign_update`.

Untested: driver-triggered `POST /mission-result` `aborted` reaching the
`Interrupted` leaderboard and aborted page as one path.

## 5. Restart and load

Sources: `src/main_entry/callbacks.rs`, `src/ingame_menu/save_load.rs`,
`src/ingame_menu/save_picker.rs`, `src/game_session/terminal_debriefing.rs`,
`src/game_session/frame_simulate.rs`, `src/ingame_menu/pause.rs`.

`SaveLoadRequest` = `Save|Load|ApplyLoad|Restart|LoadRestart|Continue|
QuickSave|QuickLoad|Sherwood`. Restart snapshot requires
`restart_allowed && has_restart_save()`; click without a snapshot falls through
to `Ok`. Existing slot load uses `LoadPicker`, `SaveLoadOutcome::Cancel`
re-enters `Debriefing`, `Slot` pins `TerminalLoadSelection` then publishes.
Quick load is host-only after synchronization. Load errors stay in the picker
or log and do not load. Emergency exit sets `exit_code` and returns
`EmergencyExit`.

Tests: `terminal_picker_selection_survives_reordering_but_rejects_retirement`,
`terminal_http_selection_is_pinned_before_authority_and_leaderboard_waits`,
`terminal_remote_and_replay_indices_never_authorize_local_files`,
`client_terminal_load_keeps_simulation_paused_before_snapshot_prepare_arrives`,
`load_validation_failure_stays_in_picker_until_retry_or_cancel`,
`failed_save_drafts_can_be_retried_but_are_not_loadable`,
`autosaves_are_loadable_but_not_overwritable_or_deletable`,
`pause_picker_uses_shared_draft_autosave_and_multiplayer_policy`,
`selection_and_confirmation_survive_reordering_but_not_disappearance`,
`shared_delete_bridge_handles_cancel_success_and_cleanup_error`,
`authoritative_transition_disables_load_and_restart_until_reenabled`.

Untested: enablement gate on Restart; quick-load host-only refusal message;
`TerminalDebriefingAction::Load` restoring campaign position end to end.

## 6. Pause menu

Source: `src/ingame_menu/pause.rs`. `PauseMenuOutcome` =
`Pending|Continue|OpenOptions|OpenLoad|OpenSave|OpenSherwoodTrading|
OpenCampaignManager|OpenReportBug|Restart|Quit`. Restart disabled in Sherwood;
authoritative transition disables Load and Restart; keyboard nav skips
disabled. Tests: `pause_menu_has_eight_buttons`,
`campaign_manager_is_available_before_sherwood_and_returns_to_pause`,
`pause_menu_escape_continues`, `pause_menu_restart_disabled_in_sherwood`,
`authoritative_transition_disables_load_and_restart_until_reenabled`,
`pause_menu_keyboard_nav_skips_disabled`,
`pause_menu_report_bug_button_opens_the_report_screen`,
`pause_menu_options_button_opens_options`,
`sherwood_trading_row_is_conditional_and_keyboard_openable`,
`pause_save_cancel_continue_flow`, `reset_after_side_menu_clears_held_buttons`,
`pause_escape_bypasses_keyboard_selection`. No widget-level gap found.

## 7. Leaderboard UI

Sources: `src/ingame_menu/leaderboard.rs`, `src/leaderboard/mission_end.rs`,
`src/game_session/leaderboard_runtime/board.rs`,
`src/game_session/leaderboard_runtime.rs`.

`MissionEndLeaderboardAction` = `SelectTab|RetryBoard|SubmitThisRun|
RetrySubmission|SetAlwaysSubmitRuns(bool)|Close`; event = `None|
PreferencesChanged|Closed`. `BoardLoadState` = `Loading|Ready|Failed`;
`MissionSubmissionState` = `WonMissionRequired|Unavailable|AwaitingConsent|
Submitting|Queued|Failed`. Submit enabled only in `AwaitingConsent`; Retry
enabled on board or submission failure; Continue always closes.

Tests: `default_consent_is_off_and_explicit_consent_uploads_once`,
`dismissing_does_not_cancel_a_consented_submission`,
`always_submit_is_explicit_and_starts_only_eligible_wins`,
`closed_delayed_upload_moves_to_session_owner_without_blocking_transition`,
`boards_open_for_failed_missions_but_submission_never_does`,
`presentation_toggle_hides_only_the_board_and_does_not_invent_work`,
`shared_archive_prompt_cancels_and_retry_checks_registration_again`,
`automatic_registration_failure_stays_visible_and_never_uploads`,
`manual_and_automatic_uploads_share_registration_even_with_hidden_boards`,
`edition_mission_and_policy_must_all_admit_the_replay`,
`coop_has_its_own_combined_board`,
`fixed_boards_are_preferred_over_any_config_boards`,
`remaining_ambiguity_is_unavailable`,
`each_terminal_transition_after_a_snapshot_restart_gets_a_fresh_preparation`,
`host_coop_results_browse_all_team_sizes`,
`client_browsing_does_not_require_a_valid_local_replay_or_offer_upload`,
`coop_submission_requires_recorded_teammates`.

Untested: widget-level enablement/action mapping for Submit/Retry/
Always-submit/Close; the UI Retry action on a failed board load.

## 8. Replay paths

Sources: `src/campaign_map.rs`, `src/mission_replays/{native,browser}.rs`,
`src/game_session/{terminal_debriefing,session_policy,replay_init}.rs`.

Watch recorded play: Enter, mouse button, double-click row →
`mission_replays::watch`, statuses "Replay opened in a separate window.",
"No recording is available for this play.", or error. Browser always errors
"Local replay playback requires the desktop game." Campaign replay of a
completed mission: `MissionAvailability::Completed` / `is_mission_replayable`,
rewards unchanged. Replay-on-modal: `ReplayModalDismissals`,
`assert_consumed`. Replay continuation: `TerminalLoadTarget::ReplayContinuation`.

Tests: `submission_controls_do_not_watch_or_launch_an_unavailable_recording`,
`details_can_select_every_play_without_launching_the_mission`,
`missing_or_invalid_recordings_report_errors_without_launching`,
`a_replaced_recording_cannot_launch_a_different_play`,
`backfill_does_not_replace_a_live_attempt_link`,
`recording_identity_uses_campaign_sequence_and_terminal_nonce`,
`failed_scan_is_observable_and_can_retry_after_repair`,
`recorded_terminal_load_does_not_wait_for_a_live_snapshot`,
`terminal_remote_and_replay_indices_never_authorize_local_files`,
`playback_terminal_keeps_recorded_update_and_debrief_progression`,
`replay_terminal_requires_exactly_one_matching_campaign_update`,
`a_recorded_control_cannot_acknowledge_an_unadmitted_lane`,
`missing_control_keeps_modal_open_until_later_replay_frame`,
`replay_attachment_rejects_raw_input_before_recording_or_file_access`.

Untested: native `watch` success path from the map; full completed-mission
campaign replay entry with unchanged rewards.

## 9. Dialogs

Sources: `src/ingame_menu/{yesno,mission_state,bug_report,buy_blazons}.rs`,
`src/game_session/frame_simulate.rs`.

Yes/no: `ConfirmButtons::YesNo|YesOnly`; `YesNoChoice::Yes|No`; fly-in mission
start confirmation. Leave prompt: Won→235, Aborted/Lost→238; `YesOnly` for
`H01_Lin_VL`. `MissionStatePopupState` phases `Opening|Confirming|Closing|
Done`, returns `Some(true)` on confirm, `Some(false)` on cancel. Bug report:
`BugReportOutcome::Pending|Queued|Cancelled`; `SubmitAttempt::Queued|
QueueFailed|Unsupported`.

Tests: `return_focuses_then_confirms_on_release`,
`escape_focuses_then_cancels_on_release`,
`resolution_is_reentrant_and_first_choice_wins`,
`mission_start_fly_in_starts_off_screen_right_and_lands_centered`,
`a_frame_advances_the_mission_start_fly_in_monotonically`,
`leave_prompt_ids_are_the_retail_questions`,
`an_empty_description_is_refused_without_filing`, `escape_cancels`,
`enter_with_text_reserves_a_bundle_and_queues`,
`a_missing_endpoint_keeps_the_modal_open_and_surfaces_the_error`,
`an_unsupported_build_surfaces_an_error_instead_of_closing`,
`a_queued_report_is_distinguishable_from_a_failure`.

Untested: `YesOnly` leave prompt as a driver-level decision;
`MissionStatePopupState` confirm/cancel transition itself.

## 10. Remaining unproved

1. Restart button enablement gate (`restart_allowed && restart_snapshot_exists`).
2. Full live terminal debriefing phase chain.
3. All `drive_sherwood_modal_flow` outcome arms and non-host refusal.
4. `CampaignMapChoice::Redisplay`, "Inspect" geometry, `l`/`f` status strings.
5. Widget-level leaderboard enablement/action mapping.
6. Native `watch` success path; completed-mission campaign replay entry.
7. QuickLoad host-only refusal; debriefing `Load` end-to-end restore.
8. `YesOnly` leave-prompt driver path for `H01_Lin_VL`.
9. No test ties a debriefing answer to the mission entered next; corpus ends
   at H36 with `mission(count=2)`.
