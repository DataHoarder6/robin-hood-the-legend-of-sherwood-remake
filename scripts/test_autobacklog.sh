#!/usr/bin/env bash
# Fixture tests for scripts/autobacklog.sh. The backlog and defects corpus are
# local files; no gate, no build host, no network. Also checks that the two
# read-only roles may file a verdict and nothing else.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-autobacklog.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

export ROBIN_BACKLOG_FILE=$work/backlog.md
export ROBIN_DEFECTS_FILE=$work/DEFECTS.md

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
record() { grep "^$1|" "$ROBIN_BACKLOG_FILE" || true; }

# The first argument is the record set; later arguments are literal lines.
backlog() {
    {
        printf '# Test backlog\n\n## Queue\n\n```text\n'
        printf '%s\n' "$@"
        printf '```\n'
    } >"$ROBIN_BACKLOG_FILE"
}

run() {
    status=0
    output=$(ROBIN_BACKLOG_FILE="$ROBIN_BACKLOG_FILE" \
        bash "$source_root/scripts/autobacklog.sh" "$@" 2>&1) || status=$?
}

# 1. next-ready prints the first ready record in file order.
backlog \
    'P001|proposed|docs|small|Fix a typo|docs/UNATTENDED.md' \
    'P002|ready|tests|small|Add a fixture test|scripts/test_x.sh' \
    'P003|ready|docs|small|Update a guide|docs/PLAY.md'
run next-ready
expect_status 0 'next-ready succeeds with a ready item'
expect_contains 'next-ready names the first ready item' 'P002|ready'
expect_absent 'next-ready skips a later ready item' 'P003|ready'

# 2. No ready item is a named failure, not an empty success.
backlog 'P001|proposed|docs|small|Fix a typo|docs/UNATTENDED.md'
run next-ready
expect_status 1 'next-ready fails when nothing is ready'
expect_contains 'the reason is named' 'no-ready-item'

# 3. Docs and tests are promotable.
backlog \
    'P001|proposed|docs|small|Fix a typo|docs/UNATTENDED.md' \
    'P002|proposed|tests|large|Rework the suite|scripts/test_x.sh'
run promote P001
expect_status 0 'a docs item promotes'
case $(record P001) in
    P001'|'ready'|'*) pass 'the promoted item is ready' ;;
    *) fail "the promoted item is still proposed: $(record P001)" ;;
esac
run promote P002
expect_status 0 'a tests item promotes regardless of scale'
case $(record P002) in
    P002'|'ready'|'*) pass 'the tests item is ready' ;;
    *) fail "the tests item is not ready: $(record P002)" ;;
esac

# 4. Small refactors promote; large ones do not.
backlog \
    'P001|proposed|refactor|small|Tidy a helper|scripts/helper.sh' \
    'P002|proposed|refactor|large|Rewrite the engine|crates/robin_engine/src/lib.rs'
run promote P001
expect_status 0 'a small refactor promotes'
run promote P002
expect_status 1 'a large refactor does not promote'
expect_contains 'the refusal is named' 'not-promotable'

# 5. Every high-risk kind stays proposed, even at small scale.
backlog \
    'P001|proposed|gameplay|small|Tune bow damage|crates/robin_rs/src/combat.rs' \
    'P002|proposed|schema|small|Change a save field|crates/robin_rs/src/save.rs' \
    'P003|proposed|protocol|small|Change the invite|crates/robin_rs/src/net.rs' \
    'P004|proposed|release|small|Change the tag|scripts/release.sh' \
    'P005|proposed|deletion|small|Remove a file|crates/robin_rs/src/lib.rs' \
    'P006|proposed|version|small|Bump major|Cargo.toml'
for id in P001 P002 P003 P004 P005 P006; do
    run promote "$id"
    expect_status 1 "$id stays proposed"
    expect_contains "$id names the refusal" 'not-promotable'
    case $(record "$id") in
        "$id|proposed|"*) pass "$id is unchanged" ;;
        *) fail "$id was changed: $(record "$id")" ;;
    esac
done

# 6. Promotion is idempotent by state and unknown items fail closed.
backlog 'P001|ready|docs|small|Already ready|docs/PLAY.md'
run promote P001
expect_status 1 'promoting a ready item is refused'
expect_contains 'the state reason is named' 'not-proposed'
run promote P999
expect_status 1 'promoting an unknown item is refused'
expect_contains 'the unknown item is named' 'unknown-item'

# 7. Reviewer verdicts overwrite the class; unknown verdicts fail closed.
backlog 'P001|proposed|docs|small|Fix a typo|docs/UNATTENDED.md'
run verdict P001 needs-work
expect_status 0 'needs-work verdict is recorded'
case $(record P001) in
    P001'|'needs-work'|'*) pass 'the class is needs-work' ;;
    *) fail "the class is wrong: $(record P001)" ;;
esac
run verdict P001 rejected
expect_status 0 'rejected verdict is recorded'
case $(record P001) in
    P001'|'rejected'|'*) pass 'the class is rejected' ;;
    *) fail "the class is wrong: $(record P001)" ;;
esac
run verdict P001 promoted
expect_status 1 'an unknown verdict is refused'
expect_contains 'the reason is named' 'unknown-verdict'
run verdict P999 needs-work
expect_status 1 'verdict on an unknown item is refused'
expect_contains 'the item reason is named' 'unknown-item'

# 8. generate-proposed appends one item with the next id.
backlog 'P007|ready|docs|small|Existing|docs/PLAY.md'
run generate-proposed 'Investigate the load flake'
expect_status 0 'generate-proposed succeeds'
expect_contains 'the new item is proposed' '|proposed|'
expect_contains 'the new item carries the title' 'Investigate the load flake'
case $output in
    P008'|'*) pass 'the next id is allocated' ;;
    *) fail "the id is wrong: $output" ;;
esac
count=$(grep -c '^P0' "$ROBIN_BACKLOG_FILE")
if [ "$count" -eq 2 ]; then pass 'exactly one item was appended'; else fail "the backlog has $count items"; fi

# 9. With no title, generate-proposed reads a deferred DEFECTS.md row.
backlog
printf '| D12 | presentation | step | check | deferred |\n' >"$ROBIN_DEFECTS_FILE"
run generate-proposed
expect_status 0 'a deferred defect supplies a proposal'
expect_contains 'the proposal names the defect' 'D12'
printf '| D12 | presentation | step | check | fixed |\n' >"$ROBIN_DEFECTS_FILE"
run generate-proposed
expect_status 1 'no source fails closed'
expect_contains 'the reason is named' 'no-proposal-source'

# 10. Permission guards: the read-only roles may file a verdict and nothing else.
for role in reviewer verifier; do
    file=$source_root/.opencode/agents/$role.md
    if grep -q 'resource: "\*"' "$file" && grep -A2 'action: "edit"' "$file" | grep -q 'effect: deny'; then
        pass "$role denies edits"
    else
        fail "$role does not deny edits"
    fi
    if grep -q 'autobacklog.sh verdict' "$file"; then
        pass "$role may file a verdict"
    else
        fail "$role may not file a verdict"
    fi
    if grep -A2 'autobacklog.sh promote' "$file" | grep -q 'effect: allow'; then
        fail "$role may promote (it must not)"
    else
        pass "$role may not promote"
    fi
done
reviewer=$source_root/.opencode/agents/reviewer.md
if grep -A2 'action: "shell"' "$reviewer" | grep -q 'effect: deny'; then
    pass 'reviewer denies shell by default'
else
    fail 'reviewer has no shell deny'
fi

# 11. The committed backlog parses and starts with no ready item.
ROBIN_BACKLOG_FILE=$source_root/docs/AUTONOMOUS_BACKLOG.md
run next-ready
expect_status 1 'the committed backlog has no ready item at rest'
expect_contains 'the committed backlog uses the named reason' 'no-ready-item'

printf '\n%s checks passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
