# Mission end: which screens appear, and who can end the application

Written 2026-11-18 while investigating #47 (finishing a mission exits the whole
game) and #48 (a lost mission asks Yes/No before the debriefing). The page
sequence itself, with the retail text ids, is
[mission-end pages](MISSION_END_PAGES.md); this file covers the terminal
narrative order and every path that ends the process.

## The sequence a single-player mission end runs

1. The tick ends the mission and stages a terminal campaign update
   (`game_session/tick.rs`, `stage_terminal_campaign_update`).
2. The next frame constructs `TerminalDebriefingState`
   (`game_session/terminal_debriefing.rs`). Its first phase is
   `Debriefing(DebriefingModalState)` — the mission-end parchment, which carries
   the won/lost/aborted title itself (`MissionEndKind::title_id`).
3. The player's dismissal is one `ModalDismiss` command; it settles the
   `TerminalDecisionOrder` (`game_session/session_policy.rs`) and pushes the
   decision into `SettledDebriefingOutcome`.
4. `LeaderboardPendingStart` → `AwaitingLeaderboard`: an `ActiveUiTask::
   MissionEndLeaderboard` task runs.
5. `apply_terminal_debriefing_action` maps the settled decision:
   `Ok → Continue`, `Restart → LoadRestart`, `Load{slot} → Load`,
   `EmergencyEnd → EmergencyExit`.

Until #48 there was an extra step between 1 and 2: a `MissionStatePopupState`
title card asked Yes/No first. Both answers ran `record_popup_decision`, which
always began the debriefing — the question had no consequence and the page it
led to repeated the same title. It is gone; `MissionStatePopupState` remains only
as the rail's abandon confirmation, where the two answers differ.

## Everything that ends the process

| Path | Trigger | Log line today |
| --- | --- | --- |
| Window close on the debriefing page | `GameEvent::Quit` → `ScreenKey::Quit` → `PageOutcome::EmergencyEnd` (`ingame_menu/debriefing.rs`) | none |
| Debriefing dismissed as Aborted | `DialogResult::Aborted` → `DebriefingOutcome::EmergencyEnd` (`game_session/modal_state.rs`) | none |
| Terminal flow reports `EmergencyExit` | step 5 above (`frame_simulate.rs`) | none |
| Window closed while the session was being built | `window.close_requested` → `SessionResult::ExitRequested` (`game_session/mod.rs`) | none |
| Window closed inside a pause/modal UI task | `UiTaskOutcome::ExitRequested` (`game_session/ui_task_state.rs`) | none |
| Direct `--mission` or replay run | `run.rs` returns `Ok(0)` after one mission | none |

`main_entry/run.rs` is the single place that turns these into "stop the menu
loop", and it logs nothing. The last line of a run that ended this way is
`game future returned, exit_code=0` — which is also the last line of a normal
quit. A report that says "the game turned off" therefore cannot be diagnosed
from the log after the fact; that is how #47 reached "looks like a crash" with
an orderly exit code. Naming the cause at each of those boundaries is the open
work.

## Measured live (build host, current tree, real data)

- `robin --mission Emb01_FoA_EC` (the ambush) under Xvfb, driven over the JSON
  surface: a synthetic `Return` dismisses the entry briefing, the mission runs.
- `POST /command '"QuitMissionRequested"'` gives `LevelInterrupted`, the
  debriefing opens, and **the process stays alive** waiting on the page. So the
  interrupted path does not exit by itself; the player's log reaches the exit
  only after `terminal flow handing presentation to mission-end leaderboard`.
- `GET /screenshot` **never answers** once the mission has ended. Screenshots
  are drained only from the mission-play render path (`game_session/flow.rs`)
  and from pause-side UI tasks (`frame_simulate.rs`); the terminal flow renders
  its page but never drains a presented-UI capture, so a request sits in ingress
  until the client disconnects. An agent cannot photograph a finished or stuck
  client — the exact state a bug report is about.

## Open questions this doc does not answer

- Which exit path the player's session took. It needs one run whose exit states
  its reason.
- Whether the leaderboard task can settle `EmergencyEnd`. Reading
  `game_session/leaderboard_runtime.rs`, a task failure surfaces as a
  `RankedError`, not as an outcome; nothing there sets `EmergencyEnd`.
