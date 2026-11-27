#!/usr/bin/env bash
# Run a gate's suites in concurrent lanes on the build host.
#
#   scripts/gate-lanes.sh SUITE...      # from the gate copy, at the gated commit
#
# Run from the gate copy (gate-shared), whose HEAD is the commit being gated.
# Suites are grouped by the build configuration they compile, and each group (a
# lane) runs in its own git worktree of the gate copy with its own warm target
# directory, all lanes at once. One target directory serializes every Cargo
# invocation behind its lock; lanes let the lint check, the engine and client
# test builds and the wasm check compile side by side. Suites inside a lane run
# through scripts/run-suites.sh. Lane worktrees sit beside the gate copy, outside
# the root every prune scans. Each lane's output is printed whole in lane order;
# the exit status is non-zero if any lane failed.
set -uo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

(( $# > 0 )) || { printf 'usage: scripts/gate-lanes.sh SUITE...\n' >&2; exit 2; }

lane_of() {
    case $1 in
        engine | core | contracts | scripting-llvm | assets) printf engine ;;
        client-release | client | client-check | protocols | services | parity | tools | gpu | gpu-gl | host) printf client ;;
        wasm | wasm-tests | wasm-release | browser-audio) printf wasm ;;
        *) printf main ;;
    esac
}

commit=$(git rev-parse HEAD)
lanes=(main engine client wasm)
declare -A lane_suites=()
for suite in "$@"; do
    lane=$(lane_of "$suite")
    lane_suites[$lane]+="$suite "
done

logs=$(mktemp -d "${TMPDIR:-/tmp}/robin-lanes.XXXXXX")
trap 'rm -rf -- "$logs"' EXIT
started=()
pids=()
for lane in "${lanes[@]}"; do
    suites=${lane_suites[$lane]:-}
    [ -n "$suites" ] || continue
    if [ "$lane" = main ]; then
        dir=$PWD
    else
        dir=$PWD-$lane
        if [ -e "$dir/.git" ]; then
            git -C "$dir" checkout --quiet --force --detach "$commit"
        else
            git worktree add --quiet --detach "$dir" "$commit"
        fi
    fi
    # shellcheck disable=SC2086 # suites are validated names, one word each
    (cd "$dir" && bash scripts/run-suites.sh $suites) >"$logs/$lane.log" 2>&1 &
    started+=("$lane")
    pids+=("$!")
    printf 'gate-lanes: %s lane: %s\n' "$lane" "$suites" >&2
done

failed=()
for index in "${!started[@]}"; do
    lane=${started[index]}
    if wait "${pids[index]}"; then
        result=passed
    else
        result=FAILED
        failed+=("$lane")
    fi
    printf '=== lane %s %s\n' "$lane" "$result"
    cat -- "$logs/$lane.log"
done
if (( ${#failed[@]} > 0 )); then
    printf 'gate-lanes: failed lanes: %s\n' "${failed[*]}" >&2
    exit 1
fi
