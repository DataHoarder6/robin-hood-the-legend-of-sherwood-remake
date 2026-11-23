#!/usr/bin/env bash
# Fixture tests for scripts/land.sh: a synthetic repository pair in a temp dir,
# no network. The remote is a bare repository and the gate is a stub beside the
# copy of land.sh, so the run proves the landing flow's decisions — when it
# rebases, when it gates again, what it refuses — and nothing about the suites.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-land.XXXXXX")
if [ -n "${ROBIN_LAND_TEST_KEEP:-}" ]; then
    printf 'land test fixtures kept in %s\n' "$work" >&2
else
    trap 'rm -rf -- "$work"' EXIT
fi
export work

# land.sh refuses unsupervised runs (they hold the queue lock for ~30-60 min and
# a bare ssh/sshd-scope caller cannot outlive the agent's tool shell). These
# offline fixtures are supervised by this harness, so opt in for the suite;
# the bare-run case below clears it explicitly to prove the guard.
export ROBIN_LAND_ALLOW_BARE=1

export HOME=$work/home
# land.sh and the queue tests below share one state root. The queue lock and
# tickets are addressed as $HOME/.local/state/robin-land-queue further down, so
# XDG_STATE_HOME must resolve there or the holder locks a different file than
# land.sh reads. The gate stamp lives under the same root.
export XDG_STATE_HOME=$HOME/.local/state
mkdir -p "$XDG_STATE_HOME/robin-batch/gates"
export GIT_CONFIG_GLOBAL=$work/gitconfig
export GIT_CONFIG_NOSYSTEM=1
mkdir -p "$HOME"
git config --file "$GIT_CONFIG_GLOBAL" user.name Fixture
git config --file "$GIT_CONFIG_GLOBAL" user.email fixture@example.invalid
git config --file "$GIT_CONFIG_GLOBAL" init.defaultBranch main
git config --file "$GIT_CONFIG_GLOBAL" commit.gpgsign false

pass_count=0
failures=0
pass() { printf 'ok   %s\n' "$1"; pass_count=$((pass_count + 1)); }
fail() {
    printf 'FAIL %s\n' "$1"
    failures=$((failures + 1))
}
expect_status() {
    if [ "$status" -eq "$1" ]; then pass "$2"; else fail "$2 (exit $status, want $1)"; fi
}
expect_contains() {
    case $output in
        *"$2"*) pass "$1" ;;
        *)
            fail "$1 (missing '$2')"
            printf '%s\n' "$output" | sed 's/^/     /'
            ;;
    esac
}
expect_gate_line() { # LINE DESCRIPTION EXPECTED
    local actual
    actual=$(sed -n "$1p" "$FAKE_GATE_LOG")
    if [ "$actual" = "$3" ]; then
        pass "$2"
    else
        fail "$2 (gate line $1 is '$actual', want '$3')"
    fi
}
expect_absent() {
    if [ -e "$2" ]; then
        fail "$1 ($2 exists)"
        sed 's/^/     /' "$2"
    else
        pass "$1"
    fi
}
expect_output_absent() { # DESCRIPTION SUBSTRING  — assert $output does not contain $2
    case $output in
        *"$2"*) fail "$1 (unexpected '$2')" ;;
        *) pass "$1" ;;
    esac
}

# A peer landing one commit, run from a stub gate. One move per case: a second
# invocation returns immediately, or a case that expects two gates would see
# three.
peer_lands() { # NAME MESSAGE PATH CONTENT
    if [ -e "$work/$1.moved" ]; then
        return 0
    fi
    : >"$work/$1.moved"
    rm -rf "$work/$1.peer"
    git clone --quiet "$work/$1.git" "$work/$1.peer"
    printf '%s\n' "$4" >"$work/$1.peer/$3"
    git -C "$work/$1.peer" add "$3"
    git -C "$work/$1.peer" commit --quiet -m "$2"
    git -C "$work/$1.peer" push --quiet origin main
}
export -f peer_lands

# A fresh bare remote with one seed commit, a clone of it carrying the real
# land.sh, and a stub gate beside it.
case_repo() { # NAME
    git init --quiet --bare "$work/$1.git"
    git -C "$work/$1.git" symbolic-ref HEAD refs/heads/main
    git init --quiet "$work/$1.seed"
    printf 'base\n' >"$work/$1.seed/file.txt"
    git -C "$work/$1.seed" add file.txt
    git -C "$work/$1.seed" commit --quiet -m base
    git -C "$work/$1.seed" push --quiet "$work/$1.git" HEAD:main

    git clone --quiet "$work/$1.git" "$work/$1"
    mkdir -p "$work/$1/scripts"
    cp -- "$source_root/scripts/land.sh" "$work/$1/scripts/land.sh"
    cp -- "$source_root/scripts/gate-stamp.sh" "$work/$1/scripts/gate-stamp.sh"
    chmod +x "$work/$1/scripts/gate-stamp.sh"
    cat >"$work/$1/scripts/push-gate.sh" <<'GATE'
#!/usr/bin/env bash
# Stand-in for the real gate: records the (base, head) pair it was asked to
# prove, then lets a case move the remote the way a concurrent landing does.
set -euo pipefail
printf '%s %s\n' "$1" "${2:-HEAD}" >>"$FAKE_GATE_LOG"
if [ -n "${FAKE_GATE_SLEEP:-}" ]; then
    sleep "$FAKE_GATE_SLEEP"
fi
if [ -n "${FAKE_GATE_MOVE_REMOTE:-}" ]; then
    bash "$FAKE_GATE_MOVE_REMOTE"
fi
GATE
    chmod +x "$work/$1/scripts/push-gate.sh"
}

run_land() { # NAME
    status=0
    output=$(cd "$work/$1" && ROBIN_LAND_REMOTE=$work/$1.git bash scripts/land.sh 2>&1) || status=$?
}

commit_file() { # NAME MESSAGE PATH CONTENT
    printf '%s\n' "$4" >>"$work/$1/$3"
    git -C "$work/$1" add "$3"
    git -C "$work/$1" commit --quiet -m "$2"
}

remote_tip() { git -C "$work/$1.git" rev-parse refs/heads/main; }
remote_subjects() { git -C "$work/$1.git" log --format=%s refs/heads/main | tr '\n' ' '; }

# A branch that already reached the integration ref costs nothing: no gate, no
# push.
case_repo landed
commit_file landed 'my work' mine.txt mine
git -C "$work/landed" push --quiet "$work/landed.git" HEAD:main
export FAKE_GATE_LOG=$work/landed.gate.log
run_land landed
expect_status 0 'a branch already on the integration ref is a no-op'
expect_contains 'the no-op says why' 'already contains'
expect_absent 'a no-op runs no gate' "$FAKE_GATE_LOG"

# The ordinary landing: one gate, on the commit that lands, and the remote read
# back rather than trusted.
case_repo plain
commit_file plain 'my work' mine.txt mine
tip=$(git -C "$work/plain" rev-parse HEAD)
export FAKE_GATE_LOG=$work/plain.gate.log
run_land plain
expect_status 0 'a clean landing exits 0'
expect_contains 'the landing prints the remote tip it read back' "main = $tip"
expect_gate_line 1 'the gate is asked for the landing commit' \
    "$(git -C "$work/plain" rev-parse HEAD~1) $tip"
if [ "$(remote_tip plain)" = "$tip" ]; then
    pass 'the remote carries the landing commit'
else
    fail 'the remote tip is not the landing commit'
fi
if [ -z "$(git -C "$work/plain" for-each-ref refs/robin)" ]; then
    pass 'the private base ref does not outlive the run'
else
    fail "the run left refs behind: $(git -C "$work/plain" for-each-ref refs/robin)"
fi

# A gate that passed on a stale base does not authorise the push: the remote
# moves during the gate, the push is refused, the branch is rebased, and the tree
# that lands — a different tree — is gated again.
case_repo moved
commit_file moved 'my work' mine.txt mine
cat >"$work/moved.peer.sh" <<'MOVE'
#!/usr/bin/env bash
set -euo pipefail
peer_lands moved 'their work' theirs.txt peer
MOVE
export FAKE_GATE_LOG=$work/moved.gate.log
export FAKE_GATE_MOVE_REMOTE=$work/moved.peer.sh
run_land moved
unset FAKE_GATE_MOVE_REMOTE
expect_status 0 'a landing whose integration ref moved during the gate still lands'
expect_contains 'the run says the remote moved' 'is not built on it'
expect_contains 'the run says it rebased' 'rebased onto'
if [ "$(wc -l <"$FAKE_GATE_LOG")" = 2 ]; then
    pass 'the moved remote is gated twice'
else
    fail "the moved remote ran the gate $(wc -l <"$FAKE_GATE_LOG") times, want 2"
fi
first_gate=$(sed -n '1p' "$FAKE_GATE_LOG")
second_gate=$(sed -n '2p' "$FAKE_GATE_LOG")
final_tip=$(remote_tip moved)
if [ "$(awk '{print $2}' <<<"$second_gate")" = "$final_tip" ]; then
    pass 'the second gate proved the commit that landed'
else
    fail "the second gate proved $(awk '{print $2}' <<<"$second_gate"), not $final_tip"
fi
if [ "$(awk '{print $1}' <<<"$second_gate")" = "$(git -C "$work/moved.git" rev-parse 'main~1')" ]; then
    pass 'the second gate used the moved remote as its base'
else
    fail "the second gate used $(awk '{print $1}' <<<"$second_gate") as its base"
fi
if [ "$(awk '{print $1}' <<<"$first_gate")" = "$(git -C "$work/moved.git" rev-parse 'main~2')" ]; then
    pass 'the first gate used the base that was current when it ran'
else
    fail 'the first gate did not use the original base'
fi
if [ "$(awk '{print $2}' <<<"$first_gate")" != "$final_tip" ]; then
    pass 'the rebase changed the commit the first gate had proved'
else
    fail 'the landing commit is unchanged, so the re-gate proved nothing new'
fi
if [ "$(remote_subjects moved)" = 'my work their work base ' ]; then
    pass 'the remote carries every commit exactly once'
else
    fail "the remote history is: $(remote_subjects moved)"
fi
if [ -z "$(git -C "$work/moved.git" rev-list --merges refs/heads/main)" ]; then
    pass 'the landing is linear, not a merge'
else
    fail 'the landing introduced a merge commit'
fi

# A rebase onto a moved remote that conflicts stops before the push: the branch
# keeps its commit, the remote keeps the other agent's, and the path is named.
case_repo conflict
printf 'mine\n' >"$work/conflict/file.txt"
git -C "$work/conflict" commit --quiet -am 'my edit'
mine=$(git -C "$work/conflict" rev-parse HEAD)
cat >"$work/conflict.peer.sh" <<'MOVE'
#!/usr/bin/env bash
set -euo pipefail
peer_lands conflict 'their edit' file.txt theirs
MOVE
export FAKE_GATE_LOG=$work/conflict.gate.log
export FAKE_GATE_MOVE_REMOTE=$work/conflict.peer.sh
run_land conflict
unset FAKE_GATE_MOVE_REMOTE
their_tip=$(remote_tip conflict)
expect_status 1 'a conflicting rebase stops the landing'
expect_contains 'the refusal names the conflicted path' 'land:   file.txt'
expect_contains 'the refusal says the branch is unchanged' 'are unchanged'
if [ "$(git -C "$work/conflict" rev-parse HEAD)" = "$mine" ]; then
    pass 'the refusal leaves the branch at its commit'
else
    fail 'the refusal moved the branch'
fi
if [ "$(remote_tip conflict)" = "$their_tip" ] && [ "$their_tip" != "$mine" ]; then
    pass 'the refusal leaves the remote at the other agent commit'
else
    fail 'the refusal moved the remote'
fi
if [ -z "$(git -C "$work/conflict" status --porcelain --untracked-files=no)" ]; then
    pass 'the aborted rebase leaves no half-resolved worktree'
else
    fail 'the aborted rebase left changes behind'
fi

# The retry is bounded: a remote that moves on every gate fails the run instead
# of looping.
case_repo refused
commit_file refused 'my work' mine.txt mine
cat >"$work/refused.peer.sh" <<'MOVE'
#!/usr/bin/env bash
set -euo pipefail
count=$(( $(cat "$work/refused.count" 2>/dev/null || printf 0) + 1 ))
printf '%s\n' "$count" >"$work/refused.count"
rm -f "$work/refused.moved"
peer_lands refused 'their work' "clock-$count.txt" peer
MOVE
export FAKE_GATE_LOG=$work/refused.gate.log
export FAKE_GATE_MOVE_REMOTE=$work/refused.peer.sh
run_land refused
unset FAKE_GATE_MOVE_REMOTE
expect_status 1 'a remote that moves on every gate fails the run'
expect_contains 'the failure names the rounds it spent' 'after 3 rounds'

# A merge queue: two simultaneous landings must not race on the remote. The
# loser waits its turn instead of rebasing into a moving integration branch.
case_repo queue-a
case_repo queue-b
commit_file queue-a 'my work' mine.txt mine
commit_file queue-b 'other work' mine.txt mine
: >"$work/queue.gate.log"
export FAKE_GATE_LOG=$work/queue.gate.log
export FAKE_GATE_SLEEP=1
(cd "$work/queue-a" && ROBIN_LAND_REMOTE=$work/queue-a.git bash scripts/land.sh) \
    >"$work/queue-a.out" 2>&1 &
pid_a=$!
sleep 0.2
(cd "$work/queue-b" && ROBIN_LAND_REMOTE=$work/queue-b.git bash scripts/land.sh) \
    >"$work/queue-b.out" 2>&1 &
pid_b=$!
wait "$pid_a"; rc_a=$?
wait "$pid_b"; rc_b=$?
unset FAKE_GATE_SLEEP
if [ "$rc_a" -eq 0 ] && [ "$rc_b" -eq 0 ]; then
    pass 'queued landings both succeed'
else
    fail "queued landings failed (a=$rc_a b=$rc_b)"
fi
if [ "$(wc -l <"$FAKE_GATE_LOG")" -eq 2 ]; then
    pass 'each queued landing gates once'
else
    fail "queued landings gated $(wc -l <"$FAKE_GATE_LOG") times"
fi
if grep -q 'waiting in landing queue' "$work/queue-a.out" "$work/queue-b.out"; then
    pass 'the queued landing reports its position'
else
    fail 'no landing reported a queue position'
fi
if [ "$(remote_tip queue-a)" = "$(git -C "$work/queue-a" rev-parse HEAD)" ] &&
   [ "$(remote_tip queue-b)" = "$(git -C "$work/queue-b" rev-parse HEAD)" ]; then
    pass 'both queued commits reached their integration refs'
else
    fail 'a queued commit did not reach its integration ref'
fi

# The queue wait is bounded by a named timeout, never an indefinite stall.
case_repo queued-timeout
commit_file queued-timeout 'my work' mine.txt mine
queue_state=$HOME/.local/state/robin-land-queue
mkdir -p -- "$queue_state/tickets"
# Hold the lock directly: a ticket with a later deadline would sort behind ours
# and let us through, which is the opposite of a blocked queue.
(
    exec 9>"$queue_state/land.lock"
    flock -x 9
    sleep 3
) &
holder=$!
sleep 0.2
export ROBIN_LAND_QUEUE_WAIT_SECS=1
run_land queued-timeout
unset ROBIN_LAND_QUEUE_WAIT_SECS
kill "$holder" 2>/dev/null || true
wait "$holder" 2>/dev/null || true
expect_status 1 'a blocked landing times out at its bound'
expect_contains 'the timeout is named' 'timed out waiting for a landing slot'

# A killed agent leaves a future-dated ticket behind. Its owner PID is gone, so
# the queue must reap it instead of blocking every later landing for hours.
case_repo stale-ticket
commit_file stale-ticket 'my work' mine.txt mine
stale_dir=$HOME/.local/state/robin-land-queue/tickets
mkdir -p -- "$stale_dir"
stale_deadline=$(( $(date +%s) + 30 ))
stale_name=$(printf '%020d' "$stale_deadline")-stale-99999999-1
: >"$stale_dir/$stale_name"
export ROBIN_LAND_QUEUE_WAIT_SECS=2
run_land stale-ticket
unset ROBIN_LAND_QUEUE_WAIT_SECS
expect_status 0 'a dead holder does not block the queue'
if [ ! -e "$stale_dir/$stale_name" ]; then
    pass 'the dead holder ticket is reaped'
else
    fail 'the dead holder ticket was left behind'
fi

# A bare land.sh is never correct: it would hold the landing FIFO for the whole
# gate with no supervisor to outlive the agent's tool shell. Refuse before a
# ticket is created, and print the exact remedy.
case_repo bare
commit_file bare 'my work' mine.txt mine
ticket_dir=$HOME/.local/state/robin-land-queue/tickets
before=$(find "$ticket_dir" -type f 2>/dev/null | wc -l)
unset ROBIN_LAND_ALLOW_BARE
status=0
output=$(cd "$work/bare" && env -u RUNBG_STATE bash scripts/land.sh 2>&1) || status=$?
export ROBIN_LAND_ALLOW_BARE=1
after=$(find "$ticket_dir" -type f 2>/dev/null | wc -l)
expect_status 2 'a bare land.sh is refused'
expect_contains 'the refusal prints the runbg remedy' 'runbg run NAME -- bash scripts/land.sh'
if [ "$before" = "$after" ]; then
    pass 'the refusal creates no queue ticket'
else
    fail "the refusal changed the ticket count ($before -> $after)"
fi

# A genuine runbg job carries RUNBG_STATE; that landing proceeds unchanged.
case_repo supervised
commit_file supervised 'my work' mine.txt mine
export FAKE_GATE_LOG=$work/supervised.gate.log
status=0
output=$(cd "$work/supervised" && ROBIN_LAND_REMOTE=$work/supervised.git RUNBG_STATE=$work/runbg-state bash scripts/land.sh 2>&1) || status=$?
expect_status 0 'a supervised land.sh proceeds'

# --- Gate pass-through: only a controller-owned stamp may skip the gate. ---
stamp_helper=$source_root/scripts/gate-stamp.sh
case_base() { git --git-dir="$work/$1.git" rev-parse refs/heads/main; }
head_tree() { git -C "$work/$1" rev-parse 'HEAD^{tree}'; }
unset ROBIN_LAND_GATE_PASSED_TREE ROBIN_LAND_GATE_PASSED_FILE

# (a) No stamp: the gate still runs, as before.
case_repo stamp-none
commit_file stamp-none 'my work' mine.txt mine
export FAKE_GATE_LOG=$work/stamp-none.gate.log
run_land stamp-none
expect_status 0 'a landing without a stamp succeeds'
if [ "$(wc -l <"$FAKE_GATE_LOG")" -eq 1 ]; then pass 'no stamp runs the gate'; else fail 'the gate did not run once'; fi

# (b) Correct internal stamp, exact tree and base: the gate is skipped.
case_repo stamp-good
commit_file stamp-good 'my work' mine.txt mine
base=$(case_base stamp-good)
tree=$(head_tree stamp-good)
bash "$stamp_helper" write "$tree" "$base" green
export FAKE_GATE_LOG=$work/stamp-good.gate.log
export ROBIN_LAND_GATE_PASSED_TREE=$tree
run_land stamp-good
unset ROBIN_LAND_GATE_PASSED_TREE
expect_status 0 'a stamped landing succeeds'
expect_contains 'the skip is reported' 'gate skipped'
if [ ! -e "$FAKE_GATE_LOG" ] || [ ! -s "$FAKE_GATE_LOG" ]; then
    pass 'a valid stamp runs no gate'
else
    fail "a valid stamp still ran the gate: $(cat "$FAKE_GATE_LOG")"
fi

# (c) A stamp for a different tree never authorises a skip.
case_repo stamp-wrong-tree
commit_file stamp-wrong-tree 'my work' mine.txt mine
base=$(case_base stamp-wrong-tree)
bash "$stamp_helper" write deadbeefdeadbeefdeadbeefdeadbeefdeadbeef "$base" green
export FAKE_GATE_LOG=$work/stamp-wrong-tree.gate.log
export ROBIN_LAND_GATE_PASSED_TREE=deadbeefdeadbeefdeadbeefdeadbeefdeadbeef
run_land stamp-wrong-tree
unset ROBIN_LAND_GATE_PASSED_TREE
expect_status 0 'a wrong-tree stamp does not block the landing'
if [ "$(wc -l <"$FAKE_GATE_LOG")" -eq 1 ]; then pass 'a wrong-tree stamp runs the gate'; else fail 'the gate was skipped for the wrong tree'; fi
expect_output_absent 'no skip was reported' 'gate skipped'

# (d) A stamp file that is not green fails closed into a gate.
case_repo stamp-red
commit_file stamp-red 'my work' mine.txt mine
base=$(case_base stamp-red)
tree=$(head_tree stamp-red)
bash "$stamp_helper" write "$tree" "$base" red
export FAKE_GATE_LOG=$work/stamp-red.gate.log
export ROBIN_LAND_GATE_PASSED_TREE=$tree
run_land stamp-red
unset ROBIN_LAND_GATE_PASSED_TREE
expect_status 0 'a red stamp does not block the landing'
if [ "$(wc -l <"$FAKE_GATE_LOG")" -eq 1 ]; then pass 'a red stamp runs the gate'; else fail 'a red stamp skipped the gate'; fi
expect_output_absent 'no skip was reported for red' 'gate skipped'

# (d2) The stamp is bound to the base as well as the tree.
case_repo stamp-wrong-base
commit_file stamp-wrong-base 'my work' mine.txt mine
tree=$(head_tree stamp-wrong-base)
bash "$stamp_helper" write "$tree" deadbeefdeadbeefdeadbeefdeadbeefdeadbeef green
export FAKE_GATE_LOG=$work/stamp-wrong-base.gate.log
export ROBIN_LAND_GATE_PASSED_TREE=$tree
run_land stamp-wrong-base
unset ROBIN_LAND_GATE_PASSED_TREE
expect_status 0 'a wrong-base stamp does not block the landing'
if [ "$(wc -l <"$FAKE_GATE_LOG")" -eq 1 ]; then pass 'a wrong-base stamp runs the gate'; else fail 'the gate was skipped for the wrong base'; fi

# (d3) A caller-chosen stamp file path is ignored: only the internal path counts.
# The forged file is exactly `green` — the value the old vulnerable land.sh
# trusted from ROBIN_LAND_GATE_PASSED_FILE — so reverting that read would skip.
case_repo stamp-forged-file
commit_file stamp-forged-file 'my work' mine.txt mine-forged
tree=$(head_tree stamp-forged-file)
forged=$work/forged-stamp
printf 'green\n' >"$forged"
export FAKE_GATE_LOG=$work/stamp-forged-file.gate.log
export ROBIN_LAND_GATE_PASSED_TREE=$tree ROBIN_LAND_GATE_PASSED_FILE=$forged
run_land stamp-forged-file
unset ROBIN_LAND_GATE_PASSED_TREE ROBIN_LAND_GATE_PASSED_FILE
expect_status 0 'a forged file path does not block the landing'
if [ "$(wc -l <"$FAKE_GATE_LOG")" -eq 1 ]; then pass 'a forged file path runs the gate'; else fail 'a forged file path skipped the gate'; fi
expect_output_absent 'no skip was reported for a forged file' 'gate skipped'

# (e) A new commit changes the head tree and invalidates the stamp.
case_repo stamp-head-change
commit_file stamp-head-change 'first' mine.txt one
base=$(case_base stamp-head-change)
tree=$(head_tree stamp-head-change)
bash "$stamp_helper" write "$tree" "$base" green
commit_file stamp-head-change 'second' other.txt two
export FAKE_GATE_LOG=$work/stamp-head-change.gate.log
export ROBIN_LAND_GATE_PASSED_TREE=$tree
run_land stamp-head-change
unset ROBIN_LAND_GATE_PASSED_TREE
expect_status 0 'a changed head still lands'
if [ "$(wc -l <"$FAKE_GATE_LOG")" -eq 1 ]; then pass 'a changed head tree runs the gate'; else fail 'a stale stamp skipped the gate'; fi
expect_output_absent 'no skip was reported after the head moved' 'gate skipped'

printf '\n%s checks passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
