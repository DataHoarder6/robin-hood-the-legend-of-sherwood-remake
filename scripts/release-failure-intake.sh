#!/usr/bin/env bash
# Turn a refused release-evidence verdict into one admitted tracker item.
# This is the estate's version of GitHub's `safe-outputs: create-issue`: the
# only write is a labelled issue with named evidence, and duplicates are
# refused by an explicit search before creation.
#
#   scripts/release-failure-intake.sh <reason> <candidate-short> [version]
set -uo pipefail

reason=${1:?usage: release-failure-intake.sh <reason> <candidate-short> [version]}
candidate=${2:?missing candidate short}
version=${3:-0.5.0}
repo=${ROBIN_RELEASE_REPO:-DataHoarder6/robin-hood-the-legend-of-sherwood-remake}

command -v gh >/dev/null || { printf 'intake: missing-gh\n' >&2; exit 1; }

fingerprint="release-evidence refusal: $reason"
if existing=$(gh issue list --repo "$repo" --state open \
    --search "$fingerprint in:title" --limit 1 --json number \
    --jq '.[0].number // ""' 2>/dev/null) && [ -n "$existing" ]; then
    printf 'intake: already-filed #%s\n' "$existing"
    exit 0
fi

body=$(printf '%s\n\n- reason: `%s`\n- candidate: `%s`\n- version: %s\n- written: %s\n\nRelease evidence refused; auto-release was not reached. The director owns the fix. Do not weaken the release gate to make it pass.\n' \
    "Release evidence refused for $version at $candidate" \
    "$reason" "$candidate" "$version" "$(date -u +%FT%TZ)")
url=$(gh issue create --repo "$repo" \
    --title "$fingerprint ($candidate)" \
    --label bug --label ready-for-agent \
    --body "$body" 2>&1) || { printf 'intake: create-failed: %s\n' "$url" >&2; exit 1; }
printf 'intake: filed %s\n' "$url"
