#!/usr/bin/env bash
# Write one terminal state back to the selected issue. The tracker is the
# audit log; the local handoff is only a resume pointer, advanced by
# director-state.sh after the tracker write succeeds.
#
#   scripts/director-writeback.sh <LANDED|FILED|BLOCKED|NEEDS-DECISION> \
#       [attempt] [commit-short] [gate-verdict] [reason]
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
state_script=$source_root/scripts/director-state.sh

state=${1:?usage: director-writeback.sh LANDED|FILED|BLOCKED|NEEDS-DECISION [attempt] [sha] [gate] [reason]}
case $state in LANDED | FILED | BLOCKED | NEEDS-DECISION) ;; *) printf 'bad state: %s\n' "$state" >&2; exit 2 ;; esac
shift
attempt=${1:-unknown}
commit=${2:-none}
gate=${3:-none}
reason=${4:-}

repo=${ROBIN_RELEASE_REPO:-DataHoarder6/robin-hood-the-legend-of-sherwood-remake}
handoff=${ROBIN_HANDOFF:-/home/matt/.local/state/robin-director/handoff.json}

# Explicit selection only. A hashtag anywhere in a prose field is data, not an
# issue number: the old grep sent #49 write-backs to #83.
issue=${ROBIN_ISSUE:-${ROBIN_CRASH_ISSUE:-}}
if [ -z "$issue" ] && [ -f "$handoff" ]; then
    issue=$(jq -r '.issue // empty' "$handoff" 2>/dev/null || true)
fi
[[ "$issue" =~ ^[0-9]+$ ]] || {
    printf 'writeback: no explicit issue number (set ROBIN_ISSUE/ROBIN_CRASH_ISSUE or handoff.issue)\n' >&2
    exit 1
}

command -v gh >/dev/null || { printf 'writeback: missing-gh\n' >&2; exit 1; }

body=$(printf 'Director terminal state: **%s**\n\n- attempt: %s\n- commit: %s\n- gate: %s\n- reason: %s\n- written: %s\n' \
    "$state" "$attempt" "$commit" "$gate" "${reason:-none}" "$(date -u +%FT%TZ)")
gh issue comment "$issue" --repo "$repo" --body "$body" || {
    printf 'writeback: comment failed for #%s\n' "$issue" >&2
    exit 1
}
gh issue edit "$issue" --repo "$repo" --remove-label ready-for-agent --remove-label agent-working >/dev/null
case $state in
    FILED)
        gh issue edit "$issue" --repo "$repo" --add-label filed >/dev/null
        ;;
    BLOCKED | NEEDS-DECISION)
        gh issue edit "$issue" --repo "$repo" --add-label ready-for-human >/dev/null
        ;;
esac

# Only now may the local resume pointer advance. A failed tracker write must
# leave the handoff pending so the next tick retries the same iteration.
revision=$(jq -r '.revision // empty' "$handoff" 2>/dev/null || true)
if [ -n "$revision" ]; then
    bash "$state_script" finish "$handoff" "$revision" "$state" "${reason:-none}" || {
        printf 'writeback: handoff finish failed after tracker write\n' >&2
        exit 1
    }
fi

printf 'writeback: %s #%s (attempt %s, commit %s)\n' "$state" "$issue" "$attempt" "$commit"
