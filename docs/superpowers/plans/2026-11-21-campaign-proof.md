# Campaign proof harness — phase 2

Goal: make the first real campaign co-op assertion executable. Issue #44 is
closed and H30 proves a two-seat campaign *join* for one mission; no run of more
than one mission exists. The smallest gap is one mission-end handoff.

## Global Constraints

- One fact one source; links must resolve; historical docs keep their evidence.
- E2E stays rare, seeded, hermetic, per-install isolated, evidence-producing.
  No new assertion verb unless H36 cannot express its claim with `mission` and
  `hash-equal`.
- Suite is `multiplayer-e2e`; entry point `scripts/check-quality.sh
  multiplayer-e2e` on the provisioned Linux build host (`minipc`). Gate is
  single-flight; use `scripts/remote-check.sh`, never a second concurrent gate.
- Never `CARGO_TARGET_DIR`, never `git stash`, commit by pathspec.
- All subagents use `opencode/hy4-flash-free#high`.

### Task 1: Add H36 campaign-handoff scenario

Files:
- Create `scripts/validation/scenarios/H36.json`: `schema: 4`, `campaign: true`,
  extends H30, two seats, zero faults, `resolve` = both seats Completed after
  `/mission-result` `level_succeeded`. Assert `mission(count=2)`,
  `hash-equal(peers=[1], every=25)`, `exit(peers=0,1,status=running)`.
- Modify `docs/multiplayer/HARNESS.md` §1 corpus table: add the H36 row and its
  `asserts:` tokens; do not duplicate the scenario's assertions in prose.
- Modify only if required: `scripts/validation/multiplayer_e2e.py` (new verb is
  a last resort; name why in the report).

Verify: `python3 scripts/validation/scenario_corpus.py --check` green; run the
slice on the build host with `python3 scripts/validation/multiplayer_e2e.py
--only H36` (or the documented runner) and keep its evidence directory; then
`bash scripts/check-quality.sh multiplayer-e2e` if the host is provisioned.

Honest limit: H36 asserts handoff + hash agreement, not campaign slot
persistence or Windows cross-play.

### Task 2: Mark the stale slop matrix as superseded

File: `docs/slop/multiplayer-terminal-matrix.md:73-77`.
Change "rows to add"/live-matrix framing into a dated superseded note pointing
at `docs/multiplayer/HARNESS.md:105-127`; name H22→H32 and H26→H27/H28. Keep the
historical table as evidence. Do not touch the 50 pre-existing broken links in
retained historical docs — the gate is deliberately scoped to changed paths
(`scripts/push-gate.sh:110-117`); record the rot as a known cost, not a fix.

### Task 3: One landing host for the shared gate copy

`gate-shared` lives on `minipc`, but `remote-check.sh` takes its lock locally
per host. A landing from this runtime raced a landing from `oci-vm`: the other
tree replaced `gate-shared` mid-gate and the docs suite failed on files that
were present in the source checkout. Fix as a steering rule, not a new lock:
`docs/PARALLEL_AGENTS.md` states that robin landings run from `oci-vm`, whose
agents share one lock namespace; other hosts transfer commits to an `oci-vm`
worktree before calling `scripts/land.sh`. Leave the gate unchanged.

### Task 4: File the campaign-handoff defect

H36 is RED for a real game defect, not a scenario error. Repo rule: a defect
seen in a session becomes a `docs/DEFECTS.md` row before a fix. Add the next
free id (D06 is taken) with class `generation` or `identity`, reproduction
`python3 scripts/validation/multiplayer_e2e.py --only H36` with its evidence
directory, and the failing check = H36's `mission(count=2)`,
`hash-equal(every=25)`, `exit(running)`. Evidence: host commits `CampaignExit`
and starts mission 2; client drops the prepared transition because its
transport generation ended, then exits 0 on `lincoln` with one desync. Cite
the evidence files, do not copy the audit into the row.

### Task 5: Fix the campaign handoff so H36 goes green

Fix the game defect from Task 4, not the scenario. Keep the client in the
host's campaign across the mission-end transport rebuild: either re-admit the
joining seat on the new generation, or keep the generation across the
campaign-exit snapshot so the committed transition is not dropped. Add or
extend a Rust-side regression test that fails without the fix, then re-run the
H36 slice on the build host through `scripts/remote-check.sh`. H36 going green
is the proof; do not weaken its assertions.
