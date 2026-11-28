# Issue #86 — first-run crash: regression exists, live repro does not

Issue #86 reports that pressing Play with a newly created character crashed
immediately on the owner's laptop, stable native `0.4.0+08b53515f5d0` (the
v0.4.0 tag). It also says no replay and no crash-receiver entry was produced
for that run, so the failing path was not identified from the player's
evidence. Last verified: 2026-11-26.

## What is in the tree

`crates/robin_rs/src/main_menu/player_select.rs` carries
`new_character_play_path_resolves_unresolved_camps_without_a_diplomacy_query`
(commit `b5ffd3320`): first-launch profile replacement commits a playable
profile and its difficulty, and a fresh engine answers the unresolved
`Camp::Error` sentinel without reaching the strict diplomacy guard. The guard
itself and its unit test predate that commit; the added test binds the whole
new-character Play path to them.

That is a regression test for the diagnosed guard, not a live reproduction of
the 0.4.0 crash: it runs in a unit-test process, not on the owner's stable
install with the full-game data root. The test's comment cites a 0.4.0 crash
bundle naming the diplomacy guard; the issue text says no crash-receiver entry
was produced. Both statements are recorded as given; this page does not
reconcile them.

## Exact-tree regression status

On the landed tree `9e09789bbdef019ef689f7739fc5c4cbff3a7e87` the named test
`main_menu::player_select::tests::new_character_play_path_resolves_unresolved_camps_without_a_diplomacy_query`
passed on the build host (`runbg verify86-current2`, 1 passed, 0 failed, exit
0). Issue #86 was closed on that evidence; no live repro was claimed or
supplied. Last verified: 2026-11-28.

## Exact live repro needed

On the owner laptop, with the stable 0.4.0 install and the full-game data root:

1. Start from a fresh install home (or move the existing one aside) so the
   first-launch profile path runs.
2. Set `ROBINHOOD_DATA_DIR` to the full-game root, create a new character, press
   Play, and keep the stderr/run log.
3. If it crashes, retain the crash bundle under the install home's `crashes/`
   and the session replay under `~/.local/share/robin_hood/replays/`, then run
   `scripts/triage_bug_bundle.py <bundle>` and record a DEFECTS.md row (class,
   reproduction, failing check) before a fix.

The runtime here cannot perform steps 1–3: the stable native install is on
`laptop` (and the Windows seat in [PLAY.md](../PLAY.md)), not on this host. The
replays in this runtime's `~/.local/share/robin_hood/replays/` are earlier
session recordings, not an owner-supplied #86 repro; no full-game data root is
present here either. No live repro is claimed.
The regression test remains the only check bound to the issue until the owner
supplies a bundle or a replay.
