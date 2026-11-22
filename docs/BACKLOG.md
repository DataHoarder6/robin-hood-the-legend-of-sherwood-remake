# Deferred work backlog

Frozen features are postponed, not abandoned. Freezing is a lifecycle decision
owned by [SESSION_FINISH.md](SESSION_FINISH.md) §"Freezing deferred work"; a
feature is frozen only if it has an entry here.

Live state — worktree, branch and last commit — is printed by
`scripts/backlog-check.sh` from `git worktree list`, `git branch --list` and
`git log -1 <branch>`. It is not copied into this file. The reasons below are
planning decisions, not live state.

Last verified: 2026-11-21

## Rotating map

- Target release: 0.5.0
- Why deferred: Frozen after 0.3.0. The rail-click architecture superseded the
  old fix, so it needs a fresh port, not a cherry-pick.
- A live worktree and branch are expected.

<!-- backlog: key=map-rotation live=yes -->

## Fighting mechanics

- Target release: later patch
- Why deferred: Postponed behind the 0.3.0 and 0.5.0 priorities; no live worktree.
- No live worktree.

<!-- backlog: key=fighting-mechanics live=no -->

## Unmerged gate guard

- Target release: later patch
- Why deferred: The guard change was not merged and waits for a later patch
  window; no live worktree.
- No live worktree.

<!-- backlog: key=gate-guard live=no -->

## HANDOVER docs

- Target release: later patch
- Why deferred: `docs/HANDOVER.md` is referenced by `docs/DEFECTS.md` but is
  absent from the integration ref; reconstructing it is deferred.
- No live worktree.

<!-- backlog: key=handover-docs live=no -->
