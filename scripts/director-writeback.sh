#!/usr/bin/env bash
# Write one terminal state back to the selected issue. The tracker is the
# audit log; the local handoff is only a resume pointer.
#
#   scripts/director-writeback.sh <LANDED|FILED|BLOCKED> \
#       [attempt] [commit-short] [gate-verdict] [reason]
#
# Read-only on the repo; the only writes are the issue comment and labels.
set -euo pipefail

state=${1:?usage: director-writeback.sh LANDED|FILED|BLOCKED|NEEDS-DECISION [attempt] [sha] [gate] [reason]}
case $state in LANDED | FILED | BLOCKED | NEEDS-DECISION) ;; *) printf 'bad state: %s\n' "$state" >&2; exit 2 ;; esac
shift
attempt=${1:-unknown}
commit=${2:-none}
gate=${3:-none}
reason=${4:-}

repo=${ROBIN_RELEASE_REPO:-DataHoarder6/robin-hood-the-legend-of-sherwood-remake}
handoff=${ROBIN_HANDOFF:-/home/matt/.local/state/robin-director/handoff.json}

issue=${ROBIN_ISSUE:-}
if [ -z "$issue" ] && [ -f "$handoff" ]; then
    issue=$(grep -oE '#[0-9]+' "$handoff" | head -1 | tr -d '#' || true)
fi
[ -n "$issue" ] || { printf 'writeback: no issue number in handoff or ROBIN_ISSUE\n' >&2; exit 1; }

command -v gh >/dev/null || { printf 'writeback: missing-gh\n' >&2; exit 1; }

body=$(printf 'Director terminal state: **%s**\n\n- attempt: %s\n- commit: %s\n- gate: %s\n- reason: %s\n- written: %s\n' \
    "$state" "$attempt" "$commit" "$gate" "${reason:-none}" "$(date -u +%FT%TZ)")
gh issue comment "$issue" --repo "$repo" --body "$body" || {
    printf 'writeback: comment failed for #%s\n' "$issue" >&2
    exit 1
}
gh issue edit "$issue" --repo "$repo" --remove-label ready-for-agent --remove-label agent-working >/dev/null
case $state in
    BLOCKED | NEEDS-DECISION)
        gh issue edit "$issue" --repo "$repo" --add-label ready-for-human >/dev/null
        ;;
esac
printf 'writeback: %s #%s (attempt %s, commit %s)\n' "$state" "$issue" "$attempt" "$commit"
