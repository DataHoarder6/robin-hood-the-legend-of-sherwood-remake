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
| D10 | data | on the private browser origin the full campaign boots fresh on every page load: the previous page's player profile (name/config) and campaign progress are absent, so no Continue restores them (#76) | start a full-campaign mission on robin.jautis.lt, reach a checkpoint/save, reload the same origin | new reload check in scripts/validation/browser-game.mjs: start a mission, reload, assert digests of the profile name/config and the campaign mission/attempt state survive; owning wasm test campaign_persistence_probe_distinguishes_retained_progress_from_rejection (player_profile_store/browser.rs) | open |
| D11 | release | the second fix attempt for H38's scripted-victory discriminator cannot be proved: the fix's own `natives::tests::harness_mark_mission_won_stamps_the_native_path_marker` captures no marker from its scoped tracing subscriber (`native must stamp the path-unique marker, got:`), and the `engine` suite also fails the pre-existing `engine::movement::tests::compiled_navigation::exported_stairs::arbitrarily_rotated_stairs_support_complete_actor_routes` (`assertion failed: self.is_increment_map_computed()`), so the gate is red and issue #49 criterion 2 stays unproved ([#77](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/77)) | `runbg-c2m3-fix2-gate.service` at fix2 tip `aabe3cc6a`: `bash scripts/remote-check.sh --suites format lint check client-release engine tooling docs` from `director-c2-m3-fix2`, 2026-11-23 05:16–06:15 EET, rc 101; `engine` suite 4498 passed / 2 failed | the `engine` suite must pass with both tests green; H38's `log(contains=harness native victory path: HarnessMarkMissionWon yielded the scripted Win command,count=2)` stays unproved until then | deferred |

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
