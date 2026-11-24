#!/usr/bin/env bash
# Blocking release parity gate: one tag must name both the native release and
# the browser artifact served by the estate. Run on oci-vm after both paths
# have published. Read-only; no publish, no deploy, no tag move.
#
#   scripts/release-parity-check.sh <v-tag>
set -uo pipefail

tag=${1:?usage: scripts/release-parity-check.sh <v-tag>}
repo=${ROBIN_REPO:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)}
cd "$repo"

fail=0
ok()   { printf 'ok:   %s\n' "$*"; }
bad()  { printf 'FAIL: %s\n' "$*" >&2; fail=1; }

git fetch -q fork --tags
commit=$(git rev-parse "$tag^{commit}" 2>/dev/null) || { bad "no such tag on fork: $tag"; exit 1; }
short=$(git rev-parse --short=12 "$tag^{commit}")
version=${tag#v}
expected_assets=(
    releases.win.json releases.linux.json
    robinhood-remake-windows-Setup.exe robinhood-remake-windows-Portable.zip
    "robinhood-remake-$version-windows-full.nupkg"
    robinhood-remake-linux.AppImage
    "robinhood-remake-$version-linux-full.nupkg"
)

# Native release: asset inventory, not the job status.
release=$(gh release view "$tag" \
    -R DataHoarder6/robin-hood-the-legend-of-sherwood-remake \
    --json assets --jq '.assets[].name' 2>/dev/null) || { bad "no GitHub release for $tag"; release=""; }
for asset in "${expected_assets[@]}"; do
    printf '%s\n' "$release" | grep -qx "$asset" \
        && ok "native asset $asset" \
        || bad "native release is missing $asset"
done

# Browser artifact: the served manifest must name the same commit.
site=${ROBIN_SITE_ROOT:-/var/www/robin}
manifest=$site/wasm/$short/manifest.json
[ -f "$manifest" ] || { bad "site has no wasm/$short/manifest.json"; exit 1; }
read_commit=$(jq -er .commit "$manifest" 2>/dev/null)
read_short=$(jq -er .short "$manifest" 2>/dev/null)
schema=$(jq -er .ticketSchema "$manifest" 2>/dev/null)
[ "$read_commit" = "$commit" ] && ok "site manifest commit $commit" \
    || bad "site manifest commit $read_commit != $commit"
[ "$read_short" = "$short" ] && ok "site manifest short $short" \
    || bad "site manifest short $read_short != $short"
# The boot gate owns the ticket-schema verdict; the parity gate must not
# hard-code a version that moves with the runtime (browser-boot.mjs:49).
if [ -n "$schema" ] && [ "$schema" -gt 0 ] 2>/dev/null; then
    ok "site manifest ticketSchema $schema (boot gate owns its verdict)"
else
    bad "site manifest has no positive ticketSchema: ${schema:-none}"
fi

demo_url=$(jq -er .multiplayerContent.demo.url "$manifest" 2>/dev/null)
demo_sha=$(jq -er .multiplayerContent.demo.sha256 "$manifest" 2>/dev/null)
# The site root is a local path; the manifest pins an absolute URL. Parse the
# URL against the site's origin instead of stripping a literal origin string,
# so a wrong-origin or malformed URL fails the gate rather than resolving to a
# misleading local path.
site_origin=https://robin.jautis.lt
site_path() {
    local url=$1 path
    [[ $url == "$site_origin"/* ]] || return 1
    path=${url#"$site_origin"}
    [[ $path == /* && $path != *'?'* && $path != *'#'* && $path != *'..'* ]] || return 1
    printf '%s%s\n' "$site" "$path"
}
demo_path=$(site_path "$demo_url") || bad "Demo URL $demo_url is not a path on $site_origin"
if [ -n "$demo_path" ] && [ -f "$demo_path" ]; then
    live_sha=$(sha256sum "$demo_path" | cut -d' ' -f1)
    [ "$live_sha" = "$demo_sha" ] && ok 'served Demo datadir SHA matches manifest' \
        || bad "served Demo datadir SHA $live_sha != manifest $demo_sha"
else
    bad "served Demo datadir not found at $demo_path"
fi

latest=$(jq -er .short "$site/wasm/latest.json" 2>/dev/null)
[ "$latest" = "$short" ] && ok "site latest.json selects $short" \
    || bad "site latest.json selects $latest, expected $short"
# A minimal {commit,short} document can name the right checkout while serving
# no multiplayer content. The site must publish the exact manifest bytes.
if cmp -s "$site/wasm/latest.json" "$manifest"; then
    ok 'site latest.json is byte-identical to the versioned manifest'
else
    bad 'site latest.json bytes differ from the versioned manifest'
fi
# The selector is byte-for-byte the versioned manifest, not a rewritten copy:
# the browser fetches both and the boot gate compares their identity.
cmp -s "$site/wasm/latest.json" "$manifest" \
    && ok 'site latest.json bytes equal the versioned manifest' \
    || bad 'site latest.json bytes differ from the versioned manifest'

# Seat version: check the local install when this host has one.
if command -v robin >/dev/null 2>&1; then
    version=$(robin --version 2>/dev/null | grep -oE '[0-9a-f]{12}' | head -1)
    [ "$version" = "$short" ] && ok "local seat version $short" \
        || bad "local seat version ${version:-none} != $short"
else
    printf 'skip: no local robin binary; check each seat with robin --version\n'
fi

(( fail == 0 )) || { printf 'release-parity: FAIL\n' >&2; exit 1; }
printf 'release-parity: OK (%s, %s)\n' "$tag" "$short"
