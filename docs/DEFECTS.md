# Defect ledger

One row per diagnosed defect, in the order it was diagnosed. A row is the unit
of work: "fix all the bugs" is not a task, "empty this table" is.

This file is the index, not the evidence. A row links to the document or test
that owns the detail; the audits (`docs/slop/multiplayer-unbounded-waits.md`),
the harness matrix (`docs/multiplayer/HARNESS.md`) and the session handovers
(`docs/HANDOVER.md`) stay the place where a class of defect is described. A row
that repeats their content is a second source for one fact.

## Classes

Every row names one. The class decides the shape of the fix, which is why it
comes before the patch:

| Class | The defect is | The fix is a mechanism, not a patch |
| --- | --- | --- |
| `wait` | progress depends on a message a peer may never send | a deadline with a named consequence (`docs/slop/multiplayer-unbounded-waits.md`) |
| `authority` | two participants author the same state | one author per event, replicated as a command |
| `desync` | two simulations disagree at a frame | deterministic tick, one RNG source, per-frame state hash |
| `generation` | a transition applies to a stale transport generation | a sequence on every transition; stale ones are refused |
| `identity` | two participants disagree about what session they are in | refuse at join with a named reason; adopt host-authored context |
| `presentation` | the screen and the simulation disagree about a page | presentation reads state, never mutates it |
| `release` | the artifact is wrong, not the code | a build step that proves the artifact before it ships |
| `data` | the content is wrong (mission, asset, save) | a check over the data, not a runtime guess |

## Statuses

`open` — reproduced, no fix. `fixed` — the check exists and fails without the
fix. `deferred` — reproduced, deliberately not fixed yet; the row names the
issue. `out-of-scope` — the behaviour is the original's, or the platform's.

## Rows

| id | class | symptom | reproduction | check | status |
| --- | --- | --- | --- | --- | --- |
| D01 | release | the shipped Windows exe exits `0xc0000005` before `main`, even for `--version`; MinGW wrote TLS `SECREL32` offsets as `.text` base relocations | ship a `windows-gnu` release and run it on Windows | `native-release.yml`: the built exe starts and carries no `.text` base relocations (`docs/WINDOWS.md`) | fixed |
| D02 | identity | a client reconnecting across a mission start was refused as an incompatible seat, so every session died at the first win or loss | finish a mission while a client is reconnecting | `reconnect_rejects_identity_change_and_adopts_host_context` (`docs/HANDOVER.md` §5) | fixed |
| D03 | generation | a snapshot-transition prepare was applied to the transport generation that did not receive it, so a lost commit suppressed the replacement mission's `begin-sim` and the restart deadlocked | drop a client's `CommitSnapshotTransition` at the `transition-commit` barrier, then restart the mission (#43) | `H20` | fixed |
| D04 | presentation | the Sherwood perks panel showed the literal `%i` as the clover price when game data was loaded, because the row and balance lines used the `%u/%d/%s`-only trading substitute on `"Amulets: %i"` | launch with full-game data (`Level.res` ships `"Amulets: %i"`; the Rust fallback ships `%d`) and open the Sherwood perks panel | `perk_row_renders_numeric_cost_from_the_loaded_i_template` and `balance_renders_numeric_clovers_from_the_loaded_i_template` (`client`) | fixed |
| D05 | wait | every process-layer multiplayer scenario hung waiting for the host's connect string after children moved into systemd service units: the driver polls loopback HTTP inside its `unshare --net` namespace, but a systemd service starts in the manager's namespace, so `/state` was unreachable and no sample was ever recorded | run `multiplayer-e2e` on `fork/main` after `b5ccc6bdd`; the host logs show the HTTP server listening and a live iroh endpoint, while `samples/host.jsonl` stays empty | `supervised_test.py`: the child command uses `--scope` (caller's namespace) and carries the TTL and control-group kill; the gate reaches `observed` for the connect string | fixed |
| D06 | generation | the joining client aborted with "mission-end leaderboard was captured more than once" (`terminal_debriefing.rs:1099`) on the second terminal transition after a host mission-result + restart fault: the one-shot `MissionLeaderboardRuntime::preparation` was captured on the first mission end and never re-armed for the replacement mission, so `capture_terminal` panicked and the process aborted | run the full `multiplayer-e2e` gate under load; H23 fails with the client at SIGABRT while H23 alone passes | `each_terminal_transition_after_a_snapshot_restart_gets_a_fresh_preparation` and `H23`: a unit test that drives two terminal transitions through a snapshot restart and asserts each capture gets a fresh preparation; H23 observed in a full gate run — scripts/release-gate.sh on eafb7b915261, evidence minipc:~/e2e-gate/release-eafb7b915261/summary.json (`.verdict` and `.verdicts.H23` observed, 17 process rows selected) | fixed |
| D07 | generation | on a won campaign mission both seats reach the debriefing and the host commits one `CampaignExit`, then starts mission 2 at Nottingham in the same process; the client drops the prepared transition because its transport generation ended and is never re-admitted, so it stays on `lincoln` with one desync and exits 0 instead of entering the second mission | `python3 scripts/validation/multiplayer_e2e.py --only H36`; evidence `/home/admin/campaign-e2e-H36-20261121-234314` — `H36/1/events.jsonl`, `H36/1/logs/host.log`, `H36/1/logs/peer1.log`, `H36/1/summary.json` | `H36`: `mission(count=2)`, `hash-equal(every=25)`, `exit(running)`; `campaign_endpoint_survives_mission_rotation_and_frozen_addr_reconnects` (`multiplayer/native/server/tests.rs`) | fixed |
| D08 | release | the web build was silent: a browser datadir converted with `--web-content-manifest` but without `--audio-format opus` embeds source audio and has no `audio_assets`, so warmup sees `items = 0` and every Web Audio request is rejected (#54) | `convert_datadir --format shipping --web-content-manifest --web-content-edition demo` with default audio, then boot the browser build | `web_content_manifest_refuses_source_audio` (`tools`); canonical recipe in `scripts/build_web_shipping_datadir.sh` | fixed |
| D09 | release | known full-corpus load flakes: `H14` fails at `--jobs 2`, `H28` and `H33` fail at `--jobs 1`, and each passes when run in isolation — the artifact under load is wrong, not the code ([#75](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/75)) | full corpus run on the build host (`multiplayer-e2e`) | isolated row recheck (`--only H14`, `--only H28`, `--only H33`) | deferred |
| D10 | data | on the private browser origin the full campaign boots fresh on every page load: the previous page's player profile (name/config) and campaign progress are absent, so no Continue restores them (#76) | start a full-campaign mission on robin.jautis.lt, reach a checkpoint/save, reload the same origin | new reload check in scripts/validation/browser-game.mjs: start a mission, reload, assert digests of the profile name/config and the campaign mission/attempt state survive, plus a live mission map/frame after reload; owning wasm test campaign_persistence_probe_distinguishes_retained_progress_from_rejection (player_profile_store/browser.rs); pure-contract test scripts/validation/campaign-persistence.test.mjs in the `tooling` suite, full browser evidence /home/admin/d10-c8-evidence-20261123-080234 (stateAfterReload map=leicester frame=0, corruptProof present) | fixed |
| D11 | release | the named discriminator test `natives::tests::harness_mark_mission_won_stamps_the_native_path_marker` is absent from `fork/main`, so the row's old `fixed` check cannot be claimed; #77 is closed and H38 was later observed at `5aab6dccc`, but no gate at the current integration ref records the discriminator | `rg -n harness_mark_mission_won_stamps_the_native_path_marker crates/` returns no match at `6a86371f3930`; `python3 scripts/validation/scenario_corpus.py --check` passes with the H38 marker assertion present | gate-backed check: H38's `log(contains=harness native victory path: HarnessMarkMissionWon yielded the scripted Win command,count=2)`, which the corpus matrix now declares; it stays deferred until a gate at the exact tree observes that row — the older evidence `minipc:~/h38-clean-verify-retry2-20261123-082822/attempt3/summary.json` is not at this ref | deferred |
| D12 | desync | two native seats desync at frame 50/175 after `TeleportSelectedToPoint` is injected at frame 40; `H36` without teleport has zero desyncs; teleport alone with `/mission-result` reproduces it 3x | `python3 scripts/validation/multiplayer_e2e.py --only H38` on the build host; evidence minipc:~/h38-teleport-only-20261123-021825, minipc:~/h38-explore-20261123-015956 | Two non-ignored tests in `crates/robin_engine/src/engine/rollback_safe/tests.rs` (`engine`) apply the same host-authored teleport at target frame 42 on one engine and a frame late on the other, then assert full `EngineInner::state_hash` equality at absolute frame 50: `teleport_fade_state_matches_at_target_frame_despite_one_frame_apply_lag` (one PC) and `two_pc_teleport_matches_at_target_frame_despite_one_frame_apply_lag` (two PCs). The two-PC check is red on round 1's tree `99832e64e` (9397893188934422043 vs 12246651237733334055; minipc:~/d12-a2-red.log), green after round 2 (2 passed, 0 failed; minipc:~/d12-a2-final-green.log). Full `engine` suite green on the fixed tree: 4501 passed, 0 failed. `H38` is not a D12 check — it holds frame skew at zero and passes with D12 present. Mechanism and remaining risk: [docs/slop/teleport-late-input-desync.md](slop/teleport-late-input-desync.md); tracking [#83](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/83). | fixed |
| D13 | presentation | the Debriefing restart seal was drawn and clickable with `restart_allowed=true` even when no restart snapshot existed, so clicking it silently advanced to the stat panel instead of restarting | open a Debriefing with `restart_allowed=true` and no restart save, click Restart | `restart_button_without_snapshot_does_not_offer_restart` (`client-release`) | fixed |
| D14 | release | the editor verify suite fails at type-aware oxlint with ~36 pre-existing errors once its formatter drift is cleared; the errors precede this branch and are not caused by the loop work ([#95](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/95)) | `bash scripts/check-quality.sh editor` on `fork/main` | the editor suite passes `fmt:check`, `lint`, `test` and typechecks with pinned pnpm 12.3.4 / oxlint 1.85.0 | deferred |
| D15 | presentation | after every mission the browser shows a "Leaderboards unavailable" notice on the end-of-mission screen: at mission-end no replay snapshot bytes are available to encode, and the preparation error becomes that notice ([#79](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/79)). native runs that likewise publish no replay bytes at mission-end now skip the board silently too — they previously showed the notice; a poisoned spool still reports its real error | win or lose a mission on robin.jautis.lt with "Show mission-end boards" on; the debriefing hands presentation to the mission-end leaderboard task and the notice appears | `no_replay_export_finishes_mission_end_leaderboard_without_notice` exists in `crates/robin_rs/src/game_session/leaderboard_runtime.rs` and asserts `Finished` plus no `owns_presentation()`; it is a source check, not a gate observation at this ref, so the row stays open until the `client` suite is green on the exact tree | open |
| D16 | desync | a late `TeleportSelectedToPoint` rewind could reclaim only part of the frame fallback tail, leaving the target-frame fallback owned by another actor; both seats panicked at `sequence/selection.rs:420`, windows vanished and H38 froze ([#94](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/94)) | H38 evidence `minipc:/home/admin/e2e-gate/release-b07c1de25435/H38/1/`, panic `target-frame fallback SequenceId(96) is not owned by Pc(PcId(126))` at frame 42 | `sequence::tests::late_teleport_rewind_reclaims_another_actors_wait_in_the_tail` plus the two D12 convergence tests; full `engine` suite 4506 passed, 0 failed | fixed |
| D17 | presentation | the erase-profile confirmation can freeze a stale main-menu composite after the player visits another main-menu modal first, so the dialog shows the wrong background ([#117](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/117)) | open a main-menu modal, return to player selection, then open erase confirmation | renderer-level regression: pre-seed a stale frozen scene, draw player selection, and assert the next modal backdrop is the selection composite | open |
| D18 | data | restoring a snapshot can fail world preflight when an entity names a valid sprite profile that was not preloaded by the campaign-peasant path ([#90](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/90)) | load the #90 Sherwood save whose entity 51 names `RobinHood/Robin des bois`; crash bundle `7c0fcc2e9196b6ce` | snapshot restoration preloads exactly the serialized sprite keys and succeeds for a present profile; a genuinely absent profile still fails explicitly | open |
| D19 | generation | selecting a different mission while a completed-mission practice return is pending records an ordinary campaign attempt against the wrong checkpoint and panics ([#92](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/92), [#93](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/93)) | start practice for mission A, select mission B before the practice terminal boundary, then finish B | selecting B returns a named refusal and leaves campaign/practice state unchanged; the original practice mission still restores exactly once | open |
| D20 | release | before this batch, the private browser root requested `/campaign.json?build=<short>` but deployment neither staged nor gated the operator-managed licensed-corpus binding, so the live site returned 404 ([#127](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/127)) | deploy a browser build with the full private corpus and load `/` | `wasm-www/scripts/static-deployment-workflow.test.mjs`; operator contract in [`docs/slop/browser-android-private-campaign.md`](slop/browser-android-private-campaign.md) | open |

## How a row gets here

1. A session ends with evidence: a `bug-reports/<utc>-<origin>-<n>/` bundle, a
   desync report, or a log tail. `scripts/triage_bug_bundle.py <bundle>`
   prints the row to paste and the `--replay` command that reproduces it.
2. The class is chosen from the table above, by what the evidence shows, not by
   where the code happens to be.
3. The check is written before the fix, and it fails. A harness row (`Hnn`) is
   preferred when the defect is a session; a named test when it is one process.
4. `fixed` is claimed only when the check is in a suite a gate runs — the push
   gate or the release gate (`scripts/release-gate.sh`, `docs/RELEASES.md` step
   4). A full-run harness row such as H23 is proved by the release gate; record
   its `summary.json` path in the row.

A defect with no reproduction stays `open` and says so; it does not get a
"probably fixed" row.

## What this file is not

- Not the wait audit: `docs/slop/multiplayer-unbounded-waits.md` owns which
  waits exist and which budgets they carry.
- Not the coverage matrix: `docs/multiplayer/HARNESS.md` owns which transition
  and fault each scenario proves.
- Not the issue tracker: a `deferred` row names its issue, and the issue keeps
  the discussion.
