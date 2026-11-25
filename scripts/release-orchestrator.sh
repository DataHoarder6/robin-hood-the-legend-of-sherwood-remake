#!/usr/bin/env bash
# Idempotent release tail: verify notes -> tag once -> dispatch -> read back
# native assets -> promote web -> blocking parity -> record RELEASED. Nothing is
# recorded before every phase passes; no tag is ever moved or deleted.
#
#   scripts/release-orchestrator.sh <vX.Y.Z>
#
# The director may execute this script; it may not edit it. All authority comes
# from docs/AUTONOMOUS_RUN_CONTRACT.md and ADR 0011.
set -euo pipefail

tag=${1:?usage: release-orchestrator.sh vX.Y.Z}
repo=${ROBIN_REPO_DIR:-$(pwd)}
git_bin=${ROBIN_GIT:-git}
gh_bin=${ROBIN_GH:-gh}
curl_bin=${ROBIN_CURL:-curl}
state=${ROBIN_RELEASE_STATE:-$repo/.release-state.json}
polls=${ROBIN_RELEASE_POLL:-5}
promote=${ROBIN_PROMOTE:-$repo/scripts/release/promote-web.sh}
parity=${ROBIN_PARITY:-$repo/scripts/release-parity-check.sh}
api=https://api.github.com/repos/DataHoarder6/robin-hood-the-legend-of-sherwood-remake

fail() { printf 'release-orchestrator: %s\n' "$1" >&2; exit 1; }
cd -- "$repo"

grep -q "^## $tag\$" docs/RELEASE_NOTES.md || fail 'notes-missing'
[ -f docs/releases/whats-new.json ] || fail 'manifest-missing'
jq -e --arg tag "$tag" '.[$tag]' docs/releases/whats-new.json >/dev/null 2>&1 || fail 'manifest-entry-missing'
"$git_bin" show-ref --verify --quiet "refs/tags/$tag" || fail 'local-tag-missing'

# A tag already on the integration ref is a resume, not a reason to re-tag.
if ! "$git_bin" ls-remote fork "refs/tags/$tag" 2>/dev/null | grep -q "refs/tags/$tag"; then
    "$git_bin" push fork "$tag" || fail 'tag-push-failed'
fi
"$gh_bin" workflow run native-release.yml --ref "$tag" || fail 'dispatch-failed'

body=
for (( attempt = 0; attempt <= polls; attempt++ )); do
    body=$("$curl_bin" -fsS "$api/releases/tags/$tag" 2>/dev/null) || body=
    names=$(printf '%s\n' "$body" | jq -r '.assets[].name' 2>/dev/null || true)
    if printf '%s\n' "$names" | grep -q '^robinhood-remake-linux.AppImage$' &&
       printf '%s\n' "$names" | grep -q '^robinhood-remake-windows-Setup.exe$'; then
        break
    fi
    (( attempt < polls )) && sleep 5
done
assets=$(printf '%s\n' "$body" | jq -c '[.assets[] | {name, digest}]' 2>/dev/null || true)
[ -n "$assets" ] && [ "$assets" != '[]' ] || fail 'native-assets-missing'

bash "$promote" "$tag" || fail 'web-promote-failed'
bash "$parity" "$tag" || fail 'parity-failed'

jq -n --arg tag "$tag" --argjson assets "$assets" --arg at "$(date -u +%FT%TZ)" \
    '{tag: $tag, outcome: "RELEASED", assets: $assets, at: $at}' >"$state"
printf 'release-orchestrator: RELEASED %s\n' "$tag"
