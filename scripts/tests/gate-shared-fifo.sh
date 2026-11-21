#!/usr/bin/env bash
# Fixture tests for FIFO admission on the shared gate copy's lock. No build
# host: lock_copy() is read out of the shipped remote-check.sh and run by local
# waiters that hold one fake copy lock. It proves ticket order, a logged queue
# position per waiter, and the named queue timeout.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/../.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-gate-fifo.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

export HOME=$work/home
export XDG_STATE_HOME=$work/state
mkdir -p -- "$HOME" "$XDG_STATE_HOME"

# Exercise the function the gate actually ships, not a paraphrase of it.
sed -n '/^lock_copy() {/,/^}/p' "$source_root/scripts/remote-check.sh" >"$work/lock_copy.sh"
if ! grep -q '^lock_copy() {' "$work/lock_copy.sh"; then
    printf 'FAIL lock_copy() not found in scripts/remote-check.sh\n' >&2
    exit 1
fi
# shellcheck source=/dev/null
source "$work/lock_copy.sh"

host=fake-host
name=gate-shared
failures=0

pass() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; failures=$((failures + 1)); }
expect_contains() { # FILE NEEDLE DESCRIPTION
    if grep -qF -- "$2" "$1"; then
        pass "$3"
    else
        fail "$3 (no '$2' in $1)"
        sed 's/^/     /' "$1"
    fi
}

# A waiter takes a ticket through the real lock_copy(), records the order in
# which it was admitted, holds the copy lock briefly, then releases it by
# closing fd 9. The function's own stderr is the queue log under test.
waiter() { # ID HOLD-SECS
    local id=$1 hold=$2
    if lock_copy 2>"$work/waiter-$id.queue"; then
        printf '%s\n' "$id" >>"$work/admitted"
        sleep "$hold"
        exec 9>&-
        printf 'acquired\n' >"$work/waiter-$id.result"
    else
        printf 'timeout\n' >"$work/waiter-$id.result"
        return 1
    fi
}
export -f waiter lock_copy
export work host name

queue_dir=$XDG_STATE_HOME/robin-remote-check/queue/fake-host-gate-shared
wait_for_tickets() { # N
    local i
    for i in $(seq 200); do
        [ "$(find "$queue_dir" -maxdepth 1 -type f 2>/dev/null | wc -l)" -eq "$1" ] &&
            return 0
        sleep 0.05
    done
    return 1
}

# ---- FIFO admission -------------------------------------------------------
# Three waiters, launched in a known order, each holding long enough that the
# next must queue. Admission must follow ticket order, not lock luck. Waiting
# for each ticket makes the arrival order deterministic without long sleeps.
pids=()
for id in 1 2 3; do
    waiter "$id" 1 >"$work/waiter-$id.out" 2>&1 &
    pids+=("$!")
    if ! wait_for_tickets "$id"; then
        fail "waiter $id did not take a ticket"
    fi
done
for pid in "${pids[@]}"; do wait "$pid" || true; done

admitted=$(paste -sd, "$work/admitted" 2>/dev/null || true)
if [ "$admitted" = '1,2,3' ]; then
    pass 'tickets are admitted in arrival order'
else
    fail "admission order was '$admitted', want '1,2,3'"
fi
for id in 2 3; do
    expect_contains "$work/waiter-$id.queue" "waiting in gate copy queue ($id/" \
        "waiter $id logs its queue position"
done
expect_contains "$work/waiter-3.queue" 'for gate-shared' \
    'the queue log names the copy'

# ---- Named timeout -------------------------------------------------------
# An outsider holds the lock without a ticket. A waiter bounded at one second
# must leave with a named message, not join the two-hour default queue.
lock_path=$XDG_STATE_HOME/robin-remote-check/fake-host-gate-shared.lock
mkdir -p -- "${lock_path%/*}"
(
    exec 9>"$lock_path"
    flock -x 9
    sleep 5
) &
holder=$!
sleep 0.2
set +e
(ROBIN_GATE_QUEUE_WAIT_SECS=1 waiter 4 0.1) >"$work/waiter-4.out" 2>&1
rc4=$?
set -e
kill "$holder" 2>/dev/null || true
wait "$holder" 2>/dev/null || true

if [ "$rc4" -eq 1 ]; then
    pass 'a blocked gate exits at its queue bound'
else
    fail "a blocked gate exited $rc4, want 1"
fi
expect_contains "$work/waiter-4.queue" \
    'timed out waiting for a gate copy slot after 1 seconds' \
    'the timeout names the bound'
if grep -qx '4' "$work/admitted" 2>/dev/null; then
    fail 'the timed-out waiter reached the copy'
else
    pass 'the timed-out waiter never reached the copy'
fi
if [ -e "$work/waiter-4.result" ] &&
   grep -qF 'acquired' "$work/waiter-4.result"; then
    fail 'the timed-out waiter acquired the copy'
else
    pass 'the timed-out waiter did not acquire the copy'
fi

printf '\n%s checks failed\n' "$failures"
(( failures == 0 ))
