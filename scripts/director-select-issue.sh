#!/usr/bin/env bash
# Select exactly one machine-checkable assertion for the next director tick.
# Deferred rows, out-of-scope rows, HUMAN_GATE items and assertions already
# FILED for the same integration tree are inadmissible. Nothing falls back to
# prose: an unevaluable check or a failed tracker read is a named error.
#
#   scripts/director-select-issue.sh
#
# stdout: ISSUE <n>|assertion=<id>|sha=<12>|revision=<n>
#         NO-PROGRESS repeat-<assertion>  (exit 1)
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
state=$source_root/scripts/director-state.sh
repo=${ROBIN_RELEASE_REPO:-DataHoarder6/robin-hood-the-legend-of-sherwood-remake}
handoff=${ROBIN_HANDOFF:-/home/matt/.local/state/robin-director/handoff.json}
defects=${ROBIN_DEFECTS:-$source_root/docs/DEFECTS.md}
gaps=${ROBIN_GAPS:-$source_root/docs/GAPS.md}
standing_goal=${ROBIN_STANDING_GOAL:-49}

die() { printf 'director-select: %s\n' "$1" >&2; exit 1; }

sha=${ROBIN_INTEGRATION_SHA:-}
if [ -z "$sha" ]; then
    sha=$(git ls-remote fork refs/heads/main 2>/dev/null | awk 'NR == 1 { print substr($1, 1, 12) }')
fi
[ -n "$sha" ] || die 'unknown-integration-sha'

# A non-terminal handoff for this tree resumes before any tracker read.
resumed=$(bash "$state" resume "$handoff" "$sha" 2>/dev/null) || true
if [ -n "$resumed" ]; then
    printf '%s\n' "$resumed"
    exit 0
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/robin-select.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

if [ -n "${ROBIN_ISSUES_JSON:-}" ]; then
    issues_json=$(cat "$ROBIN_ISSUES_JSON")
else
    if ! gh issue list --repo "$repo" --state open --label ready-for-agent \
        --json number,title,updatedAt,labels >"$work/issues.json" 2>"$work/gh.err"; then
        cat "$work/gh.err" >&2
        die 'tracker-read-failed'
    fi
    issues_json=$(cat "$work/issues.json")
fi

# HUMAN_GATE items are named in GAPS.md; their issue numbers are in the same
# paragraph, never in a machine-readable field. Read the paragraph, not memory.
human_gate=$(
    grep -n 'HUMAN_GATE' "$gaps" 2>/dev/null | while IFS=: read -r line _; do
        start=$((line - 5)); [ "$start" -lt 1 ] && start=1
        sed -n "${start},${line}p" "$gaps"
    done | grep -oE 'issues/[0-9]+' | cut -d/ -f2 | sort -u || true
)
hg_json=$(printf '%s\n' "$human_gate" | jq -Rsc 'split("\n") | map(select(length > 0) | tonumber)')

issue=$(
    printf '%s\n' "$issues_json" | jq -r --argjson hg "$hg_json" '
        map(select(any(.labels[]?; .name == "ready-for-agent")))
        | map(select(. as $i | ($hg | index($i.number)) == null))
        | sort_by(.updatedAt) | .[0].number // empty
    ' 2>/dev/null
)
[ -n "$issue" ] || die 'no-admitted-issue'

assertion=ISSUE-$issue
repeat=
if [ "$issue" = "$standing_goal" ]; then
    seen=$(jq -c '.seen // {}' "$handoff" 2>/dev/null || printf '{}')
    while IFS='|' read -r id status check; do
        [ -n "$id" ] || continue
        case $status in deferred | out-of-scope) continue ;; esac
        [ -n "$check" ] || die "unknown-assertion-state for $id"
        seen_state=$(printf '%s\n' "$seen" | jq -r --arg a "$id" '.[$a].state // empty')
        seen_sha=$(printf '%s\n' "$seen" | jq -r --arg a "$id" '.[$a].tree_sha // empty')
        if [ "$seen_state" = FILED ] && [ "$seen_sha" = "$sha" ]; then
            repeat=$id
            continue
        fi
        assertion=$id
        break
    done < <(
        awk -F'|' '/^\| D[0-9]+ \|/ {
            gsub(/^ +| +$/, "", $2); gsub(/^ +| +$/, "", $6); gsub(/^ +| +$/, "", $7);
            print $2 "|" $7 "|" $6
        }' "$defects"
    )
    if [ "$assertion" = "ISSUE-$issue" ] && [ -n "$repeat" ]; then
        printf 'NO-PROGRESS repeat-%s\n' "$repeat"
        exit 1
    fi
fi

revision=$(bash "$state" claim "$handoff" "$issue" "$assertion" "$sha")
printf 'ISSUE %s|assertion=%s|sha=%s|revision=%s\n' "$issue" "$assertion" "$sha" "$revision"
