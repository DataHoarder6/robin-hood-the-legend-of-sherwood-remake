# Director loop capability: why D09 repeats and how to make selection advance

Last verified: 2026-11-25 against `fork/main` `3b0691320225ad0fed59e1a54ae0583f0c361733`
(the worktree `loop-resilience` is at that tip), the live GitHub tracker, and `oci-vm`
units/handoff. Reference doc: evidence and proposed changes, not policy. Live
state below was printed by the commands named in the text; commands, not this
file, remain the source for it.

## 1. The exact code path, and why D09 repeats

### What launches a tick

- [`UNATTENDED.md:15-16`](../UNATTENDED.md#L15-L16) says the loop is launched
  explicitly with `opencode run --agent director ...`; there is no default agent.
- On `oci-vm`, `systemctl list-timers --all | grep -i robin` shows only
  `robin-crash-dispatch.timer` (every ten minutes), `robin-watchdog.timer`,
  `robin-health-check.timer` and `robin-build-prune.timer`. There is **no**
  `director-loop.timer` (system or user), despite the kill switch in
  [`UNATTENDED.md:224-231`](../UNATTENDED.md#L224-L231).
- `systemctl cat robin-crash-dispatch.service` runs
  `/nix/store/.../crash-dispatch.sh`. Its lines 10-24 list open issues with
  `crash` + `ready-for-agent`, claim `agent-working`, and launch
  `opencode run --agent director "Diagnose and fix GitHub issue #N in crash-issue mode."`
  with `ROBIN_CRASH_ISSUE=$number`. It never selects #49: the live tracker shows
  #49 labelled only `enhancement`, and its label events since 2026-11-19 contain
  no `ready-for-agent`. The five #49 ticks were therefore explicit directives,
  not timer dispatches. Their own comments say so: attempts 3 and 4 record
  "the live selector printed #76; #49 was named".
- The selector was committed at `70579fced` (2026-11-23T18:14+02:00) and the
  writeback at `f5de19735` (2026-11-24T19:15+02:00). The five FILED comments on
  #49 are dated 2026-11-23T18:01Z, 18:36Z, 19:10Z, 20:37Z and 2026-11-24T08:54Z,
  i.e. the writeback script did not exist for any of them. Their bodies start
  `Director tick — terminal state **FILED**`, not the script's template.

### Selector

[`scripts/director-select-issue.sh`](../../scripts/director-select-issue.sh):

1. Lines 11-12: default handoff
   `/home/matt/.local/state/robin-director/handoff.json` (on `oci-vm`).
2. Lines 14-21: read `.state` and `.selected_item`; **resume only** when state
   is exactly `pending` and a selected item exists. The observed handoff says
   `state: LANDED` for D12 and its `next_action` says "Next tick: recompute from
   sources". Both are ignored: the condition at line 17 is false.
3. Lines 23-27: `gh issue list --state open --label ready-for-agent --json
   number,title,updatedAt --jq 'sort_by(.updatedAt) | .[0] | ...'`, with stderr
   discarded. It prints `ISSUE <number> <title>` (line 8 contract): no assertion,
   no tree sha, no attempt, no claim. Run now it prints
   `ISSUE 80 Remove idle PLAN ACTIONS HUD box from the browser build`.
4. Lines 28-29: a failed/empty `gh` call collapses to one message,
   `no admitted issue is selectable`, with no distinction between auth failure,
   network failure and an empty queue.

Once the director decides the selected issue is #49,
[`UNATTENDED.md:100-120`](../UNATTENDED.md#L100-L120) imposes the fixed order:
the nine criteria of #49, then HARNESS rows `H01..Hnn`, then
[`DEFECTS.md`](../DEFECTS.md) rows `D01..Dnn`. Line 118 says a row with no
failing assertion is skipped, but there is no machine-readable predicate for
"failing". [`DEFECTS.md:46`](../DEFECTS.md#L46) is D09: status `deferred`,
check = isolated row recheck, backing issue #75 (still open). The isolated
recheck is green by the row's own evidence; the director nonetheless treated
`deferred` as "the first failing assertion" in all five comments. The known
flake policy ([`UNATTENDED.md:188-198`](../UNATTENDED.md#L188-L198)) says a
known flake is deferred on identification and never retried; the FILED
definition ([`UNATTENDED.md:32-38`](../UNATTENDED.md#L32-L38)) says the row ends
the *iteration*. Neither rule removes D09 from the source set for the next tick.

### Writeback

[`scripts/director-writeback.sh`](../../scripts/director-writeback.sh):

1. Lines 11-17 validate the state and four optional arguments, including the
   attempt number.
2. Lines 22-26 pick the issue: `ROBIN_ISSUE` if set, otherwise the **first**
   `#NNN` matched anywhere in the handoff. The live handoff's first match is
   `#83` (verified with `grep -oE '#[0-9]+' handoff | head -1`), so a writeback
   with this handoff targets #83, not the standing-goal issue. The one real
   script-produced comment found on the tracker is on #83 at
   2026-11-24T17:18:43Z with exactly this template; no #49 comment has it.
3. Lines 30-35 post the comment; line 36 removes `ready-for-agent` and
   `agent-working`; lines 37-41 add `ready-for-human` only for BLOCKED and
   NEEDS-DECISION. **LANDED and FILED add no terminal/quarantine label.**
4. The script never writes the handoff. A repo-wide search finds no handoff
   writer at all: [`UNATTENDED.md:128-139`](../UNATTENDED.md#L128-L139) requires
   the director to write it at selection and after every transition, but that
   is left to agent prose. The file's mtime on `oci-vm` is 2026-11-24 10:54 and
   its state is still the D12 LANDED record.

[`scripts/test_director_writeback.sh:20,29`](../../scripts/test_director_writeback.sh#L20-L29)
enshrines the wrong-target behaviour: the fixture handoff contains
`D12 / #83 teleport desync` and the test asserts `comment 83`.

### The repeating chain

1. The source set is unchanged between ticks: D09 remains `deferred`, #75
   remains open, no rule excludes either from the fixed order.
2. Each fresh context recomputes from those same sources, so the first item is
   D09 again. "The row ends the iteration" is implemented as an ending, not as
   an advancement of a cursor.
3. The handoff is not authoritative: state `LANDED` defeats the `pending`-only
   resume, and the "recompute" next action is prose the selector never reads.
4. #49 has never carried `ready-for-agent`, so tracker-level admission cannot
   suppress it; explicit directives keep naming it and the agent prompt calls
   it a standing goal ([`director.md:68-71`](../../.opencode/agents/director.md#L68-L71)).
5. Even when writeback runs, it can target the wrong issue via the first
   hashtag, and FILED leaves no filed/quarantine marker on the selected issue —
   so nothing on the tracker records that #49/D09 was already filed.
6. The circuit breaker ([`UNATTENDED.md:173-186`](../UNATTENDED.md#L173-L186))
   keeps the attempt count and repeat-commit ledger in agent context. Fresh
   ticks reset both; five attempts were emitted for one row under a stated
   ceiling of two.
7. The selector's priority is *oldest `updatedAt`* ([line 26](../../scripts/director-select-issue.sh#L26)).
   When writeback comments a different issue, the selected issue's timestamp is
   not bumped, so it stays at the head of the queue whenever it is admitted.

## 2. Antipatterns found

| # | Antipattern | Source | Effect |
| --- | --- | --- | --- |
| A1 | `deferred` defect rows stay admissible in the fixed order; "failing" has no machine-checked predicate | [`DEFECTS.md:46`](../DEFECTS.md#L46), [`UNATTENDED.md:110-120`](../UNATTENDED.md#L110-L120) | Quarantined D09 is re-selected as a "failure" forever |
| A2 | FILED ends an iteration but is not recorded as progress; no cursor advances | [`UNATTENDED.md:32-38`](../UNATTENDED.md#L32-L38) | Same assertion is legal next tick |
| A3 | Handoff is read but not authoritative: resume only on `pending`, `next_action` ignored, no writer in the repo | [`director-select-issue.sh:14-21`](../../scripts/director-select-issue.sh#L14-L21), [`UNATTENDED.md:128-139`](../UNATTENDED.md#L128-L139) | A LANDED handoff asking to recompute produces the same D09 selection |
| A4 | Writeback infers the issue from the first `#NNN` anywhere in handoff text; test codifies it | [`director-writeback.sh:22-26`](../../scripts/director-writeback.sh#L22-L26), [`test_director_writeback.sh:20,29`](../../scripts/test_director_writeback.sh#L20-L29) | Audit comment and label changes land on #83 while #49 keeps its state |
| A5 | LANDED/FILED add no terminal label; only BLOCKED/NEEDS-DECISION get `ready-for-human` | [`director-writeback.sh:36-41`](../../scripts/director-writeback.sh#L36-L41) | No tracker-level quarantine; re-admission is trivial |
| A6 | Selector discards `gh` stderr and reports one generic error for every failure | [`director-select-issue.sh:23-28`](../../scripts/director-select-issue.sh#L23-L28) | Auth/network failure looks like an empty queue; silent fallback to prose selection |
| A7 | Selector output has no assertion, sha, attempt or claim | [`director-select-issue.sh:8,29`](../../scripts/director-select-issue.sh#L8) | The director must recompute the assertion from prose each tick; strict advancement is impossible |
| A8 | Attempts and repeat-commit detection live in agent context | [`UNATTENDED.md:173-186`](../UNATTENDED.md#L173-L186), [`director.md:90-95`](../../.opencode/agents/director.md#L90-L95) | Two-attempt ceiling does not survive a fresh context; five attempts observed |
| A9 | Selector does not atomically claim an issue; the crash dispatcher claims only crash issues | [`director-select-issue.sh:23-29`](../../scripts/director-select-issue.sh#L23-L29), `/nix/store/.../crash-dispatch.sh:13-24` | Two manual ticks can select the same work; #49 is never claimed |
| A10 | Four separate state models (tracker labels, handoff JSON, DEFECTS status, autonomous backlog/status) with no bridge | [`AUTONOMOUS_BACKLOG.md`](../AUTONOMOUS_BACKLOG.md), [`autonomy-driver.sh:191-233`](../../scripts/autonomy-driver.sh#L191-L233) | The run driver loops backlog batches and never dispatches a director; #49 ticks continue outside it |
| A11 | No run-level no-progress detection; documented kill switch names a timer that does not exist | [`autonomy-driver.sh:393-398`](../../scripts/autonomy-driver.sh#L393-L398), [`UNATTENDED.md:224-231`](../UNATTENDED.md#L224-L231) | Nothing halts a repeated FILED, and no automatic tick process exists to stop |
| A12 | Director fixtures are not in any gate | [`check-quality.sh:198-240`](../../scripts/check-quality.sh#L198-L240), [`TESTING.md:799-805`](../TESTING.md#L799-L805) | The one existing writeback test can codify a wrong behaviour (A4) and nothing catches it |
| A13 | Launcher sets `ROBIN_CRASH_ISSUE`, writeback reads `ROBIN_ISSUE` | `/nix/store/.../crash-dispatch.sh:23`, [`director-writeback.sh:22`](../../scripts/director-writeback.sh#L22) | Crash-mode ticks fall back to hashtag inference (A4) |
| A14 | Priority is oldest `updatedAt`, not most actionable work | [`director-select-issue.sh:26`](../../scripts/director-select-issue.sh#L26) | An issue whose writeback targets another issue is never bumped and stays first |
| A15 | Handoff schema is fixed at five fields, too small to hold a progress ledger | [`UNATTENDED.md:135-139`](../UNATTENDED.md#L135-L139) | "Strictly advance" has nowhere to be recorded |

## 3. Ranked fix list (smallest durable set first)

The minimal viable set is F1-F3: two scripts, one new fixture, one agent-prompt
edit and one contract edit. F4 adds the one-run driver; F5 gates the whole set.

**F1 (P0) — Make deferred/quarantined rows inadmissible and define "failing"
mechanically.** In [`director-select-issue.sh`](../../scripts/director-select-issue.sh)
add a candidate filter before selection: skip `DEFECTS.md` rows whose status is
`deferred`/`out-of-scope`, skip rows already FILED for the current tree sha, skip
`HUMAN_GATE` items, and skip HARNESS rows with no red check. If a check cannot
be evaluated, fail closed with `unknown-assertion-state` instead of assuming a
failure. Update [`UNATTENDED.md:110-120`](../UNATTENDED.md#L110-L120) to state
that predicate: a `deferred` row is a fact about a quarantined defect, not a
failing assertion. Tests in a new, not-yet-created
[`scripts/test_director_select_issue.sh`](../../scripts/test_director_select_issue.sh):
`deferred-defect-row-is-not-admissible`, `human-gate-row-is-not-admissible`,
`unevaluable-check-fails-closed`, `first-red-assertion-follows-fixed-order`.

**F2 (P0) — Give the handoff a writer and make it the resume authority.** Add
`scripts/director-state.sh claim|finish` using temp-file + `mv` under `flock`,
with `revision`, `issue`, `assertion`, `tree_sha`, `attempt`, `state`, `seen`
(map assertion → last terminal state/sha/reason) and `next_action`.
[`director-select-issue.sh`](../../scripts/director-select-issue.sh) calls `claim`
and prints a machine-readable line (`ISSUE|assertion|sha|revision`);
[`director-writeback.sh`](../../scripts/director-writeback.sh) calls `finish`
after the comment/label succeeds, with compare-and-swap on `revision`. Resume
on any non-terminal-closed state, not only `pending`; honour `next_action` as a
cursor. Update [`UNATTENDED.md:128-139`](../UNATTENDED.md#L128-L139) to the new
schema. Tests: `pending-handoff-resumes-before-tracker-read`,
`terminal-handoff-does-not-resume-stale-assertion`,
`selection-advances-handoff-revision`,
`finish-refuses-stale-revision`,
`resume-honours-recorded-next-action`.

**F3 (P0) — Strict advancement and no-progress halt.** In the selector, compare
the recomputed candidate with `seen` for the same tree sha: if it is the last
FILED assertion and the integration sha has not changed, print
`NO-PROGRESS repeat-<assertion>` and stop; otherwise select the next admissible
candidate. Never fall back to D09/#49 by prose. Fix writeback's issue source to
the handoff's explicit `issue` field or `ROBIN_ISSUE` (never a hashtag grep),
add a `filed` label for FILED so the tracker mirrors the quarantine, and
accept `ROBIN_CRASH_ISSUE` as the crash-mode alias. Correct
[`test_director_writeback.sh:20,29`](../../scripts/test_director_writeback.sh#L20-L29)
to assert the explicit issue. Tests: `same-assertion-same-sha-is-no-progress`,
`new-integration-sha-reopens-selection`,
`filed-writeback-targets-explicit-issue`,
`filed-writeback-adds-filed-label`,
`writeback-fails-closed-on-missing-issue`,
`crash-issue-env-is-honoured`.

**F4 (P1) — Continue in one run without extra sessions.** Add
`scripts/director-loop.sh`, started once with
`runbg run director -- bash scripts/director-loop.sh`: it re-reads bounds via
`scripts/run-bounds.sh`, then loops: call the selector → if `NO-PROGRESS` or a
bound, record `RUN-COMPLETE <reason>` and stop → otherwise run one fresh
`opencode run --agent director` subprocess with the printed assertion as an
explicit directive → verify the handoff revision advanced (that is the tick's
completion proof) → repeat. Each tick still gets a fresh context, which is what
[`unattended-loop-research.md:17`](unattended-loop-research.md#L17) argues for;
the human starts one run and `runbg stop director` halts it. Do **not** reuse
[`autonomy-driver.sh`](../../scripts/autonomy-driver.sh) for this: its `while` loop
works [`AUTONOMOUS_BACKLOG.md`](../AUTONOMOUS_BACKLOG.md) via `batch-controller`
and never launches a director (lines 191-233, 393-398). Fixture
`scripts/tests/director-loop.sh` with stub `opencode`/`gh` should assert
`bounded-ticks`, `no-progress-halts`, `resume-after-interrupted-tick`, and
`stop-flag-at-tick-boundary`.

**F5 (P1) — Gate the tests.** Add the two new director fixtures to the curated
`tooling` list in [`check-quality.sh:198-240`](../../scripts/check-quality.sh#L198-L240)
and name them in [`TESTING.md:799-805`](../TESTING.md#L799-L805). Without this,
A12 remains: a wrong-target test stays wrong and green.

**F6 (P2) — Consistency cleanup.** Point the kill switch at `runbg stop
director` (no `director-loop.timer` exists), align the queue contract with the
new handoff schema, and state that a `deferred` row is owned by its linked issue
until that issue closes. This is documentation only; F1-F3 carry the mechanism.

## 4. What could not be verified

- Who launched each of the five #49 ticks and whether `ROBIN_ISSUE` was set in
  their environments. Their comments are the only record; no session log was
  read. The tick author is the owner account in both the script-produced and
  hand-written formats, so the author field does not distinguish them.
- Whether #49 was ever intentionally admitted and then unlabelled: its label
  events show only `enhancement` (2026-11-19), with no `ready-for-agent` event.
- The historical meaning of "the live selector printed #76" in attempts 3-4.
  #76 is not open now; the current selector prints #80.
- The Nix source that generates `robin-crash-dispatch.service` could not be
  found by text search in `~/nixos-config`; the unit was read from its Nix
  store path on `oci-vm` instead.
- No isolated recheck of H14/H28/H33 was run here. D09's "passes in isolation"
  claim comes from [`DEFECTS.md:46`](../DEFECTS.md#L46) and the tick comments.
- `scripts/director-writeback.sh` is known to have produced one comment (#83,
  2026-11-24T17:18:43Z) from a tracker search; whether it was invoked on any
  issue without posting (failed writeback) cannot be determined from the
  tracker alone.
- The exact contents of earlier versions of the handoff are unavailable: only
  the current file (state LANDED, mtime 2026-11-24 10:54) was read.
