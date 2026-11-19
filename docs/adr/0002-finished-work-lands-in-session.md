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

1. `git fetch fork`, then rebase the worktree's branch onto
   `refs/remotes/fork/main`.
2. Re-run the affected suites with `scripts/remote-check.sh`.
3. `git push fork HEAD:main` — a fast-forward of the integration ref.
4. Confirm with `git ls-remote fork refs/heads/main` that it matches `HEAD`.
5. Remove the worktree and delete the branch in the same turn.

No pull requests and no finished work parked on feature branches in this repo.
A rejected push is a fetch, rebase, re-verify and retry — never a force push.
Only green trees reach `fork/main`.

## Consequences

- Integration happens while the work is still understood; no branch archaeology.
- There is no PR review gate here. Tests and the pre-commit hook carry quality.
- Every landing is a fast-forward, so the integration history stays linear.
