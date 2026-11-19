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

export HOME=$work/home
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
    cat >"$work/$1/scripts/push-gate.sh" <<'GATE'
#!/usr/bin/env bash
# Stand-in for the real gate: records the (base, head) pair it was asked to
# prove, then lets a case move the remote the way a concurrent landing does.
set -euo pipefail
printf '%s %s\n' "$1" "${2:-HEAD}" >>"$FAKE_GATE_LOG"
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

printf '\n%s checks passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
