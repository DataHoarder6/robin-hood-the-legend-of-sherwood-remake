#!/usr/bin/env bash
# Print the one issue the next director tick must work. Read-only: selection is
# evidence, not a state change. The tracker is the queue; the local handoff is
# only a resume pointer for a tick that was killed mid-work.
#
#   scripts/director-select-issue.sh
#
# Output is one tab-separated line: RESUME <selected item> | ISSUE <number> <title>
set -euo pipefail

repo=DataHoarder6/robin-hood-the-legend-of-sherwood-remake
handoff=${ROBIN_HANDOFF:-/home/matt/.local/state/robin-director/handoff.json}

if [ -f "$handoff" ]; then
    state=$(jq -r '.state // empty' "$handoff" 2>/dev/null || true)
    selected=$(jq -r '.selected_item // empty' "$handoff" 2>/dev/null || true)
    if [ "$state" = pending ] && [ -n "$selected" ]; then
        printf 'RESUME\t%s\n' "$selected"
        exit 0
    fi
fi

issue=$(
    gh issue list --repo "$repo" --state open --label ready-for-agent \
        --json number,title,updatedAt \
        --jq 'sort_by(.updatedAt) | .[0] | [.number, .title] | @tsv' 2>/dev/null
)
[ -n "$issue" ] || { printf 'no admitted issue is selectable\n' >&2; exit 1; }
printf 'ISSUE\t%s\n' "$issue"
