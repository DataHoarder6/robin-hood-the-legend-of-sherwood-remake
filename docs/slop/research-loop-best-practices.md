# Loop resilience: external best practices

Primary-source research behind the director loop's flake, no-progress, release
and bounded-autonomy rules. Each practice is tied to a URL fetched on
2026-11-25. This is a reference doc: the policy stays in
[UNATTENDED.md](../UNATTENDED.md), [PARALLEL_AGENTS.md](../PARALLEL_AGENTS.md),
[the run contract](../AUTONOMOUS_RUN_CONTRACT.md) and
[ADR 0011](../adr/0011-autonomous-release.md); earlier loop evidence is in
[unattended-loop-research.md](unattended-loop-research.md).

Last verified: 2026-11-25.

## (a) Quarantine flaky tests without hiding defects

**1. Quarantine is a separate suite with a forced expiry, not an ignore flag.**
Fowler: put non-deterministic tests in a quarantined area, keep them out of the
main deployment pipeline, and impose a numeric limit or a time limit (e.g. one
week) so they cannot be forgotten; a more aggressive team can run quarantine one
stage *after* the healthy tests. Source:
<https://martinfowler.com/articles/nonDeterminism.html>.
Maps here: the `deferred` row in `DEFECTS.md` is the quarantine, and the
known-flake policy already defers on identification.
Recommendation: add an explicit re-check date/owner to the deferred row and a
quarantine cap, and exclude deferred rows from `director-select-issue.sh`
selection until that date. Trade-off: a cap forces flake work into the critical
path; without it the ledger becomes a dumping ground.

**2. Detection is a rerun, but suppression is a separate, reversible decision.**
Azure DevOps documents system detection (rerun a failed test inside the same
pipeline; a fail-then-pass marks it flaky), a choice to suppress flaky failures
in the summary, manual mark/unmark, and "close the loop" by resetting a test
after its bug is fixed. Source:
<https://learn.microsoft.com/en-us/azure/devops/pipelines/test/flaky-test-management?view=azure-devops>.
Maps here: this repo's CI-2/local-0 retry budget and its deferred-row close-out.
Recommendation: keep the retry budget as *classification* only; the gate verdict
comes from the surviving failure, and a deferred row is the only suppression.
Never let a rerun turn a red suite green silently. Trade-off: CI spends two
extra attempts on infrastructure noise, but a real regression still fails.

**3. Google's own numbers say suppression is dangerous, and its mitigations are
explicitly imperfect.** 1.5% of test runs flake, ~16% of tests show some
flakiness, ~84% of pass→fail transitions involve a flaky test; presubmit
mitigations are rerun-failing-only, opt-in automatic rerun, and a "flaky" mark
that fails only after 3 consecutive failures. Its quarantine tool removes the
test from the critical path *and files a bug*, but Google states it "could
easily mask a real race condition or some other bug in the code being tested".
Source:
<https://testing.googleblog.com/2016/05/flaky-tests-at-google-and-how-we.html>.
Maps here: the red-gate flake problem and the "never keep fixing a known flake"
rule.
Recommendation: do **not** copy the 3-strikes mark into this gate — three
executions of an hour-long suite is a long latency and a real defect can fail
three times. Keep the repo's stricter rule: full-corpus fail + isolated pass ⇒
one `deferred` row, no further retries, no green. Trade-off: occasional false
quarantine; mitigated by the periodic recheck in (1).
A first-party blog comment adds that when a stable test becomes flaky and the
change is traceable, ~1/6 of those cases were production bugs, and "if the
default is to ignore the flaky tests then you will eventually be ignoring a real
bug" (<https://testing.googleblog.com/2017/04/where-do-our-flaky-tests-come-from.html>);
**evidence is thin** — this is a comment, not the article body.

**4. Fix the causes, and make setup failures loud.** Fowler's root causes are
lack of isolation, sleeps instead of callback/polling, remote services without
test doubles, direct system-clock calls, and resource leaks; a smoke test should
abort the run when an async service is down. Source: same Fowler URL as (1).
Maps here: build-host load flake and the "missing binary or pruned artifact is a
prerequisite failure, not a red assertion" rule.
Recommendation: keep prerequisite failure distinct from assertion failure in
`summary.json` so a flake classification cannot absorb an environment fault.
Trade-off: extra classification work per red gate.

## (b) No-progress detection and deterministic termination

**5. Bound the loop explicitly and take ground truth from the environment each
step.** Anthropic: agents should gain "ground truth from the environment at each
step", may pause at blockers, and should include "stopping conditions (such as a
maximum number of iterations)"; it names orchestrator-workers (a central agent
delegates to workers and synthesises results) and evaluator-optimizer (generator
plus evaluator in a loop) as established patterns. Source:
<https://www.anthropic.com/engineering/building-effective-agents>.
Maps here: director → builder/reviewer/verifier, and the two-attempt plus
repeat-commit circuit breaker.
Recommendation: keep the written termination contract and make no-progress a
first-class verdict, not a timeout: no new commit, same commit repeated, or no
change in the selected row ⇒ FILED/advance. Trade-off: a hard ceiling can file a
fixable item; the ceiling must be per item, not per run.

**6. OpenCode gives the same bounds as configuration, not prose.** `steps` is a
positive maximum number of model steps; on the final step tools are removed and
the model summarises. Subagents run in their own child sessions with **fresh
context**, always in the background, and the parent's `subagent` permissions
control which agents may be launched. Source:
<https://opencode.ai/docs/agents/>.
Maps here: fresh-context ticks and the bounded dispatch.
Recommendation: set `steps` on director and each subagent as a second, mechanical
bound behind the contract; treat a steps-exhaustion as the same terminal state as
a failed attempt. Do not rely on prompt wording alone.
The same page documents last-match-wins permission rules and that a custom
subagent uses its own permissions, not a subset of its parent's — relevant to
risk routing (<https://opencode.ai/docs/agents/>).

**7. Determine "no progress" from artefacts, not narration.** The loop's existing
evidence doc records that self-reported success is unreliable; the primary
pattern here is Anthropic's "ground truth from the environment at each step"
(source as in (5)). Source:
<https://www.anthropic.com/engineering/building-effective-agents>.
Maps here: "LANDED requires origin read-back" and "gate evidence, not agent
narration".
Recommendation: define tick progress as a change in (selected row, commit sha,
gate verdict) recorded in the handoff file; if none changes for a tick, advance
or halt by the contract. Trade-off: legitimate long gates may look stalled — use
the per-gate timeout as the discriminator, already 5400 s in the contract.

## (c) Reproducible, idempotent release gating

**8. Release only after all required checks pass, in CI, in a fixed step order.**
semantic-release: run the command only after all CI jobs pass; its steps are
Verify Conditions → Get Last Release → Analyze Commits → Verify Release →
Generate Notes → Create Git Tag → Prepare → Publish → Notify, and "a failure in
an early step prevents later steps from running"; "to keep releases
deterministic, run semantic-release only in CI and only after all required checks
pass". Sources:
<https://semantic-release.org/usage/ci-configuration/> and
<https://semantic-release.org/foundation/release-steps/>.
Maps here: `release-gate.sh` verdict feeding `auto-release.sh`.
Recommendation: model the release as ordered, fail-closed phases with an
explicit verify-conditions step (version, tag absence, breaking-change check,
gate verdict) before any tag/push. Trade-off: more preflight code; the script
already implements most of it.

**9. Idempotency comes from "no relevant change ⇒ no release" and immutable
tags.** semantic-release: "No relevant commits since the last release means no
new release"; a specific version cannot be requested ("you deliberately cannot
trigger a specific version release"); and a bad release is fixed by a new
release, never by removing the Git tag, "otherwise semantic-release will later
try to republish that version". It also recommends against committing version
changes during release because it forces branch-protection bypasses.
Sources: <https://semantic-release.org/foundation/release-steps/> and
<https://semantic-release.org/support/faq/>.
Maps here: MINOR-only, new-tags-only, Conventional-Commits-driven bump in
ADR 0011.
Recommendation: keep these invariants explicitly in the release script and its
tests, and make a second invocation with no new relevant commits a no-op, not an
error. Trade-off: adopting semantic-release itself would conflict with the
MINOR-only rule (it emits MAJOR on `BREAKING CHANGE`); adopt the decision logic,
not the tool.

**10. GitHub's branch/environment controls are the platform-level version of the
same gate.** Protected branches support required status checks (strict = branch
must be up to date), required deployments to succeed, merge queue, no force
push/deletion, signed commits, linear history, and "do not allow bypassing".
Source:
<https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches>.
Environments add required reviewers with optional self-review prevention, wait
timers, deployment branch/tag restrictions, and approval-gated secrets.
Source:
<https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments>.
Maps here: origin read-back, one landing at a time, and the owner-owned tail
(machine pin, seat sync).
Recommendation: treat these as owner-owned, pre-run configuration, not
director-editable state; the loop's gate remains the authority for the Linux
artifact. Trade-off: GitHub controls do not cover this repo's local `minipc`
gate, so they cannot replace `scripts/land.sh`.

**11. Serialise deployments with a FIFO concurrency group, not by cancelling
in-progress work.** GitHub Actions `concurrency` with `queue: max` processes
queued runs one at a time in FIFO order; `queue: max` cannot be combined with
`cancel-in-progress: true`. Source:
<https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#concurrency>.
Maps here: "one gate in flight" and the FIFO landing queue.
Recommendation: mirror the semantics in the repo-local queue (already done by
`gate-shared` and `scripts/land.sh` tickets); never cancel a running gate to
admit a newer one. Trade-off: latency behind a slow gate, which the two-hour
queue bound already caps.

**12. A deployment pipeline is staged, with increasing confidence and an audit
trail.** Fowler: stages buy confidence at extra time, may include manual checks
or human authorisation, and should give visibility and a thorough audit trail;
continuous delivery means deployable at any time, distinct from continuous
deployment. Sources:
<https://martinfowler.com/bliki/DeploymentPipeline.html> and
<https://martinfowler.com/bliki/ContinuousDelivery.html>.
Maps here: gate → land → tag → dispatch split, with machine pin/seats left human.
Recommendation: keep the release outcome a separate recorded fact from the
iteration terminal state; the audit trail is the tracker comment plus status
file, not agent narration. Trade-off: two facts to reconcile, but no false
"done".

## (d) Bounded autonomy over releases

**13. Bound trigger, branch, credentials and self-approval.** GitHub's Copilot
cloud-agent mitigations: only users with write access can trigger it, it can push
to one branch only, it cannot run `git push` directly, it cannot approve or merge
its own PR, workflows do not run until a human approves them, untrusted events
are ignored by default, automations are limited to chosen tools and to the
triggering issue/PR, and agent work is attributed, signed and logged. Source:
<https://docs.github.com/en/copilot/concepts/agents/coding-agent/risks-and-mitigations>.
Maps here: the contract-granted MINOR-only release authority and the immutable
run contract.
Recommendation: keep the authority narrow and pre-granted, keep the director's
write boundary to repo files it owns, and keep MAJOR/tag-move/machine pin/seat
actions outside it. Add explicit per-run audit lines for every tag and workflow
dispatch. Trade-off: less autonomy than GitHub's full agent; that is the point —
the loop publishes low-risk batches without mid-run questions.

**14. Least privilege is configurable and mechanically enforced.** OpenCode:
permission rules have `allow`/`ask`/`deny`, the last matching rule wins, any
`deny` denies the whole operation, policies can hard-deny after saved approvals,
and `question` is a permission action. Sources:
<https://opencode.ai/docs/permissions/> and <https://opencode.ai/docs/agents/>.
Maps here: `question: deny` on the director, `edit`/`shell` denies on protected
paths and the run contract.
Recommendation: rely on these rules plus `scripts/run-bounds.sh guard-edit`; a
prompt-level "do not edit" is not a control. Prefer short-lived credentials for
the publish step, as semantic-release recommends
(<https://semantic-release.org/usage/ci-configuration/>).
Trade-off: deny rules need maintenance when new protected paths appear.

**15. Risk governance is a framework, not a release checklist.** NIST AI RMF is
voluntary, organised as Govern/Map/Measure/Manage, and is being revised; the page
also notes a 2026 critical-infrastructure profile concept note. Source:
<https://www.nist.gov/itl/ai-risk-management-framework>.
Maps here: the pre-run interview, immutable contract, bounds and kill switch.
Recommendation: use it only as vocabulary for the pre-run risk decision; do not
claim it prescribes any specific CI or tag rule. **Evidence is thin** for this
question — the landing page gives no change-control clause.

## Conflicts with this repo's rules

| External practice | Conflict | Resolution |
| --- | --- | --- |
| 1 (quarantine expiry), 3 (mark-flaky-3-strikes) | Google's 3-strikes contradicts CI-2/local-0 and "known flake deferred on identification, never retried" | Keep the repo rule; borrow only the bug-link and expiry ideas |
| 10–11 (GitHub environments/concurrency) | Requires edits to `.github/workflows/*` and repo settings | Director is forbidden to edit these; they are owner pre-run setup. Missing control ⇒ BLOCKED with the capability named, or NEEDS-DECISION only for an unauthorised irreversible action |
| 9 (adopt semantic-release) | Its automatic MAJOR on `BREAKING CHANGE`, and release commits, contradict MINOR-only, no tag move, and the no-workflow-edit boundary | Adopt the decision logic and immutable-tag rule in `auto-release.sh`; do not adopt the tool |
| 5–7 (long-running orchestrator with memory) | Conflicts with fresh-context ticks and the five-field handoff artifact | Keep fresh-context ticks; progress state lives in the handoff file and the tracker, not agent memory |
| Fowler's optional "quarantine one stage after healthy" | Conflicts with one gate in flight and a red gate being a landing blocker | Keep quarantined rows out of the gate entirely; recheck in isolation on a timer |

## Where evidence is thin

- No primary source found for "one failing assertion per tick" or the exact
  four-state LANDED/FILED/BLOCKED/NEEDS-DECISION vocabulary. External sources
  support bounded loops and termination conditions, not this shape.
- No primary source covers this repo's AppImage/`native-release.yml` publication
  path specifically; GitHub environment docs are generic platform guidance.
- Anthropic's "Building effective agents" carries a 2026 note that its tooling
  landscape has changed; treat its patterns as conceptual, not current API
  guidance.
- The 1/6 production-bug figure is a blog comment (see (3)); the NIST source
  gives no specific change-control requirement (see (15)).
- The old semantic-release GitBook is discontinued; all semantic-release claims
  here use the current <https://semantic-release.org/> docs.

## What this doc does not do

It does not change policy or restate the rules. It records external claims with
their sources so the next session does not re-fetch them. Rules stay in the
policy pages linked at the top.
