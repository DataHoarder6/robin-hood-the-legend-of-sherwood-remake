# 0012 — Reviewed recovery of parked work (supersedes 0002)

- **Status:** Accepted — Supersedes [0002](0002-finished-work-lands-in-session.md)
- **Date:** 2026-11-26
- **Related:** [`0002`](0002-finished-work-lands-in-session.md), [`0008`](0008-one-stable-gate-copy.md), [`0009`](0009-batched-agent-builds.md)

## Context

ADR 0002's same-session rule is unchanged for new work: finish, gate and land
before the session ends, with no pull request and no parked feature branch. A
green tip that was parked before that rule was accepted can still hold reviewed
work. Issue #62 is that case: the parked branch `fix/issue-62-zones` ends at
`eac502ef6`, is 205 commits behind `fork/main` with one unique commit, and
encodes a modifier choice the owner has since reversed.

Superseding rather than amending keeps 0002's history intact and puts the
exception in the repo that owns the landing decision.

## Decision

Finish in the session remains the rule. Parked work may be recovered once, in
the same session, only when all of these hold:

1. Record the parked tip's SHA. Run `git cherry -v fork/main <branch>`; exclude
   every `-` commit, because its change already exists upstream. Only `+`
   commits are candidates for review.
2. Review the delta file-by-file against current `fork/main`. Apply every
   overruled decision in the recovered work — for #62, Shift, not Super — and
   do not carry forward an invariant the owner has reversed.
3. Build in a fresh worktree off current `fork/main`. Do not rebase the stale
   branch into a landing, and do not cherry-pick unless the commit is unique,
   accepted as-is and gated on that exact tree.
4. Gate the recovered tree with `scripts/land.sh`: fetch, rebase when behind,
   gate the tree that lands, push, and read the remote back. One FIFO landing,
   nothing force-pushed, no `SKIP_PUSH_GATE`, no hand push.
5. Remove the worktree and delete the parked branch in that turn. The recorded
   SHA stays here as evidence, not as a live branch.

## Consequences

No general permission to park work, and no pull requests. Only green trees at
the current integration tip land. The exception is one-shot and
evidence-bearing: a second case needs its own decision and its own ADR. The
commands that implement the rule stay in `docs/PARALLEL_AGENTS.md`; this ADR is
the only source for when recovery is allowed.
