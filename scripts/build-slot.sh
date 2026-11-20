#!/usr/bin/env bash
# Hold one of a host's build slots for the duration of CMD, and give CMD its
# share of the cores.
#
#   scripts/build-slot.sh SLOT_DIR SLOTS CMD [ARG...]
#
# One worktree copy per agent is deliberate (a shared Cargo target directory
# lets one worktree run another's build), but it leaves the host's cores
# unarbitrated: two copies building at once each start Cargo with the default
# job count, so twelve cores run twenty-four compilers and both builds finish
# later than either would have alone.
#
# The slot pool is that missing limit. At most SLOTS processes build at once,
# and each is handed `nproc / (slots currently held)` jobs — a lone build gets
# the whole machine, a contended one gets its share. Waiters queue in arrival
# order and every wait is bounded by BUILD_SLOT_WAIT_SECS.
#
# The slot is a flock held for the life of CMD, so the kernel releases it if the
# build dies; nothing has to clean up after a killed agent. The queue ticket is
# removed on exit, and stale tickets are reaped by their deadline.
set -euo pipefail

usage() {
    sed -n '2,9p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

(( $# >= 3 )) || usage
slot_dir=$1
slots=$2
shift 2

[[ $slots =~ ^[1-9][0-9]*$ ]] || {
    printf 'build-slot: SLOTS must be a positive integer, got %s\n' "$slots" >&2
    exit 2
}

wait_secs=${BUILD_SLOT_WAIT_SECS:-7200}
[[ $wait_secs =~ ^[1-9][0-9]*$ ]] || {
    printf 'build-slot: BUILD_SLOT_WAIT_SECS must be a positive integer, got %s\n' "$wait_secs" >&2
    exit 2
}

cores=$(nproc)
mkdir -p -- "$slot_dir"
queue_dir="$slot_dir/queue"
mkdir -p -- "$queue_dir"

deadline=$(( $(date +%s) + wait_secs ))
ticket="$queue_dir/$(printf '%020d' "$deadline")-$(date +%s%N)-$$-$RANDOM"
: > "$ticket"
trap 'rm -f -- "$ticket"' EXIT TERM INT

slot=
while [[ -z $slot ]]; do
    now=$(date +%s)

    # A killed harness can leave a ticket behind; its deadline retires it.
    for old in "$queue_dir"/*; do
        [[ -f $old ]] || continue
        old_deadline=${old##*/}
        old_deadline=${old_deadline%%-*}
        (( 10#$old_deadline < now )) && rm -f -- "$old"
    done

    first=$(find "$queue_dir" -maxdepth 1 -type f -printf '%f\n' | sort | head -1)
    if [[ $first == "${ticket##*/}" ]]; then
        for (( n = 1; n <= slots; n++ )); do
            exec 9>"$slot_dir/$n"
            if flock -n 9; then
                slot=$n
                break
            fi
            exec 9>&-
        done
    fi

    [[ -n $slot ]] && break
    if (( now >= deadline )); then
        printf 'build-slot: timed out waiting for a slot after %s seconds\n' "$wait_secs" >&2
        exit 1
    fi
    sleep 0.1
done

# My slot is held; count the others that are too.
held=1
for (( j = 1; j <= slots; j++ )); do
    (( j == slot )) && continue
    exec 8>"$slot_dir/$j"
    if ! flock -n 8; then held=$(( held + 1 )); fi
    exec 8>&-
done

jobs=$(( cores / held ))
(( jobs >= 1 )) || jobs=1

printf 'build-slot: slot %s/%s of %s, %s of %s cores\n' \
    "$slot" "$slots" "$held" "$jobs" "$cores" >&2

export CARGO_BUILD_JOBS=$jobs
# The slot is now held by flock, so leave the queue before exec: exec replaces
# this shell and would skip the EXIT trap, leaking a ticket that blocks others.
rm -f -- "$ticket"
trap - EXIT TERM INT
exec "$@"
