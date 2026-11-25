# Capable director Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the unattended director finish development and cut consecutive MINOR releases in one bounded run, with selection that strictly advances and a release tail that is idempotent.

**Architecture:** Add an atomic handoff ledger plus machine-checked selection/writeback; drive fresh-context ticks from one `runbg` command. Repair gate environment parity and path coverage so the verifier can trust the suites it runs. Add a human-landed authority amendment and two scripts (next-MINOR preparation, release orchestration) that close the native/web release chain without new mid-run decisions.

**Tech Stack:** Bash + `flock`/`jq`/`gh`, Python fixtures, Nix devShell, GitHub Actions, `scripts/remote-check.sh` gate on `minipc`.

**Spec:** `docs/superpowers/specs/2026-11-25-capable-director-design.md`

## Global Constraints

- One gate in flight; land with `runbg run NAME -- bash scripts/land.sh`; no `git stash`; commit by pathspec.
- Work in `.worktrees/loop-resilience` off `fork/main`; never commit on the shared `main`.
- Director `edit` denies stay on `.github/workflows/*`, `scripts/land.sh`, `scripts/remote-check.sh`, `scripts/release*.sh`, `*.nix`, secrets.
- D09 and D11 remain `deferred`; no rerun closes them. Retries classify only, never turn red green.
- MAJOR bumps, tag moves, machine pin, seats, credentials, workflow/Nix edits stay human-owned.
- Every verification reports an effect: test verdict, gate summary, artifact SHA or remote read-back — never a bare exit code.

## Review Focus

| Input / condition | Expected behaviour | Owning task test |
| --- | --- | --- |
| `deferred` defect row as first fixed-order row | Skipped; next admissible assertion selected | T2 `deferred-row-is-inadmissible` |
| Handoff in terminal `LANDED` state with `next_action: recompute` | Not resumed as stale work; cursor honoured | T1 `terminal-handoff-does-not-resume` |
| `gh` auth/network failure | Named error, exit nonzero, no fallback selection | T2 `gh-failure-fails-closed` |
| Writeback with several `#NNN` in handoff | Targets explicit `issue` field only | T3 `explicit-issue-wins-over-hashtag` |
| Suite invoked without `nextest`/`wabt` | Named preflight error before compilation | T6 `missing-tool-preflight` |
| Existing tag after partial release | Re-verify and re-dispatch, never move tag | T11 `tag-exists-is-idempotent` |

---

### Task 1: Atomic handoff ledger

**Files:** Create `scripts/director-state.sh`, `scripts/test_director_state.sh`; modify `docs/UNATTENDED.md` (handoff schema section).

**Interfaces:** Produces `director-state.sh claim <handoff> <issue> <assertion> <sha>` → prints revision; `finish <handoff> <revision> <state> <reason>` → CAS, nonzero on stale revision. Later tasks consume both.

- [ ] **Step 1:** Write `scripts/test_director_state.sh` fixtures asserting `pending-handoff-resumes`, `terminal-handoff-does-not-resume`, `selection-advances-revision`, `finish-refuses-stale-revision`, and that a killed writer leaves a valid JSON file (`flock` + temp + `mv`).
- [ ] **Step 2:** Run `bash scripts/test_director_state.sh`; expect FAIL (script absent).
- [ ] **Step 3:** Implement `director-state.sh` with `jq` for JSON, `flock` on `<handoff>.lock`, and the five-field-plus-ledger schema (`revision`, `issue`, `assertion`, `tree_sha`, `attempt`, `state`, `seen`, `next_action`).
- [ ] **Step 4:** Run the fixture; expect PASS. Run `bash scripts/test_director_writeback.sh` to confirm no regression.
- [ ] **Step 5:** Commit: `git commit -- scripts/director-state.sh scripts/test_director_state.sh docs/UNATTENDED.md`.

### Task 2: Machine-checked selection

**Files:** Modify `scripts/director-select-issue.sh`; create `scripts/test_director_select_issue.sh`.

**Interfaces:** Consumes `director-state.sh claim`. Produces stdout `ISSUE <n>|assertion=<id>|sha=<12>|revision=<n>` or `NO-PROGRESS <reason>`; exit 0 only for a claim, nonzero for no-progress and for errors.

- [ ] **Step 1:** Write fixture asserting `deferred-row-is-inadmissible`, `human-gate-row-is-inadmissible`, `same-assertion-same-sha-is-no-progress`, `new-integration-sha-reopens-selection`, `gh-failure-fails-closed`, `unevaluable-check-fails-closed`.
- [ ] **Step 2:** Run fixture; expect FAIL.
- [ ] **Step 3:** Implement candidate filter: parse status from `docs/DEFECTS.md` and labels from `gh`; skip `deferred`/`out-of-scope`, `HUMAN_GATE` and `seen[assertion].state == FILED` for the same `tree_sha`; preserve stderr and exit status of `gh`; claim only after a candidate is chosen.
- [ ] **Step 4:** Run fixture; expect PASS.
- [ ] **Step 5:** Commit: `git commit -- scripts/director-select-issue.sh scripts/test_director_select_issue.sh`.

### Task 3: Writeback targets the selected issue

**Files:** Modify `scripts/director-writeback.sh`, `scripts/test_director_writeback.sh`.

**Interfaces:** Consumes `director-state.sh finish`. Produces explicit `ROBIN_ISSUE`/`ROBIN_CRASH_ISSUE`/handoff `issue` precedence, `filed` label on FILED, and handoff update only after successful comment/label.

- [ ] **Step 1:** Change fixture to assert `explicit-issue-wins-over-hashtag`, `filed-writeback-adds-filed-label`, `crash-issue-env-is-honoured`, `writeback-fails-closed-on-missing-issue`, `finish-called-after-comment`.
- [ ] **Step 2:** Run fixture; expect FAIL on the hashtag case.
- [ ] **Step 3:** Remove hashtag inference; accept the explicit fields; add the `filed` label action; call `finish` only after `gh` succeeds; add `filed` to `docs/agents/triage-labels.md` in `~/home` only if the label vocabulary is owned there (read it first).
- [ ] **Step 4:** Run fixture; expect PASS.
- [ ] **Step 5:** Commit repo files by pathspec; commit the label doc in `~/home` separately only if changed.

### Task 4: One-run director driver

**Files:** Create `scripts/director-loop.sh`, `scripts/tests/director-loop.sh`; modify `docs/UNATTENDED.md` (kill switch).

**Interfaces:** Consumes selector line and handoff revision. Produces `runbg run director -- bash scripts/director-loop.sh` behaviour: fresh `opencode run --agent director` per tick, revision-proved completion, stop on `NO-PROGRESS`/bound/stop flag.

- [ ] **Step 1:** Write fixture with stub `opencode`/`gh` asserting `bounded-ticks`, `no-progress-halts`, `resume-after-interrupted-tick`, `stop-flag-at-tick-boundary`, `no-overlapping-ticks`.
- [ ] **Step 2:** Run fixture; expect FAIL.
- [ ] **Step 3:** Implement loop reading `scripts/run-bounds.sh`, selector, and revision check; write `RUN-COMPLETE <reason>` to stdout on every exit path.
- [ ] **Step 4:** Run fixture; expect PASS.
- [ ] **Step 5:** Commit: `git commit -- scripts/director-loop.sh scripts/tests/director-loop.sh docs/UNATTENDED.md`.

### Task 5: Gate the director fixtures

**Files:** Modify `scripts/check-quality.sh`, `docs/TESTING.md`.

- [ ] **Step 1:** Add a fixture assertion that `tooling` lists all four director fixtures; run it and expect FAIL.
- [ ] **Step 2:** Add the fixtures to the curated `tooling` list and name them in `docs/TESTING.md`.
- [ ] **Step 3:** Run `bash scripts/remote-check.sh tooling` via `runbg`; expect PASS with all fixture names in the summary.
- [ ] **Step 4:** Commit: `git commit -- scripts/check-quality.sh docs/TESTING.md`.

### Task 6: One executor for CI suites

**Files:** Modify `.github/workflows/rust-quality.yml`, `scripts/check-quality.sh`; create `scripts/tests/quality-preflight.sh`; create `docs/slop/ci-signal.md`.

- [ ] **Step 1:** Write fixture asserting that a suite invoked with `nextest`, `wasm-objdump`, `wasm-opt` and `pnpm` absent from PATH fails in under 10 s with the missing tool named, before any build.
- [ ] **Step 2:** Run fixture; expect FAIL.
- [ ] **Step 3:** Add `command -v` preflight to `check-quality.sh`; change the workflow to run suites in the pinned devShell (or install nextest from `flake.lock` and call `scripts/install_pinned_wasm_tools.sh`), restore the pnpm pin, add `timeout-minutes` to each job. Write `docs/slop/ci-signal.md` pointing at the workflow and gate selectors, not duplicating the matrix.
- [ ] **Step 4:** Run fixture; expect PASS. Dispatch `workflow_dispatch` on the same SHA and confirm all 18 jobs green or each red job names a defect row.
- [ ] **Step 5:** Commit by pathspec. Workflow change is human-authorized in this session (owner request), not a director edit.

### Task 7: Close gate path selectors

**Files:** Modify `scripts/push-gate.sh`, `scripts/test_push_gate.sh`, `docs/TESTING.md`.

- [ ] **Step 1:** Add fixture cases for `level-editor/`, `crates/robin_script_types/src/natives/signatures.rs`, `crates/robin_spellforge/**`, `vendor/rilua/**`, wasm test modules, and one negative case for an unrelated path.
- [ ] **Step 2:** Run fixture; expect FAIL for the three new selectors.
- [ ] **Step 3:** Implement selectors: editor → `editor`; ABI inputs → `contracts`; wasm test modules → `wasm-tests`. Define the two new suites in `check-quality.sh` with the exact commands from `docs/slop/research-ci-gate-reliability.md`.
- [ ] **Step 4:** Run fixture; expect PASS, including the negative case.
- [ ] **Step 5:** Commit by pathspec.

### Task 8: Repair the three real defects

**Files:** Modify `crates/robin_spellforge/tests/abi.rs`, `crates/robin_rs/src/**` test modules, `level-editor/**`; create `docs/slop/spellforge-abi-transition.md`.

- [ ] **Step 1:** Write failing regression checks: ABI golden equals the digest computed with both natives (`c19e…` → `87c3…`); `cargo check --target wasm32 … --tests` compiles; `pnpm --dir level-editor fmt:check` passes.
- [ ] **Step 2:** Run each; expect all three FAIL with the evidence in `docs/slop/research-ci-gate-reliability.md`.
- [ ] **Step 3:** Update the golden in the same change, record the reviewed transition in the slop doc; gate native-only test modules with `#[cfg(all(test, not(target_arch = "wasm32")))]`; run the editor formatter once.
- [ ] **Step 4:** Re-run all three checks; expect PASS. Run `bash scripts/remote-check.sh --suites contracts wasm-tests client-release tooling` via `runbg`; expect green summary.
- [ ] **Step 5:** Commit in three focused commits, one per defect.

### Task 9: Authority amendment

**Files:** Modify `docs/AUTONOMOUS_RUN_CONTRACT.md`, `docs/adr/0011-autonomous-release.md`, `docs/RELEASES.md`, `docs/UNATTENDED.md`, `.opencode/agents/director.md`.

- [ ] **Step 1:** Add a docs-check assertion that the contract names the release orchestrator as executable-but-not-editable and that MINOR bump items are no longer frozen.
- [ ] **Step 2:** Run the assertion; expect FAIL.
- [ ] **Step 3:** Amend the documents: execution authority for named human-authored release entry points; mechanical `prev MINOR + 1.0` bump after a landed batch; all human-owned boundaries from the design restated once.
- [ ] **Step 4:** Run `bash scripts/docs-check.sh` (or the repo's docs suite); expect PASS.
- [ ] **Step 5:** Commit by pathspec. This is the one-time human-landed authority change; no mid-run widening.

### Task 10: Mechanical next-MINOR preparation

**Files:** Create `scripts/release/prepare-next-minor.sh`, `scripts/test_prepare_next_minor.sh`.

**Interfaces:** Consumes landed batch + `docs/releases/whats-new.json`. Produces bumped `crates/robin_rs/Cargo.toml`/`Cargo.lock`, manifest entry, regenerated `RELEASE_NOTES.md`; exit nonzero when a source is missing.

- [ ] **Step 1:** Write fixture cases `missing-source-fails-closed`, `bump-is-minor-only`, `notes-quote-landed-sources`, `second-run-is-no-op`.
- [ ] **Step 2:** Run fixture; expect FAIL.
- [ ] **Step 3:** Implement using `release-notes.sh whats-new` as the only notes writer; refuse a bump that is not `prev MINOR + 1.0`.
- [ ] **Step 4:** Run fixture; expect PASS.
- [ ] **Step 5:** Commit by pathspec.

### Task 11: Idempotent release orchestrator

**Files:** Create `scripts/release-orchestrator.sh`, `scripts/test_release_orchestrator.sh`, `scripts/release/promote-web.sh` (human-authored, on `oci-vm`).

**Interfaces:** Consumes tag + orchestrator authority. Produces verified `release_outcome` only after native asset read-back, web promote and blocking parity.

- [ ] **Step 1:** Write fixture cases `tag-exists-is-idempotent`, `no-assets-does-not-record-RELEASED`, `parity-failure-blocks-RELEASED`, `retry-reuses-same-tag`, `never-moves-tag`.
- [ ] **Step 2:** Run fixture; expect FAIL.
- [ ] **Step 3:** Implement ordered fail-closed phases: notes preflight → tag once → dispatch → poll/read-back assets → call `promote-web.sh` → `release-parity-check.sh` → record outcome. Reuse `verifyRemote` semantics from `scripts/release/publish_native_release.ts`. Wire into `scripts/autonomy-driver.sh`.
- [ ] **Step 4:** Run fixture; expect PASS. Verify on the live tag that the script reports the current asset SHAs without publishing.
- [ ] **Step 5:** Commit by pathspec.

### Task 12: Land, then start the one run

**Files:** none (verification and operation).

- [ ] **Step 1:** Run `bash scripts/remote-check.sh --suites format lint check client-release engine tooling docs contracts wasm-tests web` under `runbg` and paste the summary into the landing commit message.
- [ ] **Step 2:** Run `runbg run director-land -- bash scripts/land.sh`; confirm with `git ls-remote fork refs/heads/main` and remove the worktree after `land clean` equivalent.
- [ ] **Step 3:** Start `runbg run director -- bash scripts/director-loop.sh` once. Confirm the first tick's selector line and handoff revision with `runbg log director`.
- [ ] **Step 4:** Re-run `/tmp/opencode/check-loops.py` only after the loop has cut a tag; report each check's effect.
- [ ] **Step 5:** Report the run handle, the terminal state emitted by the driver, and the release outcome recorded by the orchestrator.
