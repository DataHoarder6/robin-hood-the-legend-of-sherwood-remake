# Autonomous development loop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the unattended loop actually land features and cut releases, and land/ship the stranded 0.5.0 work.

**Architecture:** Build on the robin loop's hardened safety half (scoped roles, one gate in flight, FIFO landing with remote read-back, terminal-state doctrine, ADR 0009 batching). Add a pre-run contract, a risk-classified backlog, a batch controller, an autonomous release step, and AppImage retention. Fix the two operational causes found in the 24h diagnosis: the free-model quota is a single point of failure, and work is stranded in feature branches because nothing drives `land.sh`.

**Tech Stack:** Bash scripts (existing `scripts/land.sh`, `remote-check.sh`, `push-gate.sh`, `release-gate.sh`, `release/verify_release_tag.sh`), systemd timers, NixOS modules, GitHub release workflow, estate `runbg`/`land`.

**Spec:** `/home/matt/wt/robin-hood-remake/agent-autonomy-design/docs/superpowers/specs/2026-11-23-autonomous-loop-design.md` (owner-approved by "do this all"). Diagnosis: the 2026-11-23 session review (24h window).

## Global Constraints

- Repo scope of the loop: `robin-hood-remake` only. Other repos stay human backlog.
- No mid-run questions; `director` keeps `question: deny`. Contract fixed before each run in `docs/AUTONOMOUS_RUN_CONTRACT.md`.
- Release policy: MINOR bumps only (`0.x.0`); MAJOR is human-only; new tags only, never move an existing tag; two prior known-good AppImages retained; `robin-pin` owned by the owner and denied to the loop.
- No Sandcastle dependency. No paid model fallback; no cost cap.
- One gate in flight; land only via `scripts/land.sh` under `runbg`, with remote read-back. Never edit `push-gate.sh`, `remote-check.sh`, `land.sh`, CI workflows, Nix files or secrets from the loop.
- Bounds: 4 h wall clock, max 3 batches; circuit breaker after two consecutive red gates or a bisect yielding no landing.
- Quota exhaustion is a named BLOCKED sub-reason with fail-closed admission — never silently switch provider.

## Review Focus

- **A killed tick must not lose work:** handoff must carry uncommitted changes or finalise-after-crash; test every script is resumable from a recorded run id.
- **Quota 429 mid-run:** admission must fail closed and name `quota-exhausted`, never switch provider; test the preflight.
- **Release must never move a tag or bump MAJOR:** `auto-release.sh` must refuse existing tags and non-MINOR bumps; test both refusals.
- **A red gate must never land or release:** batch controller must halt on two consecutive reds; test the breaker.
- **Stranded work must land:** feature worktrees must call `land.sh`, not be force-discarded; test that a landed commit appears on `fork/main` via read-back.

---

### Task 1: Land the stranded 0.5.0 feature work

**Files (worktree `.worktrees/050-land`, branch `autonomy/050-land` off `fork/main`):**
- Commit: pending `docs/slop/campaign-flow-case-inventory.md` line from the red Task 9 gate.
- Cherry-pick: the 12 commits of `agent/full-campaign-proof` (Tasks 1-8 + fixes).
- Commit: the six `agent/browser-deploy-050` web-deploy fixes.

**Interfaces:** Consumes `fork/main` @ `d5ded33dc`. Produces landed commits on `fork/main`; then a `v0.5.0` tag.

- [ ] **Step 1: Commit pending fixes and cherry-pick the 12 commits**
  - In `050-land`, take the one-line inventory fix from `full-campaign-proof`'s dirty file; cherry-pick range `aae045f74^..ac1cffae9` from `agent/full-campaign-proof`. Resolve conflicts against current `fork/main`; verify `git status` clean.
- [ ] **Step 2: Bring in the web-deploy fixes** from `agent/browser-deploy-050` (six files), resolving against current `fork/main`.
- [ ] **Step 3: Re-run the full gate** (`runbg run check-050 -- bash scripts/remote-check.sh --suites format lint check engine client client-release tooling docs`), scoped docs via `ROBIN_DOCS_CHECK_PATHS` for the changed docs only. Gate must be green.
- [ ] **Step 4: Land** with `runbg run land-050 -- bash scripts/land.sh`; confirm with `git ls-remote fork refs/heads/main`.
- [ ] **Step 5: Cut `v0.5.0`** via the scriptable RELEASES.md steps (release gate, release notes, tag, push, dispatch). Report the human Windows-seat and machine-repo pin steps as remaining.

### Task 2: Contract, bounds and terminal states

**Files (`autonomy-impl`):**
- Create: `docs/AUTONOMOUS_RUN_CONTRACT.md`, `scripts/run-bounds.sh`, `scripts/test_run_bounds.sh`
- Modify: `docs/UNATTENDED.md` (terminal states, goal, tag policy), `docs/GLOSSARY.md` if new terms.

**Interfaces:** Produces `run-bounds.sh check <contract>` (exit 0/1, named reason on stdout) and the four iteration states + `RELEASED`/`ROLLED-BACK` outcome vocabulary.

- [ ] **Step 1: Write failing tests** for bound parsing (4h, 3 batches, red-gate circuit breaker, no cost cap) and for refusal to edit the contract.
- [ ] **Step 2: Run them; expect FAIL.**
- [ ] **Step 3: Write the contract** from the approved spec's §"The contract" and `run-bounds.sh` implementing the checks. Amend `UNATTENDED.md` terminal-state and goal paragraphs (add `NEEDS-DECISION`; move release outcomes out of iteration states).
- [ ] **Step 4: Run tests; expect PASS. Commit by pathspec.**

### Task 3: Backlog and promotion

**Files:** Create `docs/AUTONOMOUS_BACKLOG.md`, `scripts/autobacklog.sh`, `scripts/test_autobacklog.sh`; modify `.opencode/agents/reviewer.md`, `.opencode/agents/verifier.md`.

**Interfaces:** `autobacklog.sh next-ready|generate-proposed|promote <item>|verdict <item> <verdict>`; risk classes `proposed|ready|needs-work|rejected`; only docs/tests/small refactors promotable.

- [ ] Steps: failing tests → FAIL → implement + narrow reviewer/verifier write permits → PASS → commit.

### Task 4: Batch controller (ADR 0009)

**Files:** Create `scripts/batch-controller.sh`, `scripts/test_batch_controller.sh`.

**Interfaces:** consumes `autobacklog.sh next-ready`; readiness refs keyed by head SHA, FIFO epochs, `git merge-tree` conflict preflight, controller-owned staging tree, one tree-keyed gate, bisect on red, `land.sh` read-back.

- [ ] Steps: failing tests for FIFO ordering, conflict preflight, one-gate invariant, bisect on red → FAIL → implement → PASS → commit.

### Task 5: Autonomous release

**Files:** Create `scripts/auto-release.sh`, `scripts/test_auto_release.sh`, `docs/adr/0011-autonomous-release.md`; modify `UNATTENDED.md:56` (tag-policy line), `.opencode/agents/director.md:47`, `docs/RELEASES.md:84-87`.

**Interfaces:** `auto-release.sh <candidate> <binary>` — refuses if tag exists, refuses non-MINOR bump, requires release-gate verdict=observed and H23=observed, then `git tag -a` + `git push fork` + `gh workflow run native-release.yml --ref <tag>`; never touches machine-repo pin or installs.

- [ ] Steps: failing tests for existing-tag refusal, MAJOR refusal, missing-verdict refusal → FAIL → implement + doc amendments + ADR → PASS → commit.

### Task 6: AppImage retention and release-worthy predicate

**Files:** Create `scripts/appimage-retain.sh`, `scripts/test_appimage_retain.sh`; add release-worthy predicate to `batch-controller.sh`; add `robin-pin` pointer and `previous-good`.

**Interfaces:** retain last two known-good AppImages; predicate derived from Conventional Commits in the batch.

- [ ] Steps: failing tests (keeps exactly two, never deletes pinned) → FAIL → implement → PASS → commit.

### Task 7: Runtime config (nixos-config worktree `autonomy-runtime`)

**Files:** locate actual module under `modules/agent/` (grep for `director-loop`, `unattended-jobs`). Amend:
- stop/disable `director-loop.timer` when autonomy is enabled (they compete for gate/worktrees);
- point the timer at the new contract-driven tick;
- consolidation of the two dispatchers that share one free-model quota, with budget/admission control naming `quota-exhausted` and no provider switch;
- crash-finalise/intent so an interrupted tick can be completed without a second LLM call.

- [ ] Steps: `just check-eval` → edit module → `just fmt` → `just check-eval` → gate via `land` with `--wait` → `just deploy-wait`.

### Task 8: Estate steering (home worktree `autonomy-steering`)

**Files:** `docs/agents/unattended-queue.md` (add `NEEDS-DECISION`/human-handoff, release outcome), `docs/agents/dispatch-baseline.md` if needed, new ADR under `docs/adr/`, research docs (greenfield/autonomous-infra) updated with the decisions.

- [ ] Steps: edit docs → verify links/consistency → `land finish --wait` → confirm origin read-back.

### Task 9: End-to-end verification

- [ ] Each robin task gated by `remote-check.sh` and landed via `scripts/land.sh`; confirm with `ls-remote`.
- [ ] Smoke test: after 0.5.0 lands, run one low-risk item manually, read-only attach, stop via `runbg stop`, verify contract file and clean landing — no release in the smoke test.
- [ ] `just deploy-wait` for nixos-config; verify timer state on `oci-vm`.
