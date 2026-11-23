#!/usr/bin/env bash
# Fixture tests for scripts/batch-controller.sh: local git repos, stub gate and
# stub land. No build host, no remote gate, no network. The gate stub only looks
# for a marker file in the tree it is asked to prove, so bisect is deterministic.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-batch-controller.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

export GIT_AUTHOR_NAME=fixture GIT_AUTHOR_EMAIL=fixture@example.invalid
export GIT_COMMITTER_NAME=fixture GIT_COMMITTER_EMAIL=fixture@example.invalid

pass_count=0
failures=0
pass() { printf 'ok   %s\n' "$1"; pass_count=$((pass_count + 1)); }
fail() {
    printf 'FAIL %s\n' "$1"
    failures=$((failures + 1))
}
expect_status() {
    if [ "$status" -eq "$1" ]; then pass "$2"; else fail "$2 (exit $status, want $1): $output"; fi
}
expect_contains() {
    case $output in
        *"$2"*) pass "$1" ;;
        *) fail "$1 (missing '$2'): $output" ;;
    esac
}
expect_absent() {
    case $output in
        *"$2"*) fail "$1 (unexpected '$2'): $output" ;;
        *) pass "$1" ;;
    esac
}

cat >"$work/gate.sh" <<'GATE'
#!/usr/bin/env bash
printf '%s %s %s\n' "$1" "$2" "$3" >>"$FAKE_GATE_LOG"
if [ -n "${FAKE_BAD_MARKER:-}" ] &&
   git -C "$ROBIN_BATCH_REPO" cat-file -e "$2:$FAKE_BAD_MARKER" 2>/dev/null; then
    printf 'gate: red; %s is in the tree\n' "$FAKE_BAD_MARKER"
    exit 1
fi
printf 'gate: green %s\n' "$3"
GATE
# The land stub emulates land.sh's own gate, including the narrow opt-in
# pass-through: a green stamp for the exact tree being landed skips it.
cat >"$work/land.sh" <<'LAND'
#!/usr/bin/env bash
tree=$(git rev-parse 'HEAD^{tree}')
if [ "${ROBIN_LAND_GATE_PASSED_TREE:-}" = "$tree" ] &&
   [ -n "${ROBIN_LAND_GATE_PASSED_FILE:-}" ] &&
   [ "$(cat "${ROBIN_LAND_GATE_PASSED_FILE}" 2>/dev/null || true)" = green ]; then
    printf 'gate skipped for %s\n' "$tree"
else
    bash "$ROBIN_BATCH_GATE" "$(git rev-parse refs/remotes/fork/main)" \
        "$(git rev-parse HEAD)" "$tree" >/dev/null
fi
head=$(git rev-parse HEAD)
printf '%s\n' "$head" >>"$FAKE_LAND_LOG"
printf 'readback %s\n' "$head"
LAND
cat >"$work/land-lying.sh" <<'LAND'
#!/usr/bin/env bash
# Exits 0 without the read-back evidence: the controller must refuse it.
exit 0
LAND
# A gate that reproduces push-gate.sh's exact-tree refusal: it only passes when
# run from the staging worktree whose HEAD is the head under test.
cat >"$work/gate-cwd.sh" <<'GATE'
#!/usr/bin/env bash
base=$1 head=$2 tree=$3
if ! git diff --quiet "$head" -- .; then
    printf 'gate: worktree differs from the head being proved\n'
    exit 1
fi
printf '%s %s\n' "$PWD" "$head" >>"$FAKE_GATE_CWD_LOG"
printf 'gate: green %s from %s\n' "$tree" "$PWD"
GATE
chmod +x "$work/gate.sh" "$work/land.sh" "$work/land-lying.sh" "$work/gate-cwd.sh"
export ROBIN_BATCH_GATE=$work/gate.sh ROBIN_BATCH_LAND=$work/land.sh

REPO=''
fixture() { # NAME
    local name=$1
    REPO=$work/$name
    export ROBIN_BATCH_REPO=$REPO
    export ROBIN_BATCH_STATE=$work/$name.state
    export FAKE_GATE_LOG=$work/$name.gate.log
    export FAKE_LAND_LOG=$work/$name.land.log
    : >"$FAKE_GATE_LOG"
    : >"$FAKE_LAND_LOG"
    git init --quiet --bare "$REPO.git"
    git init --quiet -b main "$REPO"
    printf 'base\n' >"$REPO/file.txt"
    git -C "$REPO" add -- file.txt
    git -C "$REPO" commit -qm 'chore: base' -- file.txt
    git -C "$REPO" push --quiet "$REPO.git" HEAD:main
    git -C "$REPO" update-ref refs/remotes/fork/main HEAD
    git -C "$REPO" remote add fork "$REPO.git"
}

member() { # NAME FILE SUBJECT
    git -C "$REPO" checkout -q -b "$1" main
    printf '%s\n' "$1" >"$REPO/$2"
    git -C "$REPO" add -- "$2"
    git -C "$REPO" commit -qm "$3" -- "$2"
    git -C "$REPO" checkout -q main
}

publish() { # AGENT...
    local agent
    for agent in "$@"; do
        run ready "$agent" "$(git -C "$REPO" rev-parse "$agent")"
    done
}

run() {
    status=0
    output=$(bash "$source_root/scripts/batch-controller.sh" "$@" 2>&1) || status=$?
}
gate_calls() { wc -l <"$FAKE_GATE_LOG" | tr -d ' '; }
land_calls() { wc -l <"$FAKE_LAND_LOG" | tr -d ' '; }

# 1. FIFO order: ready tickets are cut in arrival order, not in ref-name order.
export ROBIN_BATCH_MIN=4 ROBIN_BATCH_MAX=4 ROBIN_BATCH_WAIT_SECS=3600
fixture fifo
member zeta z.txt 'docs: zeta'
member alpha a.txt 'feat: alpha'
member mid m.txt 'fix: mid'
member last l.txt 'test: last'
publish zeta alpha mid last
run epoch
expect_status 0 'an epoch departs at the maximum size'
expect_contains 'the members keep FIFO order' 'members zeta alpha mid last'
if [ "$(gate_calls)" -eq 0 ]; then pass 'assembly runs no gate'; else fail 'assembly ran a gate'; fi

# 2. A short epoch waits rather than departing below the minimum.
unset ROBIN_BATCH_MAX
export ROBIN_BATCH_MAX=5
fixture short
member a a.txt 'feat: a'
member b b.txt 'fix: b'
publish a b
run epoch
expect_status 2 'a short epoch waits'
expect_contains 'the wait is named' 'waiting 2/4'
export ROBIN_BATCH_MIN=1
run epoch
expect_status 0 'the same epoch departs when the minimum is lowered'
expect_contains 'the order is still FIFO' 'members a b'
export ROBIN_BATCH_MIN=4

# 3. Conflict preflight parks the conflicting member and names its paths.
fixture conflict
member a shared.txt 'feat: a writes'
member b shared.txt 'feat: b writes'
publish a b
export ROBIN_BATCH_MIN=1
run epoch
expect_status 0 'the epoch cuts with the non-conflicting member'
expect_contains 'the conflict is reported' 'shared.txt'
expect_contains 'the parked member is named' 'parked b'
if git -C "$REPO" rev-parse --verify -q refs/robin/parked/b >/dev/null; then
    pass 'the conflicting member has a parked ref'
else
    fail 'the conflicting member was not parked'
fi
expect_contains 'the first member is in the epoch' 'members a'

# 4. One tree-keyed gate per batch; a replay reuses the stored result.
fixture gate
member a a.txt 'feat: a'
member b b.txt 'fix: b'
publish a b
run epoch
epoch_id=$(printf '%s\n' "$output" | awk '{print $2}')
run gate "$epoch_id"
expect_status 0 'a green gate passes'
if [ "$(gate_calls)" -eq 1 ]; then pass 'the batch ran one gate'; else fail "the batch ran $(gate_calls) gates"; fi
run gate "$epoch_id"
expect_status 0 'the replay of an exact tree passes'
expect_contains 'the replay says the result is cached' 'cached-green'
if [ "$(gate_calls)" -eq 1 ]; then pass 'the replay ran no second gate'; else fail "the replay ran $(gate_calls) gates"; fi

# 4b. A different tree never reuses a stored result.
member c c.txt 'docs: c'
run ready c "$(git -C "$REPO" rev-parse c)"
run epoch
epoch_id2=$(printf '%s\n' "$output" | awk '{print $2}')
run gate "$epoch_id2"
expect_status 0 'the second tree gates independently'
if [ "$(gate_calls)" -eq 2 ]; then pass 'a new tree runs exactly one more gate'; else fail "gate log has $(gate_calls) lines"; fi

# 5. A red gate bisects: green halves land, the single bad member is quarantined.
fixture bisect
member a a.txt 'feat: a'
member b b.txt 'fix: b'
member c c.txt 'docs: c'
member bad bad.txt 'feat: bad marker'
publish a b c bad
run epoch
epoch_id=$(printf '%s\n' "$output" | awk '{print $2}')
export FAKE_BAD_MARKER=bad.txt
run gate "$epoch_id"
expect_status 1 'a red gate fails'
run bisect "$epoch_id"
expect_status 0 'bisect completes'
expect_contains 'the bad member is quarantined' 'quarantined bad'
if [ "$(land_calls)" -eq 2 ]; then pass 'both green halves landed'; else fail "land log has $(land_calls) lines"; fi
if git -C "$REPO" rev-parse --verify -q refs/robin/quarantined/bad >/dev/null; then
    pass 'the bad member has a quarantine ref'
else
    fail 'the bad member was not quarantined'
fi
if [ "$(gate_calls)" -eq 5 ]; then pass 'bisect gates each half tree once'; else fail "gate log has $(gate_calls) lines"; fi

# 6. Landing goes through land.sh and needs its read-back.
fixture land
member a a.txt 'feat: a'
publish a
run epoch
epoch_id=$(printf '%s\n' "$output" | awk '{print $2}')
batch_head=$(grep '^head ' "$ROBIN_BATCH_STATE/epochs/$epoch_id/meta" | cut -d' ' -f2)
run land "$epoch_id"
expect_status 0 'landing succeeds'
expect_contains 'the read-back is reported' 'readback'
if [ "$(land_calls)" -eq 1 ]; then pass 'land.sh was called once'; else fail "land.sh was called $(land_calls) times"; fi
if grep -qx "$batch_head" "$FAKE_LAND_LOG"; then pass 'land.sh proved the batch head'; else fail 'land.sh proved another commit'; fi

fixture lying
member a a.txt 'feat: a'
publish a
run epoch
epoch_id=$(printf '%s\n' "$output" | awk '{print $2}')
export ROBIN_BATCH_LAND=$work/land-lying.sh
run land "$epoch_id"
expect_status 1 'a lander without read-back is refused'
expect_contains 'the refusal is named' 'land-readback-missing'
export ROBIN_BATCH_LAND=$work/land.sh

# 7. The controller reuses the hardened scripts; it never pushes by hand.
controller=$source_root/scripts/batch-controller.sh
if grep -q 'scripts/land.sh' "$controller"; then pass 'the controller calls land.sh'; else fail 'the controller does not call land.sh'; fi
if grep -q 'scripts/push-gate.sh' "$controller"; then pass 'the controller calls push-gate.sh'; else fail 'the controller does not call push-gate.sh'; fi
if grep -qE 'git[[:space:]]+push' "$controller"; then fail 'the controller hand-pushes'; else pass 'the controller never hand-pushes'; fi

# 8. The gate runs from the staging worktree, not the outer checkout.
fixture cwd
member a a.txt 'feat: a'
publish a
run epoch
epoch_id=$(printf '%s\n' "$output" | awk '{print $2}')
worktree=$(sed -n 's/^worktree //p' "$ROBIN_BATCH_STATE/epochs/$epoch_id/meta")
export FAKE_GATE_CWD_LOG=$work/cwd.log
: >"$FAKE_GATE_CWD_LOG"
export ROBIN_BATCH_GATE=$work/gate-cwd.sh
run gate "$epoch_id"
expect_status 0 'the exact-tree gate passes from the staging worktree'
expect_contains 'the gate names the tree' 'gate: green'
if grep -q "$worktree" "$FAKE_GATE_CWD_LOG"; then
    pass 'the gate recorded the staging worktree as its cwd'
else
    fail "the gate ran from the wrong cwd: $(cat "$FAKE_GATE_CWD_LOG")"
fi
export ROBIN_BATCH_GATE=$work/gate.sh

# 9. One gate per batch: the pass-through stops land.sh re-gating the tree.
fixture onegate
member a a.txt 'feat: a'
publish a
run epoch
epoch_id=$(printf '%s\n' "$output" | awk '{print $2}')
run gate "$epoch_id"
run land "$epoch_id"
expect_status 0 'landing with a green stamp succeeds'
expect_contains 'the landing reports the skipped gate' 'gate skipped'
if [ "$(gate_calls)" -eq 1 ]; then pass 'the batch gated exactly once'; else fail "gate log has $(gate_calls) lines"; fi

# 9b. A wrong-tree stamp does not authorise a skip; land gates again.
fixture wrongstamp
member a a.txt 'feat: a'
publish a
run epoch
epoch_id=$(printf '%s\n' "$output" | awk '{print $2}')
run gate "$epoch_id"
tree=$(sed -n 's/^tree //p' "$ROBIN_BATCH_STATE/epochs/$epoch_id/meta")
printf 'green\n' >"$ROBIN_BATCH_STATE/gates/not-$tree"
# A caller-supplied mismatch is emulated by pointing the land stub at a
# different tree through its own env.
status=0
output=$(cd "$ROBIN_BATCH_STATE/worktrees/$epoch_id" && \
    ROBIN_LAND_GATE_PASSED_TREE=wrong ROBIN_LAND_GATE_PASSED_FILE="$ROBIN_BATCH_STATE/gates/not-$tree" \
    bash "$work/land.sh" 2>&1) || status=$?
expect_status 0 'a mismatched stamp still lands'
if [ "$(gate_calls)" -eq 2 ]; then pass 'a mismatched stamp re-runs the gate'; else fail "gate log has $(gate_calls) lines"; fi

# 10. FIFO is monotonic: re-publishing an early agent puts it at the back.
fixture republish
member a a.txt 'feat: a'
member b b.txt 'fix: b'
member c c.txt 'docs: c'
publish a b c
git -C "$REPO" checkout -q a
printf 'again\n' >"$REPO/a2.txt"
git -C "$REPO" add -- a2.txt
git -C "$REPO" commit -qm 'feat: a again' -- a2.txt
git -C "$REPO" checkout -q main
publish a
run epoch
expect_status 0 'the re-published epoch departs'
expect_contains 'the re-published agent goes to the back' 'members b c a'

printf '\n%s checks passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
