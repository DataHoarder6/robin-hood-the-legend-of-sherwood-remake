#!/usr/bin/env bash
# Fixture tests for scripts/run-bounds.sh. Every contract is a tiny local file;
# no gate, no build host, no network. The contract's bound lines are the only
# interface under test.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-run-bounds.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

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
expect_reason() {
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

# Write a contract fixture. Each argument is either KEY=VALUE (override) or
# -KEY (omit that key entirely). Unmentioned keys keep the approved values.
contract_file=$work/contract.md
contract() {
    local key value override
    local wall=240 batches=3 reds=2 bisect=true timeout=900 cap=none immutable=true
    local omit_wall=0 omit_batches=0 omit_reds=0 omit_bisect=0 omit_timeout=0 omit_cap=0 omit_immutable=0
    for override in "$@"; do
        case $override in
            -*) ;;
            *=*) key=${override%%=*}; value=${override#*=} ;;
            *) printf 'test fixture: bad override %s\n' "$override" >&2; exit 2 ;;
        esac
        case $override in
            -wall_clock_minutes) omit_wall=1 ;;
            -max_batches) omit_batches=1 ;;
            -consecutive_red_gates) omit_reds=1 ;;
            -bisect_no_landing_halts) omit_bisect=1 ;;
            -gate_timeout_secs) omit_timeout=1 ;;
            -cost_cap) omit_cap=1 ;;
            -contract_immutable) omit_immutable=1 ;;
            wall_clock_minutes=*) wall=$value ;;
            max_batches=*) batches=$value ;;
            consecutive_red_gates=*) reds=$value ;;
            bisect_no_landing_halts=*) bisect=$value ;;
            gate_timeout_secs=*) timeout=$value ;;
            cost_cap=*) cap=$value ;;
            contract_immutable=*) immutable=$value ;;
            *) printf 'test fixture: unknown key %s\n' "$override" >&2; exit 2 ;;
        esac
    done
    {
        printf '# Test contract\n\n## Bounds\n\n'
        (( omit_wall )) || printf 'wall_clock_minutes: %s\n' "$wall"
        (( omit_batches )) || printf 'max_batches: %s\n' "$batches"
        (( omit_reds )) || printf 'consecutive_red_gates: %s\n' "$reds"
        (( omit_bisect )) || printf 'bisect_no_landing_halts: %s\n' "$bisect"
        (( omit_timeout )) || printf 'gate_timeout_secs: %s\n' "$timeout"
        (( omit_cap )) || printf 'cost_cap: %s\n' "$cap"
        (( omit_immutable )) || printf 'contract_immutable: %s\n' "$immutable"
    } >"$contract_file"
}

status=0
output=''
check() {
    contract "$@"
    status=0
    output=$(bash "$source_root/scripts/run-bounds.sh" check "$contract_file" 2>&1) || status=$?
}

# 1. The approved bounds pass and print one clean token.
check
expect_status 0 'the approved contract passes'
expect_reason 'the pass prints ok' 'ok'

# 2. Each bound is read, not assumed. A different wall clock is refused by name.
check wall_clock_minutes=480
expect_status 1 'a non-4h wall clock is refused'
expect_reason 'the reason names the wall clock' 'wall-clock-not-4h'

# 3. Batch ceiling.
check max_batches=5
expect_status 1 'more than three batches is refused'
expect_reason 'the reason names the batch ceiling' 'max-batches-not-3'

# 4. Circuit breaker: two consecutive red gates, and a bisect with no landing.
check consecutive_red_gates=3
expect_status 1 'a non-two red-gate breaker is refused'
expect_reason 'the reason names the breaker' 'red-breaker-not-2'
check bisect_no_landing_halts=false
expect_status 1 'a bisect that does not halt is refused'
expect_reason 'the reason names the bisect breaker' 'bisect-halt-not-declared'

# 5. No cost cap: any cap present is refused, not just a large one.
check cost_cap=250usd
expect_status 1 'a cost cap is refused'
expect_reason 'the reason names the cap' 'cost-cap-present'
check cost_cap=none
expect_status 0 'cost_cap none is accepted'

# 6. Immutability is a declared bound the loop re-reads.
check contract_immutable=false
expect_status 1 'a mutable contract is refused'
expect_reason 'the reason names immutability' 'contract-not-immutable'

# 7. Missing keys fail closed with a named reason; no silent default.
check -max_batches
expect_status 1 'a missing bound is refused'
expect_reason 'the reason names the missing key' 'missing-max_batches'

# 8. The loop refuses to edit the contract path, but may edit other paths.
status=0
output=$(bash "$source_root/scripts/run-bounds.sh" guard-edit docs/AUTONOMOUS_RUN_CONTRACT.md 2>&1) || status=$?
expect_status 1 'editing the contract is refused'
expect_reason 'the refusal names immutability' 'contract-is-immutable'
status=0
output=$(bash "$source_root/scripts/run-bounds.sh" guard-edit docs/AUTONOMOUS_BACKLOG.md 2>&1) || status=$?
expect_status 0 'editing another doc is allowed'
expect_absent 'the other path carries no refusal' 'contract-is-immutable'

# 9. The committed contract itself satisfies its own checks.
status=0
output=$(bash "$source_root/scripts/run-bounds.sh" check "$source_root/docs/AUTONOMOUS_RUN_CONTRACT.md" 2>&1) || status=$?
expect_status 0 'the committed contract passes'
expect_reason 'the committed contract reports ok' 'ok'

# 9b. The per-gate timeout is a bound, and `value` is the driver's one source.
check gate_timeout_secs=0
expect_status 1 'a zero gate timeout is refused'
expect_reason 'the reason is named' 'gate-timeout-invalid'
check -gate_timeout_secs
expect_status 1 'a missing gate timeout is refused'
expect_reason 'the reason names the key' 'missing-gate_timeout_secs'
contract
status=0
output=$(bash "$source_root/scripts/run-bounds.sh" value "$contract_file" max_batches 2>&1) || status=$?
expect_status 0 'value reads a bound'
if [ "$output" = 3 ]; then pass 'value returns the exact value'; else fail "value returned '$output'"; fi

# 10. Immutability is enforced in the director's shell permissions too, so a
#     `sed -i` or `printf >>` cannot bypass an edit-only deny.
director=$source_root/.opencode/agents/director.md
if grep -A2 'action: "shell"' "$director" | grep -q 'AUTONOMOUS_RUN_CONTRACT.md'; then
    pass 'the director denies shell access to the contract path'
else
    fail 'the director has no shell deny for the contract path'
fi
if grep -A3 'action: "shell"' "$director" | grep -q 'effect: deny'; then
    pass 'the shell deny is a deny'
else
    fail 'the shell rule is not a deny'
fi

printf '\n%s checks passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
