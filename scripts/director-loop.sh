#!/usr/bin/env bash
# Run bounded director ticks in one process. Each tick is a fresh-context
# `opencode run --agent director`; its completion is proved by a handoff
# revision advance, never by narration. Start once with:
#
#   runbg run director -- bash scripts/director-loop.sh
#
# Stop with `runbg stop director` or by creating ROBIN_STOP_FLAG. Every exit
# path prints one `RUN-COMPLETE <reason>` line as the run's terminal fact.
set -uo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
contract=${ROBIN_CONTRACT:-$source_root/docs/AUTONOMOUS_RUN_CONTRACT.md}
selector=${ROBIN_SELECTOR:-$source_root/scripts/director-select-issue.sh}
opencode=${ROBIN_OPENCODE:-opencode}
bounds=$source_root/scripts/run-bounds.sh
handoff=${ROBIN_HANDOFF:-/home/matt/.local/state/robin-director/handoff.json}
stop_flag=${ROBIN_STOP_FLAG:-/home/matt/.local/state/robin-director/stop}

complete() { printf 'RUN-COMPLETE %s\n' "$1"; exit "${2:-0}"; }

check=$("$bounds" check "$contract" 2>&1) || complete "contract-invalid: $check" 1
max_ticks=${ROBIN_MAX_TICKS:-$("$bounds" value "$contract" max_batches)}
wall_minutes=$("$bounds" value "$contract" wall_clock_minutes)
[[ "$max_ticks" =~ ^[0-9]+$ ]] || complete 'invalid-max-batches' 1
[[ "$wall_minutes" =~ ^[0-9]+$ ]] || complete 'invalid-wall-clock' 1
started=$SECONDS

tick=0
while (( tick < max_ticks )); do
    if [ -f "$stop_flag" ]; then complete stopped; fi
    if (( (SECONDS - started) / 60 >= wall_minutes )); then complete 'wall-clock'; fi

    selection=$("$selector") || {
        case $selection in
            NO-PROGRESS*) complete no-progress ;;
            *) complete 'selection-failed' 1 ;;
        esac
    }
    [ -n "$selection" ] || complete 'empty-selection' 1
    case $selection in NO-PROGRESS*) complete no-progress ;; esac

    before=$(jq -r '.revision // 0' "$handoff" 2>/dev/null || printf '0')
    tick=$((tick + 1))
    printf 'tick %s: %s\n' "$tick" "$selection"
    "$opencode" run --agent director \
        "Director tick. Selection: $selection. Work only this assertion and write back exactly one terminal state with scripts/director-writeback.sh." \
        || complete "tick-$tick-failed" 1

    after=$(jq -r '.revision // 0' "$handoff" 2>/dev/null || printf '0')
    if [ "$after" = "$before" ]; then complete 'stalled-handoff'; fi
done

complete bound
