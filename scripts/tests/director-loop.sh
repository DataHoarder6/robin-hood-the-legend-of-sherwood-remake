#!/usr/bin/env bash
# Fixture tests for the one-run director driver: stubs for selector/opencode.
set -euo pipefail
source_root=$(cd -- "$(dirname -- "$0")/../.." && pwd -P)
driver=$source_root/scripts/director-loop.sh
state=$source_root/scripts/director-state.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-director-loop.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

stubs=$work/stubs
mkdir -p "$stubs"
cat >"$stubs/selector" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$SELECTOR_LINE"
printf 'SELECT\n' >>"$EVENT_LOG"
STUB
cat >"$stubs/opencode" <<'STUB'
#!/usr/bin/env bash
printf 'START %s\n' "$*" >>"$EVENT_LOG"
rev=$(jq -r '.revision // 0' "$ROBIN_HANDOFF")
if [ -n "${STALL_ONCE:-}" ]; then
    STALL_ONCE= bash "$STATE" finish "$ROBIN_HANDOFF" "$rev" LANDED 'stub' >/dev/null
    unset STALL_ONCE
else
    bash "$STATE" finish "$ROBIN_HANDOFF" "$rev" LANDED 'stub' >>"$EVENT_LOG" 2>&1 || {
        printf 'STUB_FAIL rev=%s state=%s\n' "$rev" "$STATE" >>"$EVENT_LOG"
        exit 1
    }
fi
printf 'END\n' >>"$EVENT_LOG"
if [ -n "${STOP_AFTER_FIRST:-}" ]; then : >"$ROBIN_STOP_FLAG"; fi
STUB
chmod +x "$stubs/selector" "$stubs/opencode"

contract=$work/contract.md
cat >"$contract" <<'CONTRACT'
wall_clock_minutes: 240
max_batches: 3
consecutive_red_gates: 2
bisect_no_landing_halts: true
cost_cap: none
contract_immutable: true
gate_timeout_secs: 7200
CONTRACT
handoff=$work/handoff.json
printf '{"revision":0,"issue":49,"assertion":"D14","tree_sha":"abc123456789","attempt":1,"state":"pending","seen":{},"next_action":null}\n' >"$handoff"

export PATH="$stubs:$PATH" ROBIN_HANDOFF="$handoff" ROBIN_CONTRACT="$contract" \
    ROBIN_SELECTOR="$stubs/selector" ROBIN_OPENCODE="$stubs/opencode" \
    ROBIN_STOP_FLAG="$work/stop" STATE="$state" EVENT_LOG="$work/events.log" \
    ROBIN_MAX_TICKS=3

fail=0
pass() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1" >&2; fail=1; }
reset() {
    : >"$EVENT_LOG"
    rm -f -- "$ROBIN_STOP_FLAG"
    printf '{"revision":0,"issue":49,"assertion":"D14","tree_sha":"abc123456789","attempt":1,"state":"pending","seen":{},"next_action":null}\n' >"$handoff"
}

reset
SELECTOR_LINE='ISSUE 49|assertion=D14|sha=abc123456789|revision=1' \
    bash "$driver" >"$work/out1" 2>&1 || true
[[ $(grep -c '^START' "$EVENT_LOG") -eq 3 && "$(tail -1 "$work/out1")" == 'RUN-COMPLETE bound' ]] \
    && pass 'bounded-ticks' \
    || bad "bounded-ticks: $(cat "$EVENT_LOG") / $(cat "$work/out1")"

reset
SELECTOR_LINE='NO-PROGRESS repeat-D14' bash "$driver" >"$work/out2" 2>&1 || true
[[ $(grep -c '^START' "$EVENT_LOG") -eq 0 && "$(tail -1 "$work/out2")" == 'RUN-COMPLETE no-progress' ]] \
    && pass 'no-progress-halts' \
    || bad "no-progress-halts: $(cat "$EVENT_LOG") / $(cat "$work/out2")"

reset
SELECTOR_LINE='RESUME|49|D14|1' bash "$driver" >"$work/out3" 2>&1 || true
grep -q 'RESUME|49|D14|1' "$EVENT_LOG" \
    && pass 'resume-after-interrupted-tick' \
    || bad "resume-after-interrupted-tick: $(cat "$EVENT_LOG")"

reset
SELECTOR_LINE='ISSUE 49|assertion=D14|sha=abc123456789|revision=1' \
    STOP_AFTER_FIRST=1 bash "$driver" >"$work/out4" 2>&1 || true
[[ $(grep -c '^START' "$EVENT_LOG") -eq 1 && "$(tail -1 "$work/out4")" == 'RUN-COMPLETE stopped' ]] \
    && pass 'stop-flag-at-tick-boundary' \
    || bad "stop-flag-at-tick-boundary: $(cat "$EVENT_LOG") / $(cat "$work/out4")"

reset
SELECTOR_LINE='ISSUE 49|assertion=D14|sha=abc123456789|revision=1' \
    bash "$driver" >"$work/out5" 2>&1 || true
awk '/^START/{if (open) bad=1; open=1} /^END/{open=0} END{exit bad ? 0 : 1}' "$EVENT_LOG" \
    && bad 'no-overlapping-ticks' \
    || pass 'no-overlapping-ticks'

(( fail == 0 )) || exit 1
printf 'director-loop: OK\n'
