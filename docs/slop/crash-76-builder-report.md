# crash-76-builder report — issue #76

Worktree: `/home/matt/git/robin-hood-remake/.worktrees/crash76-builder`
Branch: `agent/crash76-builder` (from `fork/main` @ `08b53515f`)
Issue: #76 "Browser full campaign loses player profile and progress on reload"
Mode: ROOT CAUSE FIRST — no fix before understanding why.

## Log

- Cut worktree from `fork/main`, read authoritative issue (0 comments), recorded D09 in `docs/DEFECTS.md`.
- Added failing check #1 (browser): `scripts/validation/campaign-persistence.mjs` (pure assertions) + `--campaign-persistence` mode in `scripts/validation/browser-game.mjs`. Local pre-fix evidence: both assertions throw — `campaign-persistence snapshot is missing profile (pre-reload)` and `campaign-persistence did not report a profile errorKind` — because the runtime does not expose the probe. Full e2e needs a gate-built artifact (director serializes the one gate).
- `node --check` passes on both check files.

## Root cause (proved from code, no fix written yet)

The browser has durable storage for both the profile and checkpoints, but nothing
pulls them back into the boot campaign, and failures are swallowed:

1. `main_entry/init.rs:542` — `Campaign::create(&profiles, …)` builds a **fresh
   campaign on every boot**. No persisted campaign is applied.
2. `main_entry/run.rs:1143-1175` — `MainMenuChoice::Start` *can* resume the
   latest checkpoint (`find_resume_target()`), but only after the player is
   already at the main menu. There is no boot-time auto-resume and no dedicated
   Continue button: `main_menu/mod.rs:64-84` and `:418-432` have `Start`,
   `LoadGame`, `CampaignManager`, `CustomMissions` — no `Continue` action.
3. `main_entry/callbacks.rs:553-559` / `savegame/writes.rs:644-664` — the
   browser rejects the "Continue" special-save write
   (`browser manual special-save persistence is unavailable; use durable
   autosaves`), so a reload before the next periodic autosave
   (`autosave.rs:34`, 5 minutes) has no checkpoint to resume.
4. `main_entry/callbacks/profile.rs:93-97` and `:125-141` — on wasm a failed
   profile/campaign-history publication is only `tracing::warn!`-logged and the
   change is retained **in memory** (`application.rs:735-742`,
   `RetainOnFailure`). It is lost on reload.
5. `main_entry/init.rs:698-731` — on any load error the code silently
   regenerates default profiles and overwrites the archive
   (`:738-762`), destroying the last good copy — a silent reset to zero
   progress (criterion 4).
6. The profile archive is one uncompressed JSON envelope in localStorage,
   capped at 4 MiB (`player_profile_store.rs:500,517-533`;
   `player_profile_store/browser.rs:7,38-48`). Measured against the real native
   archive `~/.local/share/robin_hood/saves/profiles.json` = 4151 bytes with
   **zero** attempts; each attempt adds roughly 0.5-1 KiB, so a full campaign
   (~tens of attempts) is far below 4 MiB. The conditional IndexedDB move in
   criterion 4 is therefore not triggered by normal play; oversize must still
   be reported explicitly (the check asserts this).

Fix: boot-time auto-resume from the durable checkpoint, an explicit main-menu
Continue action, explicit reporting + preservation of rejected profile archives,
and a non-sensitive `campaign-persistence` RPC probe for the check.

## Implementation review (verified against authoritative code)

- `main_entry/run.rs` — wasm-only preflight in `run_main_menu` calls
  `RustCallbacks::new_for_window`, `find_resume_target`, `slot_name` and
  `slot_mission_id`, then `menu.launch_session(..., Some((slot, mission_id)))`
  before the menu loop: a reload resumes from the durable checkpoint.
- `main_menu/mod.rs` — `ClickAction::Continue` returns `MainMenuChoice::Load`
  from the same resume target; no checkpoint means "stay on menu", never a
  fabricated campaign. Uses the existing `MT_BTN_CONTINUE` label.
- `main_entry/init.rs` / callbacks — rejected archives are reported via
  `campaign_persistence::report_failure` with a stable kind
  (`classify_profile_rejection`: limit → oversize, unavailable/window →
  unavailable, else corrupt) and `regenerate_default_player_profiles` returns
  before overwriting durable bytes. A missing archive is still written by
  `PlayerProfileStore::load` before that path is reached, so no legitimate
  first launch is starved.
- `campaign_persistence.rs` / `wasm_rpc.rs` — the RPC reads localStorage
  directly and reports only digests, counts, byte sizes and error kinds; no
  name, save payload, credential or origin password is exposed.
- Two coherence fixes during finalization: the stale doc comment on
  `regenerate_default_player_profiles` now describes the preserve-on-reject
  behaviour; a dead, misleading history-count branch in
  `assertCorruptionReported` was removed (the probe reports no count for a
  rejected archive by design). Neither changes the asserted contract.

## ID correction

The #76 row in `docs/DEFECTS.md` was renumbered `D09` → `D10` (the only edit
to that row). `fork/main` has since claimed `D09` for the #75 deferred
load-flake row; keeping ours at `D09` would conflict on `scripts/land.sh`'s
rebase. Row status remains `open`.

## Local verification (non-gate, this host)

```
node --check scripts/validation/campaign-persistence.mjs   -> PASS
node --check scripts/validation/browser-game.mjs           -> PASS
node --input-type=module -e '<inline import of ./scripts/validation/campaign-persistence.mjs>'
```

Inline pure-assertion outcome (`pass=6 fail=0`):

| case | expectation | result |
| --- | --- | --- |
| `assertPersistenceSurvivesReload({}, {})` | throws `missing profile (pre-reload)` | PASS |
| snapshots without `resume` | throws `missing resume (pre-reload)` | PASS |
| coherent stable before/after pair | returns summary, `attemptsAfter=3`, `resumeAvailable=true` | PASS |
| `assertCorruptionReported` with no `errorKind` | throws `did not report a profile errorKind` | PASS |
| `assertCorruptionReported(..., 'corrupt')` | returns `{errorKind:'corrupt'}` | PASS |
| `assertCorruptionReported(..., 'oversize')` | returns `{errorKind:'oversize'}` | PASS |

No build-host gate was started (director serializes the one gate); no
`cargo build/test/check/clippy` was run on this host. The full e2e reload
check and the wasm unit test still need the gate-built artifact.

- STATUS: READY_FOR_REVIEW
