#!/usr/bin/env bash
# Run-level orchestrator for one bounded autonomous run of robin-hood-remake.
# It re-reads the immutable contract, fails admission closed on invalid bounds
# or exhausted quota, selects ready work from the backlog, drives one batch at a
# time through the batch controller, meters spend, enforces the contract's
# circuit breaker and per-gate timeout, emits the four iteration terminal states
# plus RELEASED/ROLLED-BACK, and records everything in a resumable status file.
#
#   runbg run autonomy -- bash scripts/autonomy-driver.sh          # start/resume
#   bash scripts/autonomy-driver.sh resume <run-id>               # explicit resume
#   bash scripts/autonomy-driver.sh attach                        # read-only view
#   bash scripts/autonomy-driver.sh status                        # status file
#   bash scripts/autonomy-driver.sh stop                          # stop flag
#
# Environment: ROBIN_AUTONOMY_STATE, ROBIN_AUTONOMY_CONTRACT, ROBIN_AUTONOMY_SCRIPTS,
# ROBIN_QUOTA_STATUS (or $STATE/quota), ROBIN_ITEM_HEAD_DIR, ROBIN_BACKLOG_FILE,
# ROBIN_RELEASE_BINARY, ROBIN_RELEASE_SUMMARY, ROBIN_RUN_TOKENS, ROBIN_RUN_COST.
# It is repo-local: no Nix files, timer units, provider selection or installs.
set -euo pipefail

repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
cd -- "$repository"

# A run holds the gate slot and the batch worktrees for hours. A bare ssh/agent
# shell dies with the tool call; runbg sets RUNBG_STATE. Fixtures opt in.
if [[ -z ${RUNBG_STATE:-} ]] && [[ -z ${ROBIN_AUTONOMY_ALLOW_BARE:-} ]]; then
    printf 'autonomy: run me as: runbg run autonomy -- bash scripts/autonomy-driver.sh\n' >&2
    exit 2
fi

scripts=${ROBIN_AUTONOMY_SCRIPTS:-$repository/scripts}
state=${ROBIN_AUTONOMY_STATE:-${XDG_STATE_HOME:-$HOME/.local/state}/robin-autonomy}
contract=${ROBIN_AUTONOMY_CONTRACT:-$repository/docs/AUTONOMOUS_RUN_CONTRACT.md}
heads=${ROBIN_ITEM_HEAD_DIR:-$state/heads}
release_binary=${ROBIN_RELEASE_BINARY:-}
release_summary=${ROBIN_RELEASE_SUMMARY:-}
tokens=${ROBIN_RUN_TOKENS:-0}
cost=${ROBIN_RUN_COST:-0}

usage() {
    printf 'usage: autonomy-driver.sh [run|resume <id>|attach|status|stop]\n' >&2
    exit 2
}
die() { printf 'autonomy: %s\n' "$*"; exit 1; }

# Every write goes through guard-edit first, so the contract's immutability is
# enforced at runtime rather than only in the agent permission config.
assert_writable() {
    if ! bash "$scripts/run-bounds.sh" guard-edit "$1" >/dev/null 2>&1; then
        printf 'HALTED NEEDS-DECISION contract-is-immutable\n'
        exit 0
    fi
}
write_file() { # PATH TEXT
    assert_writable "$1"
    printf '%s\n' "$2" >"$1"
}
write_status() { # KEY VALUE
    local key=$1 value=$2 tmp
    assert_writable "$STATUS_FILE"
    tmp=$(mktemp "${TMPDIR:-/tmp}/robin-autonomy-status.XXXXXX")
    if [ -f "$STATUS_FILE" ]; then
        grep -v "^${key}=" "$STATUS_FILE" >"$tmp" || true
    fi
    printf '%s=%s\n' "$key" "$value" >>"$tmp"
    cat -- "$tmp" >"$STATUS_FILE"
    rm -f -- "$tmp"
}
read_field() { # KEY
    [ -f "$STATUS_FILE" ] || return 0
    sed -n "s/^$1=//p" "$STATUS_FILE" | head -1
}
log_event() {
    assert_writable "$EVENTS"
    printf '%s %s\n' "$(date -u +%FT%TZ)" "$*" >>"$EVENTS"
}
meter() { # STEP
    assert_writable "$SPEND"
    printf '%s tokens=%s cost=%s step=%s\n' \
        "$(date -u +%FT%TZ)" "$tokens" "$cost" "$1" >>"$SPEND"
}

complete() { # REASON
    write_status state complete
    write_status run_end RUN-COMPLETE
    write_status reason "$1"
    log_event "RUN-COMPLETE $1"
    printf 'RUN-COMPLETE %s\n' "$1"
    exit 0
}
halt() { # STATE REASON
    write_status state halted
    write_status run_end HALTED
    write_status terminal_state "$1"
    write_status reason "$2"
    log_event "HALTED $1 $2"
    printf 'HALTED %s %s\n' "$1" "$2"
    exit 0
}

action=${1:-run}
case $action in
    status)
        id=$(cat "$state/run-id" 2>/dev/null || die 'no-active-run')
        [ -f "$state/runs/$id/status" ] || die "unknown-run: $id"
        cat -- "$state/runs/$id/status"
        exit 0
        ;;
    attach)
        id=$(cat "$state/run-id" 2>/dev/null || die 'no-active-run')
        [ -f "$state/runs/$id/status" ] || die "unknown-run: $id"
        printf 'run %s (read-only attach)\n' "$id"
        cat -- "$state/runs/$id/status"
        if [ -f "$state/runs/$id/events.log" ]; then
            printf '%s\n' '--- recent events ---'
            tail -n 20 -- "$state/runs/$id/events.log"
        fi
        printf '%s\n' 'opencode session list | opencode session export <id>'
        exit 0
        ;;
    stop)
        id=$(cat "$state/run-id" 2>/dev/null || die 'no-active-run')
        [ -d "$state/runs/$id" ] || die "unknown-run: $id"
        assert_writable "$state/runs/$id/stop"
        : >"$state/runs/$id/stop"
        printf 'stop requested for %s; runbg stop autonomy is the durable stop\n' "$id"
        exit 0
        ;;
    run | resume) ;;
    *) usage ;;
esac

mkdir -p -- "$state" "$heads"
if [ "$action" = resume ]; then
    [ $# -eq 2 ] || usage
    id=$2
    [ -d "$state/runs/$id" ] || die "unknown-run: $id"
else
    [ $# -eq 1 ] || usage
    id=$(cat "$state/run-id" 2>/dev/null || true)
    if [ -z "$id" ] || [ ! -d "$state/runs/$id" ]; then
        id="run-$(date +%Y%m%dT%H%M%S)-$$"
    fi
fi
run_dir=$state/runs/$id
mkdir -p -- "$run_dir"
write_file "$state/run-id" "$id"
STATUS_FILE=$run_dir/status
EVENTS=$run_dir/events.log
SPEND=$run_dir/spend.log
: >>"$EVENTS"
: >>"$SPEND"
if [ "$action" = resume ]; then
    write_status state resuming
    log_event "resumed $id"
    printf 'resumed %s\n' "$id"
fi
write_status run_id "$id"
write_status state running

# Admission: bounds and quota are checked before any batch work. Quota is
# fail-closed and there is no provider switch anywhere in this script.
bounds=$("$scripts/run-bounds.sh" check "$contract" 2>&1) || halt BLOCKED "bounds-invalid:$bounds"
wall=$("$scripts/run-bounds.sh" value "$contract" wall_clock_minutes) ||
    halt BLOCKED 'bounds-invalid:missing-wall_clock_minutes'
max_batches=$("$scripts/run-bounds.sh" value "$contract" max_batches) ||
    halt BLOCKED 'bounds-invalid:missing-max_batches'
red_limit=$("$scripts/run-bounds.sh" value "$contract" consecutive_red_gates) ||
    halt BLOCKED 'bounds-invalid:missing-consecutive_red_gates'
gate_timeout=$("$scripts/run-bounds.sh" value "$contract" gate_timeout_secs) ||
    halt BLOCKED 'bounds-invalid:missing-gate_timeout_secs'

quota=${ROBIN_QUOTA_STATUS:-}
if [ -z "$quota" ] && [ -f "$state/quota" ]; then
    quota=$(cat "$state/quota")
fi
if [ "$quota" != ok ]; then
    halt BLOCKED quota-exhausted
fi

started=$(date +%s)
batches=$(read_field batches)
[ -n "$batches" ] || batches=0
reds=$(read_field consecutive_reds)
[ -n "$reds" ] || reds=0
last_item=$(read_field last_item)
write_status batches "$batches"
write_status consecutive_reds "$reds"

while :; do
    if [ -e "$run_dir/stop" ]; then
        complete stopped
    fi
    if [ "$(( $(date +%s) - started ))" -ge "$(( wall * 60 ))" ]; then
        complete wall-clock-bound
    fi
    if [ "$batches" -ge "$max_batches" ]; then
        complete batch-bound
    fi

    rc=0
    if [ -n "$last_item" ]; then
        line=$("$scripts/autobacklog.sh" next-ready-after "$last_item" 2>&1) || rc=$?
    else
        line=$("$scripts/autobacklog.sh" next-ready 2>&1) || rc=$?
    fi
    if [ "$rc" -ne 0 ]; then
        complete no-ready-item
    fi
    item=${line%%|*}
    head=$(cat "$heads/$item" 2>/dev/null || true)
    [ -n "$head" ] || halt BLOCKED missing-item-head

    "$scripts/batch-controller.sh" ready "$item" "$head" >/dev/null 2>&1 ||
        halt BLOCKED ready-failed
    epoch_out=''
    epoch_out=$("$scripts/batch-controller.sh" epoch 2>&1) || rc=$?
    if [ "$rc" -eq 2 ]; then
        complete waiting-for-batch
    fi
    [ "$rc" -eq 0 ] || halt BLOCKED epoch-failed
    epoch=$(printf '%s\n' "$epoch_out" | awk '{print $2}')
    [ -n "$epoch" ] || halt BLOCKED epoch-id-missing
    batches=$((batches + 1))
    write_status batches "$batches"
    log_event "epoch $epoch members ${line}"

    terminal=LANDED
    gate_out=''
    gate_out=$(timeout "$gate_timeout" "$scripts/batch-controller.sh" gate "$epoch" 2>&1) || rc=$?
    meter gate
    if [ "$rc" -ne 0 ]; then
        case $rc in
            124 | 137) halt FILED gate-timeout ;;
        esac
        case $gate_out in
            *gate-timeout*) halt FILED gate-timeout ;;
        esac
        reds=$((reds + 1))
        write_status consecutive_reds "$reds"
        if [ "$reds" -ge "$red_limit" ]; then
            halt FILED circuit-breaker
        fi
        bisect_out=''
        rc=0
        bisect_out=$("$scripts/batch-controller.sh" bisect "$epoch" 2>&1) || rc=$?
        case $bisect_out in
            *bisect-no-landing*) halt FILED bisect-no-landing ;;
        esac
        [ "$rc" -eq 0 ] || halt BLOCKED bisect-failed
        terminal=FILED
    else
        reds=0
        write_status consecutive_reds "$reds"
        "$scripts/batch-controller.sh" land "$epoch" >/dev/null 2>&1 ||
            halt BLOCKED land-failed
    fi

    # Only a landed batch can publish a release. A breaking bump or a refused
    # release is a human decision, not something the loop resolves itself.
    if [ "$terminal" = LANDED ]; then
        worthy=''
        worthy=$("$scripts/batch-controller.sh" release-worthy "$epoch" 2>&1) || true
        token=$(printf '%s\n' "$worthy" | head -1)
        case $token in
            major-not-allowed) halt NEEDS-DECISION major-not-allowed ;;
            minor)
                release_out=''
                rc=0
                release_out=$("$scripts/auto-release.sh" "$head" \
                    "$release_binary" "$release_summary" 2>&1) || rc=$?
                if [ "$rc" -eq 0 ]; then
                    write_status release_outcome RELEASED
                    log_event "RELEASED $release_out"
                    printf 'RELEASED %s\n' "$release_out"
                else
                    # A refused release rolls the observable outcome back to the
                    # previous-good picture. The rollback command only names the
                    # target; the owner-owned pin and any install are untouched.
                    rollback_out=''
                    if [ -n "${ROBIN_APPIMAGE_DIR:-}" ] &&
                       [ -f "$scripts/appimage-retain.sh" ]; then
                        rollback_out=$("$scripts/appimage-retain.sh" rollback \
                            "$ROBIN_APPIMAGE_DIR" 2>&1) || true
                    fi
                    write_status release_outcome ROLLED-BACK
                    log_event "ROLLED-BACK $rollback_out"
                    printf 'ROLLED-BACK %s\n' "${rollback_out:-previous-good}"
                    halt NEEDS-DECISION "release-refused:$release_out"
                fi
                ;;
            none | '') ;;
            *) halt NEEDS-DECISION "unknown-release-token:$token" ;;
        esac
    fi

    last_item=$item
    write_status last_item "$last_item"
    write_status terminal_state "$terminal"
    log_event "$terminal $epoch $item"
    printf '%s %s %s\n' "$terminal" "$epoch" "$item"
done
