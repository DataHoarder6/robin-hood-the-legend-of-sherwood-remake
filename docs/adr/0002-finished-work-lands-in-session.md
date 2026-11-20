# 0002 — Finished work lands on `fork/main` in the same session

- **Status:** Accepted
- **Date:** 2026-11-20
- **Related:** [`0001`](0001-empty-box-selection-is-not-a-deselect.md)

## Context

`origin` is upstream, `land` refuses this repo, and `fork/main` is the
integration ref. A branch parked after its work is green drifts immediately:
other agents land, the author's tree falls behind, and the next session has to
rediscover the branch and rebase through unfamiliar commits. A pull request adds
the same drift as waiting time.

## Decision

When a feature's commits are verified and its suites are green, land it before
ending the session:

1. Run `scripts/land.sh`: it fetches, rebases onto `fork/main` when behind,
   gates the exact tree with `scripts/push-gate.sh`, pushes, and reads the
   remote back. It gates before opening the push connection because a long
   pre-push gate dies with SIGPIPE.
2. A failed gate stops the landing. Fix it and rerun; no `SKIP_PUSH_GATE=1`,
   no hand push, no force push.
3. Remove the worktree and delete the branch in the same turn.

No pull requests and no finished work parked on feature branches in this repo.
Only green trees reach `fork/main`.

## Consequences

- Integration happens while the work is still understood; no branch archaeology.
- There is no PR review gate here. Tests and the pre-commit hook carry quality.
- Every landing is a fast-forward, so the integration history stays linear.
