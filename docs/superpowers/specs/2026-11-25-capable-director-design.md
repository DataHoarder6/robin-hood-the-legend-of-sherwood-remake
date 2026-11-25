# Capable director: one run, continuous releases

- **Status:** approved by the owner's "fix all / do it start to finish" (2026-11-25)
- **Last verified:** 2026-11-25 against `fork/main` `3b0691320`
- **Evidence:** [director loop](../../slop/research-director-loop-capability.md),
  [CI gate](../../slop/research-ci-gate-reliability.md),
  [continuous release](../../slop/research-continuous-release.md),
  [external practices](../../slop/research-loop-best-practices.md).

## Intent

The unattended director must finish development and cut MINOR releases back to
back in **one run**, with no extra session and no mid-run human decision. Today
it re-selects the same quarantined row, cannot see three real CI defects, and
cannot prepare the next release after a tag. The fix is to make progress
mechanical: selection, handoff, gate evidence and release tail become scripts
with tests, not agent prose.

## Goals

- Selection strictly advances; a `deferred` row is never re-selected as a failure.
- A tick's completion is proved by an artefact (handoff revision + gate verdict),
  never by narration. One command runs bounded ticks until `RUN-COMPLETE`.
- All named suites are reproducible in CI and the landing gate sees the paths
  that can break them. No silent green from missing tools.
- After a tag is verified, the next MINOR is prepared mechanically and the web
  artifact is published from the same tag, with parity blocking announcement.
- MAJOR bumps, tag moves, machine pin, seats, credentials and CI/deploy
  definitions stay outside the loop.

## Non-goals and anti-goals

- No semantic-release adoption (emits MAJOR; conflicts with MINOR-only).
- No silent rerun-to-green; retries classify only, never suppress.
- No widening the director's `edit` denies to workflows, Nix, `land.sh`,
  `remote-check.sh`, `release*.sh` or secrets. Execution of named, human-authored
  entry points is the only new authority.
- No closing D09/D11 or issue #49 by rerun or by rewriting their status.

## Constraints

- One gate in flight (`docs/PARALLEL_AGENTS.md`); `scripts/land.sh` under `runbg`.
- Fresh-context ticks; state lives in the tracker and the handoff, not memory.
- `docs/RELEASES.md` and `docs/AUTONOMOUS_RUN_CONTRACT.md` own the process and
  authority; this design changes them once, human-landed, before the run.
- Volatile facts stay behind commands; new docs follow `DOC-STYLE.md`.

## Approaches considered

1. **Selector patch only.** Fix `director-select-issue.sh` to skip `deferred`.
   Rejected: leaves CI red and the release tail manual; the loop would advance
   into a wall and stall there instead.
2. **Full capability stack (chosen).** State ledger and bounded driver for the
   loop; environment parity and path selectors for the gate; idempotent release
   orchestrator plus mechanical next-MINOR preparation. Larger, but the only
   option where "one run to continuous releases" is true.
3. **External scheduler / semantic-release.** Rejected: workflow edits are
   outside the loop's authority, semantic-release emits MAJOR, and neither fixes
   the local `minipc` gate or the estate web origin.

## Design

### Loop capability

- `scripts/director-state.sh claim|finish`: atomic (`flock` + temp + `mv`)
  handoff with `revision`, `issue`, `assertion`, `tree_sha`, `attempt`, `state`,
  `seen`, `next_action`. `finish` is compare-and-swap on `revision`.
- `scripts/director-select-issue.sh`: fail-closed on `gh` errors; recompute from
  live sources; skip `deferred`/`out-of-scope` defects, `HUMAN_GATE` rows and
  assertions already FILED for the same integration sha; print
  `ISSUE|assertion|sha|revision` or `NO-PROGRESS <reason>`; claim atomically.
- `scripts/director-writeback.sh`: issue from explicit handoff field or
  `ROBIN_ISSUE`/`ROBIN_CRASH_ISSUE` only, never a hashtag grep; add `filed`
  label on FILED; call `finish` only after comment/label succeeds.
- `scripts/director-loop.sh`: one `runbg run director -- bash scripts/director-loop.sh`
  starts fresh-context `opencode run --agent director` ticks, checks the
  revision advanced, re-reads bounds, and stops on `NO-PROGRESS`, a bound or a
  stop flag at a tick boundary. Kill switch becomes `runbg stop director`.
- Fixtures for all three, added to the `tooling` suite; the wrong-target test is
  corrected, not preserved.

### Gate capability

- One executor: suites run in the pinned devShell or with the versions from
  `flake.lock` (nextest, wabt/binaryen, pnpm). Preflight in
  `scripts/check-quality.sh` names a missing tool before any long build.
- `push-gate.sh` selectors: `level-editor/` → editor check; native signature /
 Spellforge ABI inputs → ABI suite; wasm test modules → `wasm-tests` suite.
- Repair the three real defects test-first: stale ABI golden (keep both
  natives; record the transition), `cfg(test)` on wasm32 calling native-only
  APIs, editor formatting drift. D09/D11 stay deferred.
- `docs/slop/ci-signal.md` names which Actions job is authoritative and which
  command prints its tool requirements; no suite matrix is duplicated.

### Release capability

- One human-landed contract/ADR amendment grants execution of a named release
  orchestrator and un-freezes the mechanical next-MINOR bump. Human-owned
  paths and the machine pin stay human.
- `scripts/release-orchestrator.sh`: verify notes/manifest → tag once →
  dispatch → poll and read back native assets → call the human-authored web
  promote entry point → run blocking parity → record `release_outcome` only on
  a full pass. Retry of an existing tag re-verifies and re-dispatchs; it never
  moves or deletes a tag.
- `scripts/release/prepare-next-minor.sh`: after a landed MINOR batch, bump
  `Cargo.toml`/`Cargo.lock` to previous MINOR + 1, add the manifest entry from
  landed sources, regenerate notes; fail closed when a source is missing.
- Web promote is a narrow, human-authored wrapper around the existing
  `deploy_web_jautis.sh` on `oci-vm`; the director may execute it, never edit it
  or the credentials it uses.

## What is left to the builder

Implement in the order: loop state/selection/writeback + fixtures → gate parity
and three defect fixes → contract amendment → release orchestrator and next-MINOR
preparation → gate everything in `tooling` → land, then start the single
`runbg` director run. Each stage has a failing test before its fix; verification
reports effects (test counts, gate verdicts, artifact SHAs, remote read-back),
never bare exit codes.
