#!/usr/bin/env bash
# Land this worktree's HEAD on the fork's integration branch.
#
#   scripts/land.sh     # fetch, rebase if behind, gate the tree that lands, push
#
# A round is: fetch the integration tip; rebase onto it when HEAD is not already
# ahead of it; gate the tree the push will land; push; read the remote back. The
# gate proves a tree, and a rebase rewrites the commits and changes what they sit
# on, so the rebase is followed by another gate rather than by a push. If the
# remote moves between the gate and the push, the push is refused and the round
# repeats from a fresh fetch, three rounds at most. Nothing here force-pushes, and
# a rebase conflict stops the run with the paths listed and the branch untouched.
#
# The gate is `scripts/push-gate.sh`, the same one `.githooks/pre-push` runs, so
# there is one definition of which suites a landing owes. This command runs it
# *before* opening the connection to the remote: git starts its receive-pack
# conversation before the pre-push hook, and a seven-minute gate holds that
# conversation idle long enough that the push dies with SIGPIPE after the suites
# pass (measured 2026-11-19 on every long gated push). A green gate is the
# precondition here, and the hook stays in place for a hand-run `git push`.
#
# It prints the remote tip it landed and refuses to claim success until the
# remote reports the commit: a push's exit status is not evidence.
#
# Landings are admitted FIFO by a ticket under XDG_STATE_HOME/robin-land-queue.
# One lock is held from admission through the push, so a parallel agent cannot
# move the integration tip while a gate is proving this tree. ROBIN_LAND_QUEUE_WAIT_SECS
# bounds the wait; a dead holder's ticket is reaped by its PID.
set -euo pipefail

# A landing holds the FIFO queue lock for ~30-60 min and must outlive the
# agent's tool shell. runbg sets RUNBG_STATE in its units; a bare ssh/sshd-scope
# caller does not. Refuse before any ticket or lock is created. Test fixtures
# opt in explicitly with ROBIN_LAND_ALLOW_BARE=1; production never sets it.
if [[ -z ${RUNBG_STATE:-} ]] && [[ -z ${ROBIN_LAND_ALLOW_BARE:-} ]]; then
    printf 'land: run me as: runbg run NAME -- bash scripts/land.sh\n' >&2
    exit 2
fi

exec </dev/null

repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$repository"

remote=${ROBIN_LAND_REMOTE:-fork}
integration=main
rounds=3

# A landing is an integration step, so only one may run at a time. Parallel
# agents used to rebase and gate against a moving integration tip, then lose the
# push and repeat an hour-long gate. Tickets make admission FIFO; the lock is
# held from here through the push, so no other agent moves the base while a gate
# is proving a tree. This is the local form of a merge queue.
queue_state=${XDG_STATE_HOME:-$HOME/.local/state}/robin-land-queue
queue_dir=$queue_state/tickets
queue_wait=${ROBIN_LAND_QUEUE_WAIT_SECS:-7200}
if [[ ! $queue_wait =~ ^[1-9][0-9]*$ ]]; then
    printf 'land: ROBIN_LAND_QUEUE_WAIT_SECS must be a positive integer, got %s\n' "$queue_wait" >&2
    exit 2
fi
mkdir -p -- "$queue_dir"
queue_deadline=$(( $(date +%s) + queue_wait ))
ticket="$queue_dir/$(printf '%020d' "$queue_deadline")-$(date +%s%N)-$$-$RANDOM"
: >"$ticket"

usage() {
    awk 'NR > 1 && /^[^#]/{exit} NR > 1{sub(/^# ?/, ""); print}' "${BASH_SOURCE[0]}" >&2
    exit 2
}

case "${1:-}" in
    -h | --help) usage ;;
    "")
        ;;
    *)
        usage
        ;;
esac

head=$(git rev-parse --verify HEAD^{commit})

# FETCH_HEAD is one file for the whole repository, and every worktree fetches, so
# a read after a fetch can return someone else's ref. A ref named after this
# worktree is written only here. It is not FETCH_HEAD's replacement for a human,
# only for the one read below, and it is removed on exit.
landing_ref="refs/robin/land/$(printf '%s' "$repository" | git hash-object --stdin)"
trap 'git update-ref -d "$landing_ref" 2>/dev/null || true; rm -f -- "$ticket"' EXIT

fetch_integration() {
    git fetch --quiet --no-tags "$remote" "+refs/heads/$integration:$landing_ref" >/dev/null
    git rev-parse --verify "$landing_ref^{commit}"
}

# Wait until my ticket is first and the global landing lock is free, then hold
# that lock for the whole round. A killed agent releases both the fd lock and,
# after its deadline, its stale ticket; nothing needs a reaper.
acquire_landing_slot() {
    local first position count now old old_name old_deadline old_pid announced=
    while :; do
        now=$(date +%s)
        for old in "$queue_dir"/*; do
            [ -f "$old" ] || continue
            old_name=${old##*/}
            old_deadline=${old_name%%-*}
            case $old_deadline in
                *[!0-9]*) continue ;;
            esac
            old_pid=${old_name#*-}
            old_pid=${old_pid#*-}
            old_pid=${old_pid%%-*}
            # A dead holder's fd lock is released but its ticket is not. Reap it
            # by owner PID so one killed agent cannot stall the queue for hours.
            if (( 10#$old_deadline < now )) ||
               { [[ $old_pid =~ ^[0-9]+$ ]] && ! kill -0 "$old_pid" 2>/dev/null; }; then
                rm -f -- "$old"
            fi
        done

        first=$(find "$queue_dir" -maxdepth 1 -type f -printf '%f\n' | sort | head -1)
        if [ "$first" = "${ticket##*/}" ]; then
            exec 9>"$queue_state/land.lock"
            if flock -n 9; then
                printf 'land: landing slot acquired\n' >&2
                return 0
            fi
            exec 9>&-
        fi

        count=$(find "$queue_dir" -maxdepth 1 -type f | wc -l)
        position=$(find "$queue_dir" -maxdepth 1 -type f -printf '%f\n' | sort |
            awk -v t="${ticket##*/}" '$0 == t { print NR }')
        if [ "${position:-?}" != "$announced" ]; then
            printf 'land: waiting in landing queue (%s/%s)\n' "${position:-?}" "$count" >&2
            announced=${position:-?}
        fi
        if (( now >= queue_deadline )); then
            printf 'land: timed out waiting for a landing slot after %s seconds\n' \
                "$queue_wait" >&2
            exit 1
        fi
        sleep 0.2
    done
}

# Move HEAD onto BASE, or stop with the branch exactly as it was. `head` is the
# commit the push would carry, so it is re-read after a successful rebase.
integrate() {
    local base=$1 conflicts
    if git rebase --quiet "$base"; then
        head=$(git rev-parse --verify HEAD^{commit})
        printf 'land: rebased onto %s; the gate now proves %s\n' "$base" "$head" >&2
        return 0
    fi
    conflicts=$(git diff --name-only --diff-filter=U || true)
    if [ -d "$(git rev-parse --git-path rebase-merge)" ] ||
        [ -d "$(git rev-parse --git-path rebase-apply)" ]; then
        git rebase --abort
    fi
    printf 'land: rebasing %s onto %s failed; the branch and %s/%s are unchanged.\n' \
        "$head" "$base" "$remote" "$integration" >&2
    if [ -n "$conflicts" ]; then
        printf 'land: resolve these paths, then land again:\n' >&2
        printf '%s\n' "$conflicts" | sed 's/^/land:   /' >&2
    else
        printf 'land: git refused the rebase; its reason is above. A dirty worktree\n' >&2
        printf 'land: cannot be rebased, and the gate requires a clean one.\n' >&2
    fi
    exit 1
}

acquire_landing_slot

for _round in $(seq 1 "$rounds"); do
    base=$(fetch_integration)

    if git merge-base --is-ancestor "$head" "$base"; then
        printf 'land: nothing to do; %s/%s already contains %s\n' "$remote" "$integration" "$head" >&2
        exit 0
    fi

    rebased=0
    if ! git merge-base --is-ancestor "$base" "$head"; then
        printf 'land: %s/%s is at %s; HEAD %s is not built on it\n' \
            "$remote" "$integration" "$base" "$head" >&2
        integrate "$base"
        rebased=1
    fi

    # The first round always gates; a later round gates only because it rebased,
    # since that is the only thing that changes the tree or its base.
    #
    # A batch controller that has already gated THIS EXACT tree may name the
    # tree; it may NOT name a stamp file. land.sh derives that path itself from
    # the XDG state root through the shared gate-stamp helper and requires a
    # green stamp bound to the same base. A rebase changes the tree/base, so it
    # re-enables the gate. ROBIN_LAND_GATE_PASSED_FILE is deliberately ignored:
    # an arbitrary caller cannot forge a skip by pointing at its own file. This
    # is the single narrow pass-through that keeps ADR 0009's one gate per batch
    # without weakening the exact-tree rule or making SKIP_PUSH_GATE routine.
    passed_tree=${ROBIN_LAND_GATE_PASSED_TREE:-}
    stamp_helper=$repository/scripts/gate-stamp.sh
    gate_already_passed=0
    if [ -n "$passed_tree" ] && [ -x "$stamp_helper" ] &&
       [ "$passed_tree" = "$(git rev-parse --verify "$head^{tree}")" ] &&
       bash "$stamp_helper" valid "$passed_tree" "$base"; then
        gate_already_passed=1
    fi
    if [ "$gate_already_passed" = 1 ]; then
        printf 'land: gate skipped for %s; the batch tree is already proved\n' \
            "$head" >&2
    elif [ "$rebased" = 1 ] || [ "$_round" = 1 ]; then
        # push-gate writes the suites it deferred to after the push, or `#full`
        # when main is red and it ran everything (docs/adr/0014-two-tier-landing-gate.md).
        deferred_file=$(mktemp "${TMPDIR:-/tmp}/robin-land-deferred.XXXXXX")
        ROBIN_GATE_DEFERRED_FILE=$deferred_file bash scripts/push-gate.sh "$base" "$head"
    fi

    if ! SKIP_PUSH_GATE=1 git push --quiet "$remote" "HEAD:refs/heads/$integration"; then
        printf 'land: %s/%s refused the push; fetching and integrating again\n' \
            "$remote" "$integration" >&2
        continue
    fi

    landed=$(git ls-remote "$remote" "refs/heads/$integration" | cut -f1)
    if [ "$landed" = "$head" ]; then
        printf 'land: %s/%s = %s\n' "$remote" "$integration" "$landed" >&2
        if [ -n "${deferred_file:-}" ] && [ -s "$deferred_file" ]; then
            if [ "$(head -1 "$deferred_file")" = '#full' ]; then
                bash scripts/main-red.sh clear "$head"
            else
                mapfile -t deferred < "$deferred_file"
                unit=post-land-${head:0:12}
                runbg run "$unit" -- bash "$repository/scripts/post-land-verify.sh" "$head" "${deferred[@]}" >&2
                printf 'land: verifying %s after the push: runbg log %s; red stops the line\n' \
                    "${deferred[*]}" "$unit" >&2
            fi
        fi
        exit 0
    fi
    printf 'land: %s/%s is %s after the push, not %s; integrating again\n' \
        "$remote" "$integration" "$landed" "$head" >&2
done

printf 'land: %s/%s did not reach %s after %s rounds\n' \
    "$remote" "$integration" "$head" "$rounds" >&2
exit 1
