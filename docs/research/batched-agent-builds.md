# Batched agent builds: one gate for N worktrees

Last verified: 2026-11-22. Research only — no script, ADR or config was changed.
External links below are official docs or project source, not summaries.

Thesis: the owner's goal is **Bors-style batched validation**, not a GitHub merge
queue. It is a known pattern with a known failure protocol (bisect the batch).
Implement it here as a controller-owned staging branch plus the repo's existing
one-gate machinery; do not try to make platform merge queues do it.

## 1. What the industry calls this

| Term | Mechanics | Primary source |
| --- | --- | --- |
| **Batched validation / staging-branch batch** (the exact goal) | Approved patches are merged into a `staging` branch; CI tests **that one tree**; on green the target is fast-forwarded to the tested commit, so "main contains the exact contents that were just tested, bit-for-bit". On red, the batch is **bisected** into halves and retested; a single failing patch is kicked back to its author. | [bors-ng README](https://github.com/bors-ng/bors-ng), [getting started](https://bors.tech/documentation/getting-started), [batcher source](https://github.com/bors-ng/bors-ng/blob/master/lib/worker/batcher.ex), [rust-lang/bors README](https://github.com/rust-lang/bors) |
| **Merge queue / merge train** | FIFO admission; each queued change is tested **against the base plus everything ahead of it in the queue**. GitHub: temporary `gh-readonly-queue/*` branches, `merge_group` event, CI timeout, failing entry removed and later groups rebuilt. GitLab: each MR gets a *cumulative* pipeline; pipelines run in parallel; limit 1 = sequential. | [GitHub merge queue](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue), [GitLab merge trains](https://docs.gitlab.com/ci/pipelines/merge_trains) |
| **Stacked PRs / grouped CI** | A stack is a sequence of dependent PRs; the unit of merging and CI can be the top of the stack rather than each PR. Graphite's "batching" runs CI **once per batch of stacks** (saving `batch_size × stack_height` runs); "fast-forward merge" runs CI on the top PR only; any failure fails the whole stack. | [Graphite merge queue](https://www.graphite.com/docs/graphite-merge-queue), [optimizations](https://www.graphite.com/docs/merge-queue-optimizations), [CI optimizations](https://www.graphite.com/docs/stacking-and-ci) |
| **Worktree-per-agent** | Linked worktrees share object DB and most refs; `HEAD`, the index and `refs/bisect`, `refs/worktree`, `refs/rewritten` are per-worktree. Codex and OpenCode both make this the multi-agent default (Codex uses detached-HEAD managed worktrees; OpenCode exposes `worktree_create`/`list`/`remove` and background subagents). | [git-worktree](https://git-scm.com/docs/git-worktree), [Codex worktrees](https://developers.openai.com/codex/app/worktrees), [OpenCode agents](https://opencode.ai/docs/agents/), [OpenCode tools](https://opencode.ai/docs/tools/#worktrees) |
| **Remote/cache-backed builds** | Bazel's remote cache = action cache + content-addressable store; its disk cache exists for "switching branches and/or working on multiple workspaces … such as multiple checkouts". Nix shares through binary-cache stores and content-addressed paths. Cargo has no shipped equivalent ([cargo#5931](https://github.com/rust-lang/cargo/issues/5931) is `S-needs-design`), so this repo's single warm `gate-shared` copy is the local stand-in. | [Bazel remote caching](https://bazel.build/remote/caching), [Nix store types](https://nix.dev/manual/nix/2.24/store/types/), [Nix 2.4 CA paths](https://nix.dev/manual/nix/2.24/release-notes/rl-2.4) |

**Critical distinction:** GitHub's "merge limits" group *merges*, not builds — its
own docs say "Merge limits do not combine `merge_group` builds". GitLab also runs
one cumulative pipeline per MR. Neither gives one build for the combined result.
Bors/Graphite batching does. Do not adopt a platform queue expecting it to solve
the cost problem.

## 2. Recommended design for this repo

Roles: **agents edit, commit and declare readiness; only the controller assembles,
gates, pushes and retires.** This keeps ADR 0002's exact-tree rule while paying
for one gate per batch.

1. **Worktree per agent.** Keep the existing rule: `.worktrees/<name>` off
   `fork/main`, shared `main` untouched, no stash, no `CARGO_TARGET_DIR`, commit
   by pathspec. See `AGENTS.md` and [`docs/PARALLEL_AGENTS.md`](../PARALLEL_AGENTS.md).
2. **Fast local checks while editing.** `bash scripts/check-quality.sh format`,
   the `client-check`/`cargo check` preflight, and the pre-commit hook — never a
   full remote gate. See [`docs/TESTING.md`](../TESTING.md). A cheap check is the
   cheapest way to stop a bad commit from poisoning the batch.
3. **Readiness marker (agent → controller).** After local checks pass and the
   commit exists, the agent publishes an immutable marker keyed by its head SHA:
   `git update-ref refs/robin/ready/<agent> <head> <old>` (compare-and-swap) plus
   a state file under `$XDG_STATE_HOME/robin-batch/ready/<epoch>/<agent>` holding
   agent id, head SHA, `HEAD^{tree}`, base SHA, checks run, timestamp. A replay
   with the same head SHA is a no-op; a new commit needs a new marker. No push to
   `fork/main`, no gate, no landing by the agent.
4. **Controller assembly (epochs).** Use the FIFO ticket idea already in
   [`scripts/land.sh`](../../scripts/land.sh): one ticket per ready branch, ordered
   by arrival, one assembly lock. Departure rule: min 4, max 5 members, wait
   timeout before departing short — the explicit analogue of GitHub's
   minimum/maximum group size and wait time, and of Bors' `batch_delay_sec`.
   Freeze `base = fork/main` tip at cut. Preflight each member in ticket order
   with `git merge-tree --write-tree <base> <head>` (exit 1 = conflicts, no
   worktree/index touched). A conflicting member is parked for the next epoch
   with its conflicting paths reported; the controller never resolves conflicts.
   Assemble in a **controller-only** worktree at `refs/robin/batch/<epoch>` by
   rebasing member commits onto base in ticket order, recording `Ready-by:
   <agent> <head>` trailers so membership is traceable without cherry-pick SHA
   churn. Do not run CI on the assembly ref itself: Bors excludes `staging.tmp`
   from CI and rust-lang/bors marks its `auto-merge` branch "should not be
   configured for any CI workflows".
5. **One gate.** From that worktree run `bash scripts/push-gate.sh <base>
   <batch-head>`; it selects suites from the combined diff and calls
   `scripts/remote-check.sh` once against the locked `gate-shared` copy. Keep one
   gate in flight across the estate and `CARGO_INCREMENTAL=0`
   ([ADR 0007](../adr/0007-no-cargo-incremental-in-gates.md),
   [ADR 0008](../adr/0008-one-stable-gate-copy.md)). Stamp the result under an
   idempotency key `(epoch, base, batch tree, suites)` — a ref
   `refs/robin/gate/<tree>` or state file. If that exact key is already green,
   a replayed job returns the stored result instead of rebuilding.
6. **Landing and cleanup.** Reuse `land.sh`'s gate → push → **remote read-back**
   sequence with `head` = batch commit; `git ls-remote` is the evidence, not the
   push exit code. One fast-forward push of `fork/main`. Then mark members landed
   with `git cherry -v fork/main <agent-branch>` / `git rev-list --count`, and
   retire their worktrees and branches (`just prune-worktrees --apply`).

ADR 0002 says finished work lands in the same session. A batch window bends that:
a ready branch can wait one epoch. Keep the wait short and have the controller
land the batch in the turn the last member becomes ready; if the deviation sticks,
record it as a new ADR rather than quietly ignoring 0002.

## 3. Grill: top five failure modes

1. **Late finishers miss the batch.** They must: the batch is frozen at cut. A
   member whose head SHA changes after cut is next epoch, never an amendment to a
   running batch — GitHub warns that jumping/reordering causes a full rebuild of
   in-progress groups, and Graphite that reordering breaks the commit graph.
   Mitigation is the explicit departure rule (min/max + wait timeout) and a
   visible queue position; publish "epoch N departs at T" so agents know the cut.
2. **Conflicts between branches.** Deterministic ticket order plus
   `git merge-tree --write-tree` preflight means conflicts are found before the
   gate, in seconds, with no worktree mutation. Park the conflicting branch,
   report the paths, have its owner rebase onto the new `fork/main`; never let
   one agent rebase onto another agent's unlanded branch. An all-conflict batch
   departs empty rather than burning a gate.
3. **Batch poisoning (one bad commit fails everything).** Adopt Bors bisection:
   split the failed batch in half, re-gate each half, land the green halves,
   quarantine the bad single member and return it to its owner. Gate results are
   keyed by tree so the halves reuse the warm copy. Mark known-parity-sensitive
   changes as "test alone" (Bors' `is_single`). Do **not** turn on GitHub's "only
   merge non-failing pull requests" analogue to hide real failures; that setting
   exists for intermittent tests, not broken commits.
4. **Replayed jobs / duplicate side effects after a runtime restart.** Assume
   at-least-once execution: Temporal's docs state that a worker can finish an
   activity and crash before reporting, so it is retried; the fix is an
   idempotency keyed on stable IDs plus a read of current state before acting.
   Here: every controller step is keyed by `(epoch, base, member heads, step)`;
   ref writes use compare-and-swap; the gate result is stored by tree and reused;
   pushes are confirmed by remote read-back. Keep fd-based locks with PID reaping
   (already in `land.sh`) and a transient-scope TTL with process-group kill (see
   [`BUILD_CACHE_AND_QUEUE.md`](../slop/BUILD_CACHE_AND_QUEUE.md) §3), so a killed
   replay cannot strand a lock or leave an orphan build. Never key state on
   attempt count.
5. **Agent churn (uneven finishes, restarts, late commits).** Freeze member head
   SHAs at cut; new commits become a new marker in a new epoch. Cap concurrency
   at 4–5 seats: Codex exposes `agents.max_concurrent_threads_per_session`,
   OpenCode's subagents run in background child sessions with `subagent_depth`
   defaulting to 1 and configurable permissions. Do not promote another agent's
   branch into a running batch, and do not let a finishing agent delete its
   worktree until the batch's remote read-back confirms landing.

## 4. What not to do

- **Do not use a GitHub merge queue or GitLab merge train as the mechanism.**
  Both still build per entry; GitLab's whole value is testing cumulative sets,
  not sharing one build. Wrong tool for the stated cost problem.
- **Do not cherry-pick member commits into the batch and call the SHA stable.**
  Cherry-pick rewrites commits; squash merging loses author/time metadata and
  makes `git rerere` less effective ([GitHub merge methods](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/about-merge-methods-on-github)).
  Rebase the exact member commits and gate the resulting tree once; record the
  mapping in trailers.
- **Do not weaken the exact-tree rule.** No "heavy suites only after merge"
  tiering, no `SKIP_PUSH_GATE=1` as a routine path, no force push, no hand push.
  `scripts/push-gate.sh` stays the one definition of what a landing owes.
- **Do not share `CARGO_TARGET_DIR` or add tree-keyed gate copies.** The repo
  forbids the former and measured the latter at 0.36 % Rust cache hits
  ([ADR 0008](../adr/0008-one-stable-gate-copy.md),
  [build reduction research](../slop/2026-11-20-build-reduction-research.md)).
  Do not plan around Cargo's unshipped CAS.
- **Do not let agents gate concurrently** or build on a fixed shared log path;
  one locked gate copy, one gate in flight. A stale/absent result must fail
  closed into "gate again", not "assume green".
- **Do not reuse an agent's worktree as the staging tree.** Use plumbing or a
  controller-only worktree; a sync cannot swap source under a running gate
  (ADR 0008's lock invariant). Bors and GitHub both keep their validation ref
  separate from the target branch for the same reason.
- **Do not assume a cache hit is proof of correctness.** Bazel's docs list input
  modification during a build as a way to upload invalid cache results, and
  `$PATH` leaking into action hashes as a hit-rate bug; a gate key must include
  the exact tree, toolchain and suite set, and a poisoned result store must be
  deleted wholesale.
- **Do not rely on git worktree for submodules/superprojects** — upstream still
  labels multiple checkout experimental with incomplete submodule support
  ([git-worktree](https://git-scm.com/docs/git-worktree)).
