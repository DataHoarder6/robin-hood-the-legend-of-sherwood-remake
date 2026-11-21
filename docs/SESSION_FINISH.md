# Finishing a Robin Hood session

A session is done only when its work is on `fork/main`. An ended conversation is
not a finished feature.

## The rule

Landing steps are owned by
[`docs/adr/0002-finished-work-lands-in-session.md`](adr/0002-finished-work-lands-in-session.md):
run `scripts/land.sh`, a failed gate stops the landing, then remove the worktree
and branch. Do not restate those steps here.

## Definition of done

A session may carry the `[D]` title prefix only after all of these hold:

1. `git ls-remote fork refs/heads/main` contains the session's fix commit.
2. The gate that landed it is green in `scripts/land.sh` output — not skipped,
   not `SKIP_PUSH_GATE=1`, no hand push.
3. `git worktree list` no longer contains the session's worktree.
4. `git branch --list` no longer contains its feature branch.
5. Any defect row it touched in `docs/DEFECTS.md` says `fixed` only when the
   check is in a suite the push gate runs.

Current state is printed, not quoted: use the commands above. A title is a
claim; the remote and `git worktree list` are the evidence.

## Audit

For each session, from the shared checkout:

```sh
git ls-remote fork refs/heads/main
git -C ~/git/robin-hood-remake worktree list
git -C ~/git/robin-hood-remake branch --list
git -C ~/git/robin-hood-remake log --oneline fork/main..<branch>
```

If the branch still holds commits not on `fork/main`, it is not done. If a
rebase conflict would require dropping superseded commits, stop and ask — a
fresh branch off `fork/main` with only the fix cherry-picked is the
non-destructive route. Never force a rebase through someone's commits.

## Freezing deferred work

A feature that is postponed (not abandoned) is frozen, never deleted. A feature
is frozen only if it has a [BACKLOG.md](BACKLOG.md) entry; otherwise finish it
or delete it.

1. Stop execution without removing the session:
   `opencode api POST /api/session/<id>/interrupt` — it returns
   `{"interrupted": …}` and leaves the session and its history intact. `delete`
   is only for data you truly want gone; it is irreversible.
2. Stop every job it started with `runbg stop <name>`; a session that keeps a
   job alive will respawn it. Do not kill the runtime itself.
3. Add a [BACKLOG.md](BACKLOG.md) entry: feature, target release and why it is
   deferred. Do this before renaming the session.
4. Rename the session `[FROZEN] <feature> — backlog <release>`.
5. Keep the worktree and branch.

`scripts/backlog-check.sh` flags `[FROZEN]` sessions with no entry and entries
whose worktree is gone; run it before ending the session. Freezing is a
lifecycle decision, not a patch: the code, branch and transcript all survive,
but no gate or agent runs until the backlog item is picked up.

## Builds

Every gate runs on minipc via `scripts/remote-check.sh` / `scripts/land.sh`.
No Rust build on `oci-vm` — the devShell's `scripts/cargo-runtime-guard.sh`
enforces that. Parallel sessions queue through `gate-shared`; never kill a
gate belonging to another session.
