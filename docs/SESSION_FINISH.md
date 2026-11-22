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

[BACKLOG.md](BACKLOG.md) owns what may be frozen; this section owns how, and the
criterion that says the freeze is complete.

OpenCode replays a pending tool call on every runtime restart, so `interrupt`
alone can relaunch a frozen session's `runbg` job without anyone resuming it.

1. Stop every job the session started with `runbg stop <name>`; do not kill the
   runtime or another session's job.
2. Interrupt the session with `opencode api POST /api/session/<id>/interrupt`.
   It leaves the session and history intact; `delete` is irreversible and only
   for data you truly want gone.
3. Add a [BACKLOG.md](BACKLOG.md) entry: feature, target release and why it is
   deferred. Do this before renaming the session.
4. Rename the session `[FROZEN] <feature> — backlog <release>`.
5. Keep the worktree and branch.

### Completion criterion

Two branches, one trigger each:

- `PENDING` — trigger: a tool part in a `[FROZEN]` session's `session_message`
  has `state.status` `running` or `pending`, or one of its `runbg` jobs is
  active. Not frozen; run the revert recipe below and re-check.
- `CLEAR` — trigger: zero such tool parts and zero `runbg` jobs, observed after
  a runtime restart (or directly in `session_message`). Frozen; only this
  branch may end the session.

`scripts/backlog-check.sh` proves the DB half and fails the `PENDING` branch;
`runbg list` proves the job half. Neither is a claim in a title.

### Revert recipe (`PENDING` only)

`interrupt` has already been tried. Drop the message that still carries the
pending call, then stop what it relaunched and interrupt again:

```sh
# Find <msg>: a tool part whose state.status is running or pending.
sqlite3 -readonly ~/.local/share/opencode/opencode.db "
  SELECT m.id, s.title, json_extract(p.value,'\$.name')
  FROM session_message m
  JOIN session_v2 s ON s.id=m.session_id
  JOIN json_each(m.data,'\$.content') p
  WHERE s.title LIKE '[FROZEN]%'
    AND m.type='assistant' AND json_extract(p.value,'\$.type')='tool'
    AND json_extract(p.value,'\$.state.status') IN ('running','pending');"
opencode api POST /api/session/<id>/revert/stage \
  -d '{"messageID":"<msg>","files":false}'
opencode api POST /api/session/<id>/revert/commit
runbg stop <name>
opencode api POST /api/session/<id>/interrupt
```

`files:false` is the point: the message leaves the history, the worktree and
branch stay. Re-run `scripts/backlog-check.sh` to reach `CLEAR`.

## Builds

Every gate runs on minipc via `scripts/remote-check.sh` / `scripts/land.sh`.
No Rust build on `oci-vm` — the devShell's `scripts/cargo-runtime-guard.sh`
enforces that. Parallel sessions queue through `gate-shared`; never kill a
gate belonging to another session.
