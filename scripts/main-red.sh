#!/usr/bin/env bash
# The integration branch's post-land state, shared by every landing on this
# host (landings run from one host; docs/PARALLEL_AGENTS.md).
#
#   scripts/main-red.sh suites            # print the failed suites; nothing when green
#   scripts/main-red.sh set SHA LOG SUITE...   # record a red post-land verification
#   scripts/main-red.sh clear SHA         # a full gate proved SHA green
#   scripts/main-red.sh show              # the whole record
#
# A red record makes push-gate run the full set plus the failed suites before a
# push (docs/adr/0014-two-tier-landing-gate.md). Only a landing whose pre-push
# gate ran that full set may clear it.
set -euo pipefail
state=${XDG_STATE_HOME:-$HOME/.local/state}/robin-land-queue
record=$state/main-red

case ${1:-} in
    suites)
        [ -f "$record" ] || exit 0
        sed -n 's/^suites: //p' "$record"
        ;;
    set)
        (( $# >= 4 )) || { echo 'usage: main-red.sh set SHA LOG SUITE...' >&2; exit 2; }
        sha=$2 log=$3
        shift 3
        mkdir -p -- "$state"
        previous=$( [ -f "$record" ] && sed -n 's/^suites: //p' "$record" || true)
        merged=$(printf '%s\n' $previous "$@" | awk 'NF && !seen[$0]++' | paste -sd' ' -)
        tmp=$(mktemp "$state/main-red.XXXXXX")
        printf 'sha: %s\nlog: %s\nsuites: %s\nsince: %s\n' \
            "$sha" "$log" "$merged" "$(date -Is)" >"$tmp"
        mv -- "$tmp" "$record"
        ;;
    clear)
        (( $# == 2 )) || { echo 'usage: main-red.sh clear SHA' >&2; exit 2; }
        if [ -f "$record" ]; then
            rm -f -- "$record"
            printf 'main-red: cleared by %s\n' "$2" >&2
        fi
        ;;
    show)
        [ -f "$record" ] && cat -- "$record" || echo green
        ;;
    *) echo 'usage: main-red.sh suites|set|clear|show' >&2; exit 2 ;;
esac
