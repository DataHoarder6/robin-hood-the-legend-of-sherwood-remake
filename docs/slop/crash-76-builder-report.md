# crash-76-builder report — issue #76

Worktree: `/home/matt/git/robin-hood-remake/.worktrees/crash76-builder`
Branch: `agent/crash76-builder` (from `fork/main` @ `08b53515f`, rebased onto `fork/main` @ `f65fa257b`)
Issue: #76 "Browser full campaign loses player profile and progress on reload"
Fix commit (rebased): `1e6c78ed6371fa35bc9be996d0efdd113d03445d`

## Log

- Cut worktree from `fork/main`, read authoritative issue (0 comments), recorded the D09 row from the dispatch verbatim.
- Added failing check #1 (browser): pure assertions in `scripts/validation/campaign-persistence.mjs` plus a new `--campaign-persistence` mode in `scripts/validation/browser-game.mjs`. Local pre-fix evidence: both assertions throw — `campaign-persistence snapshot is missing profile (pre-reload)` and `campaign-persistence did not report a profile errorKind` — because the runtime has no such surface. `git grep` against `fork/main` confirmed no `campaign_persistence` / `campaign-persistence` reference existed in the RPC, module table or wasm test; post-fix they exist in all three. Full e2e needs a gate-built artifact (the director serializes the one gate).
- `node --check` passes on both check files.
- Added failing check #2 (owning wasm test): `campaign_persistence_probe_distinguishes_retained_progress_from_rejection` in `player_profile_store/browser.rs`, covering retained attempts, identity/config digests, corrupt and oversize reporting, and the observable explicit-failure surface. Pre-fix this test cannot compile (the `campaign_persistence` module and `probe()` do not exist); that absence was verified against `fork/main`.
- Measured the archive: the real native profile archive `~/.local/share/robin_hood/saves/profiles.json` is 4151 bytes with zero attempts; per-attempt records are ~0.5-1 KiB, so a full campaign is far below the 4 MiB envelope. The conditional IndexedDB move in criterion 4 is not triggered by normal play; oversize is nevertheless reported explicitly. `TODO(76)`: if a long-running origin ever does exceed 4 MiB, move the campaign archive to IndexedDB using the existing `indexed_db_executor` rather than trimming history.
- Rebased onto `fork/main` (which had gained six unattended-loop commits). Upstream had already allocated D09 to the full-corpus flake defect (#75), so this issue's row was renumbered **D10** and its check column updated to name the actual owning test. Status stays `open` for the director to finalize after a gate run.

## Root cause (proved from code, no fix written before the checks)

The browser has durable storage for both the profile and checkpoints, but
nothing pulls them back into the boot campaign, and failures are swallowed:

1. `main_entry/init.rs:542` — `Campaign::create(&profiles, …)` builds a **fresh
   campaign on every boot**. No persisted campaign is applied.
2. `main_entry/run.rs:1143-1175` — `MainMenuChoice::Start` *can* resume the
   latest checkpoint (`find_resume_target()`), but only after the player is
   already at the main menu. There was no boot-time auto-resume and no dedicated
   Continue button: `main_menu/mod.rs:64-84` and `:418-432` had `Start`,
   `LoadGame`, `CampaignManager`, `CustomMissions` — no `Continue` action.
3. `main_entry/callbacks.rs:553-559` / `savegame/writes.rs:644-664` — the
   browser rejects the "Continue" special-save write
   (`browser manual special-save persistence is unavailable; use durable
   autosaves`), so a reload before the next periodic autosave
   (`autosave.rs:34`, 5 minutes) had no checkpoint to resume.
4. `main_entry/callbacks/profile.rs:93-97` and `:125-141` — on wasm a failed
   profile/campaign-history publication was only `tracing::warn!`-logged and
   the change retained **in memory** (`application.rs:735-742`,
   `RetainOnFailure`). It was lost on reload.
5. `main_entry/init.rs:698-731` — on any load error the code silently
   regenerated default profiles and overwrote the archive (`:738-762`),
   destroying the last good copy — a silent reset to zero progress (criterion 4).
6. The profile archive is one uncompressed JSON envelope in localStorage,
   capped at 4 MiB (`player_profile_store.rs:500,517-533`;
   `player_profile_store/browser.rs:7,38-48`).

## Fix summary

- **Boot auto-resume** (`main_entry/run.rs`): on wasm, before the menu loop,
  open the durable save manager and launch the same preflighted `Load` used by
  the menu's Start entry when a compatible checkpoint exists. Reload therefore
  resumes the saved mission and campaign instead of showing a new game.
- **Visible Continue action** (`main_menu/mod.rs`): a `Continue` button using the
  menu text resource resumes the latest checkpoint via `MainMenuChoice::Load`;
  when no compatible checkpoint exists it stays on the menu rather than
  fabricating a new campaign.
- **Explicit rejection, no silent reset** (`main_entry/init.rs`,
  `main_entry/callbacks.rs`, `main_entry/callbacks/profile.rs`): corrupt,
  oversize and unavailable archives are classified into stable kinds and
  reported via `campaign_persistence::report_failure`. `regenerate_default_player_profiles`
  takes a `preserve_rejected` flag; for a rejected archive it does **not**
  overwrite the durable bytes with defaults. A valid load clears the report.
- **Non-sensitive probe** (`campaign_persistence.rs`, `http_server/wasm_rpc.rs`,
  `http_server/diagnostics.rs`): a new queue-free `campaign-persistence` RPC
  reports storage kind, byte size, schema, digests of identity/config, attempt
  count, resume availability/kind and the last explicit failure. It returns no
  name, save payload, credential or password, so `--evidence` stays safe.

## Local evidence (post-fix)

```
NODE_SYNTAX_OK
PRE_FIX_FAILURES=2 POST_FIX={"identityStable":true,"configStable":true,"attemptsBefore":2,"attemptsAfter":3,"resumeAvailable":true,"resumeMissionId":7,"archiveBytesAfter":null} {"errorKind":"corrupt","archiveBytes":10} {"errorKind":"oversize","archiveBytes":4194305}
rustfmt --check --edition 2024 <all 10 changed .rs files>: exit 0
git rev-list --left-right --count fork/main...HEAD  ->  0 1
```

Full wasm/e2e suites were intentionally not started: the runtime guard refuses
raw cargo build/test on oci-vm and the director owns the single build-host gate.
The gate must run `format`, the wasm test suite and
`--campaign-persistence` before D10 is set to `fixed`.

## Review round 1 — fixes

Two read-only reviewer findings against the authoritative code, both fixed:

1. **Continue advertised with no checkpoint** (`main_menu/mod.rs`): the button
   was built with `(label, true)` for every entry, so a no-checkpoint menu
   showed a clickable no-op. `main_menu_layout` is now built after the save
   manager exists and takes `&SaveGameManager`; it derives `continue_enabled`
   from the read-only `find_resume_target()` and disables only that widget
   (disabled widgets cannot be mouse- or keyboard-activated). The click handler
   still re-reads the authoritative target. Presentation only reads state.
2. **Raw error text logged** (`main_entry/init.rs`,
   `main_entry/callbacks/profile.rs`): `validate_archive()` can embed the raw
   profile name. The rejected-archive `reason` and both profile/campaign-history
   persistence logs now carry only the stable `classify_profile_rejection` kind,
   so no profile name reaches a log used by the check (criterion 6).
   `report_failure`'s shape is unchanged; no test asserted the old strings,
   so no test edits were needed.

Also fixed a self-discovered native regression from the original fix:
`preserve_rejected` is now `cfg!(target_arch = "wasm32")`. Browser archives are
still preserved on rejection (criterion 4); native keeps its original
self-healing overwrite, so the pre-existing
`semantically_invalid_player_archive_is_regenerated_and_persisted` test keeps
passing.

Local evidence (non-gate, this host, after the fixes):

```
node --check scripts/validation/campaign-persistence.mjs -> PASS
node --check scripts/validation/browser-game.mjs         -> PASS
node --input-type=module -e '<inline assertions>'        -> pass=6 fail=0
```

Cases: empty snapshots throw `missing profile (pre-reload)`; snapshots without
`resume` throw `missing resume (pre-reload)`; a coherent stable pair passes;
missing `errorKind` throws; `corrupt` and `oversize` each return their kind.
No cargo/gate/push/land on this host. A rustfmt-only hunk in the
`callbacks/profile.rs` test module was applied by the pre-commit formatter and
is included in this follow-up.

STATUS: DONE_WITH_CONCERNS
Commit: 1e6c78ed6371fa35bc9be996d0efdd113d03445d (fix); review fixes 851869919, 22a596f9d; ledger wording 8b855d491
Evidence: `node --check` on both mjs files passed; reload assertions fail pre-fix (2/2) and pass post-fix (6/6); rustfmt --check on all changed Rust files exit 0.
