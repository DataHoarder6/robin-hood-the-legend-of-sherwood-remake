#!/usr/bin/env bash
# Fixture tests for scripts/gate-lanes.sh: a synthetic gate copy with a stub
# run-suites.sh. No cargo, no build host. Proves suites are grouped by lane,
# lanes run concurrently in worktrees at the gated commit, and a failed lane
# fails the gate.
set -euo pipefail
source_root=$(cd -- "$(dirname -- "$0")/../.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-gate-lanes.XXXXXX")
trap 'rm -rf -- "$work"' EXIT
export GIT_CONFIG_GLOBAL=$work/gitconfig GIT_CONFIG_NOSYSTEM=1
git config --file "$GIT_CONFIG_GLOBAL" user.name Fixture
git config --file "$GIT_CONFIG_GLOBAL" user.email fixture@example.invalid
copy=$work/gate-shared
mkdir -p "$copy/scripts"
cp -- "$source_root/scripts/gate-lanes.sh" "$copy/scripts/"
cat >"$copy/scripts/run-suites.sh" <<'STUB'
#!/usr/bin/env bash
# Every lane waits until all of them have started, so only concurrent lanes pass.
cd -- "$(dirname -- "$0")/.."
printf '%s %s %s\n' "$(basename "$PWD")" "$(git rev-parse --short HEAD)" "$*" >>"$LANES_LOG"
: >"$LANES_DIR/$(basename "$PWD").started"
for _ in $(seq 100); do [ "$(ls "$LANES_DIR" | wc -l)" -ge "$LANES_EXPECTED" ] && break; sleep 0.05; done
[ "$(ls "$LANES_DIR" | wc -l)" -ge "$LANES_EXPECTED" ] || { echo "lanes did not overlap"; exit 1; }
[[ " $* " != *" test "* ]] || [ -z "${FAIL_TEST:-}" ]
STUB
chmod +x "$copy/scripts/"*.sh
git -C "$copy" init -q -b main && git -C "$copy" add -A && git -C "$copy" commit -qm one
commit=$(git -C "$copy" rev-parse --short HEAD)
export LANES_LOG=$work/lanes.log LANES_DIR=$work/started LANES_EXPECTED=3
mkdir -p "$LANES_DIR"
failures=0
pass() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; failures=$((failures + 1)); }

status=0
output=$(cd "$copy" && bash scripts/gate-lanes.sh format lint test wasm-changed docs 2>&1) || status=$?
[ "$status" -eq 0 ] && pass 'three concurrent lanes pass' || fail "exit $status: $output"
sorted=$(sort "$LANES_LOG")
expected=$(printf '%s\n' "gate-shared $commit format lint docs" "gate-shared-test $commit test" \
    "gate-shared-wasm $commit wasm-changed" | sort)
[ "$sorted" = "$expected" ] && pass 'suites are grouped by lane, each lane at the gated commit' ||
    fail "lanes ran: $sorted"

echo two >"$copy/second" && git -C "$copy" add second && git -C "$copy" commit -qm two
commit=$(git -C "$copy" rev-parse --short HEAD)
: >"$LANES_LOG"; rm -f "$LANES_DIR"/*
export LANES_EXPECTED=2
status=0
output=$(cd "$copy" && FAIL_TEST=1 bash scripts/gate-lanes.sh test lint 2>&1) || status=$?
[ "$status" -eq 1 ] && [[ $output == *'failed lanes: test'* ]] && pass 'a failed lane fails the gate and is named' ||
    fail "exit $status: $output"
grep -q "gate-shared-test $commit test" "$LANES_LOG" && pass 'an existing lane worktree moves to the new commit' ||
    fail "test lane commit: $(cat "$LANES_LOG")"

printf '\n%s checks failed\n' "$failures"
(( failures == 0 ))
