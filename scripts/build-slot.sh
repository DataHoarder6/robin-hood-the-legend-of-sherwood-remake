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
# the whole machine, a contended one gets its share.
#
# The slot is a flock held for the life of CMD, so the kernel releases it if the
# build dies; nothing has to clean up after a killed agent.
set -euo pipefail

usage() {
    sed -n '2,8p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
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

cores=$(nproc)
mkdir -p -- "$slot_dir"

# fds 8 and 9: 9 holds the acquired slot, 8 probes whether another is taken.
held=
while [[ -z $held ]]; do
    for (( n = 0; n < slots; n++ )); do
        # Rotate the start so two waiters do not both queue on slot 1.
        i=$(( ( ($$ + n) % slots ) + 1 ))
        exec 9>"$slot_dir/$i"
        if flock -w 1 9; then
            slot=$i
            break
        fi
        exec 9>&-
    done
    [[ -n ${slot:-} ]] || continue

    # My slot is held; count the others that are too.
    held=1
    for (( j = 1; j <= slots; j++ )); do
        (( j == slot )) && continue
        exec 8>"$slot_dir/$j"
        if ! flock -n 8; then held=$(( held + 1 )); fi
        exec 8>&-
    done
done

jobs=$(( cores / held ))
(( jobs >= 1 )) || jobs=1

printf 'build-slot: slot %s/%s of %s, %s of %s cores\n' \
    "$slot" "$slots" "$held" "$jobs" "$cores" >&2

export CARGO_BUILD_JOBS=$jobs
exec "$@"
