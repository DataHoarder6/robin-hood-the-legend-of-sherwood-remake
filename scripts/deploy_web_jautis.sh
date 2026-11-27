#!/usr/bin/env bash
# Deploy a tag's browser build to the estate's jautis origins.
#
#   scripts/deploy_web_jautis.sh <v-tag>
#
# Run on oci-vm. Builds ONLY on minipc (the build host); oci-vm only stages and
# swaps static files served by Caddy. The previous trees stay as rollback paths
# and are never deleted by this script. No Cloudflare Worker is touched: the
# jautis origins are Caddy file_server roots (hosts/oci-vm/caddy.nix).
set -euo pipefail

tag=${1:?usage: scripts/deploy_web_jautis.sh <v-tag>}
repo=/home/matt/git/robin-hood-remake
build_host=minipc
public_root=/var/www/robin
signer_root=/var/www/robin-id
public_origin=https://robin.jautis.lt
# The private browser root fetches /campaign.json?build=<short>. The binding is
# operator-managed beside the licensed corpus, never in git or the built site.
# Require an explicit path; a missing operator file is a launch error, not a
# fallback to Demo (D20, #127).
# CAMPAIGN_INPUT_BEGIN
campaign_binding=${CAMPAIGN_BINDING_PATH:?usage: CAMPAIGN_BINDING_PATH=<operator-managed campaign.json> scripts/deploy_web_jautis.sh <v-tag>}
[ -f "$campaign_binding" ] || { echo "deploy_web_jautis: no binding at $campaign_binding" >&2; exit 1; }
# CAMPAIGN_INPUT_END

git -C "$repo" fetch -q fork --tags
commit=$(git -C "$repo" rev-parse "$tag^{commit}")
# An annotated tag names a tag object; the artifact short is the commit it
# points at. Resolving the tag itself bakes the tag object's id instead.
short=$(git -C "$repo" rev-parse --short=12 "$tag^{commit}")
verifier_remote=/home/admin/build/robin-deploy/verify-cloudflare-deployment-$short.mjs
ssh -o BatchMode=yes -o ConnectTimeout=8 "$build_host" mkdir -p /home/admin/build/robin-deploy
scp -q "$PWD/wasm-www/scripts/verify-cloudflare-deployment.mjs" "$build_host:$verifier_remote"
echo "deploy_web_jautis: $tag -> $short ($commit)"

# The runtime binds the Demo identity of the datadir corpus this deploy will
# publish, which is still live under $public_root. `latest.json` on an older
# origin can be a minimal selector with no `multiplayerContent`; in that case
# read the same identity from the newest versioned manifest it points at.
live_manifest=$public_root/wasm/latest.json
[ -f "$live_manifest" ] || { echo "deploy_web_jautis: no live manifest $live_manifest" >&2; exit 1; }
demo=$(jq -c '.multiplayerContent.demo | select(.sha256 and .byteLength and .nativeContentSha256)' "$live_manifest" 2>/dev/null || true)
if [ -z "$demo" ]; then
    while IFS= read -r candidate; do
        demo=$(jq -c '.multiplayerContent.demo | select(.sha256 and .byteLength and .nativeContentSha256)' "$candidate" 2>/dev/null || true)
        [ -n "$demo" ] && break
    done < <(find "$public_root/wasm" -mindepth 2 -maxdepth 2 -name manifest.json | sort -r)
fi
[ -n "$demo" ] || { echo "deploy_web_jautis: no Demo identity in live manifests" >&2; exit 1; }
demo_sha=$(printf '%s\n' "$demo" | jq -r .sha256)
demo_bytes=$(printf '%s\n' "$demo" | jq -r .byteLength)
demo_native_sha=$(printf '%s\n' "$demo" | jq -r .nativeContentSha256)

stage_remote=/home/admin/build/robin-deploy/jautis-deploy-$short
ssh -o BatchMode=yes -o ConnectTimeout=8 "$build_host" bash -s <<REMOTE
set -euo pipefail
tag=$tag
short=$short
verifier_remote=$verifier_remote
stage=$stage_remote
repo=/home/admin/git/robin-hood-remake
wt=/home/admin/build/robin-deploy/jautis-src-$short
toolchain=/home/admin/.local/share/robin_hood/deployment-toolchain
bindgen=/home/admin/build/robin/.tools/wasm-bindgen-0.2.129/bin/wasm-bindgen

git -C "\$repo" fetch -q fork --tags
if [ ! -d "\$wt/.git" ]; then
    # A failed attempt can leave the directory behind without registering the
    # worktree; it is this script's own staging path, so reuse it.
    [ ! -e "\$wt" ] || rm -rf "\$wt"
    # A failed deploy can leave a registered worktree whose directory is gone.
    git -C "\$repo" worktree prune >/dev/null
    git -C "\$repo" worktree add --detach "\$wt" "\$tag" >/dev/null
fi
cd "\$wt"
# The tag's tree predates deploy-time fixes. Use the verifier staged from the
# deploy checkout so the checks match the origin we actually publish.
cp "\$verifier_remote" wasm-www/scripts/verify-cloudflare-deployment.mjs

export PATH="\$toolchain/bin:\$PATH"
export ROBINHOOD_WASM_BINDGEN="\$bindgen"
export ROBIN_OPUS_TOOLS_DIR="\$toolchain/opus-tools-0.2"
# Origin-bound build: the estate signer and game origin, never upstream's.
export VITE_IDENTITY_SIGNER_URL=https://robin-id.jautis.lt/identity-signer/
export VITE_ROBIN_IDENTITY_SIGNER_ORIGIN=https://robin-id.jautis.lt
export ROBIN_SIGNER_ORIGIN=https://robin-id.jautis.lt
export VITE_GAME_ORIGIN=https://robin.jautis.lt
export ROBIN_GAME_ORIGIN=https://robin.jautis.lt
export ROBINHOOD_LEADERBOARD_WEB_ORIGIN=https://robin.jautis.lt
export ROBINHOOD_IDENTITY_SIGNER_ORIGIN=https://robin-id.jautis.lt
# datadir-release.mjs and verify-runtime-corpus.mjs default to upstream's
# origin. The published manifest must name the origin that serves its bytes.
export ROBIN_PUBLIC_ORIGIN=$public_origin
demo_sha=$demo_sha
demo_bytes=$demo_bytes
demo_native_sha=$demo_native_sha

if [ ! -d wasm-www/node_modules ]; then
    pnpm --dir wasm-www install --frozen-lockfile
fi
pnpm --dir wasm-www build:site
pnpm --dir wasm-www verify:public
# Builds the signer shell AND the origin-bound leaderboard bridge, then
# verifies the bridge. A bare build:signer-shell leaves verify:signer without
# the generated bridge files.
bash scripts/build_identity_signer.sh "\$bindgen"
pnpm --dir wasm-www verify:signer

rm -rf "\$stage" "\$stage-signer"
mkdir -p "\$stage-signer"
cp -a wasm-www/dist/. "\$stage/"
cp -a wasm-www/signer-dist/. "\$stage-signer/"

# The Cloudflare path's verified pipeline: one immutable addition bound to the
# live Demo identity, a complete assembled corpus, then a strict verification.
node wasm-www/scripts/datadir-release.mjs current "\$demo_sha" "\$demo_bytes" "\$demo_native_sha" "\$stage/datadir-release.json"
node wasm-www/scripts/stage-runtime-addition.mjs --root "\$stage/runtime-addition" --bindgen "\$bindgen" --datadir-release "\$stage/datadir-release.json"
node wasm-www/scripts/assemble-runtime-corpus.mjs --initial "\$stage/runtime-addition" "\$stage/runtime-dist"
node wasm-www/scripts/verify-runtime-corpus.mjs "\$stage/runtime-dist"
mkdir -p "\$stage/wasm"
cp -a "\$stage/runtime-dist/wasm/." "\$stage/wasm/"

echo "deploy_web_jautis: staged \$short on \$HOSTNAME"
REMOTE

public_new=${public_root}.$short.new
signer_new=${signer_root}.$short.new
# /var/www is root-owned but the published roots are owner-managed. Use the
# operator's scoped NOPASSWD sudo only for the sibling swap paths.
as_root() {
    if [ -w /var/www ]; then "$@"; else sudo -n "$@"; fi
}
as_root rm -rf "$public_new" "$signer_new"
as_root mkdir -p "$public_new" "$signer_new"
as_root chown "$(id -u):$(id -g)" "$public_new" "$signer_new"
scp -q -r "$build_host:$stage_remote/." "$public_new/"
scp -q -r "$build_host:${stage_remote}-signer/." "$signer_new/"
# Publish the operator binding at the exact path the client requests. It is
# validated under HTTP below, before either root is swapped.
# CAMPAIGN_STAGE_BEGIN
# The operator file may be private; the served copy must be Caddy-readable.
# Match an already published file's owner/group rather than guessing one.
[ -f "$public_new/index.html" ] \
    || { echo "deploy_web_jautis: no staged index.html to match campaign.json ownership" >&2; exit 1; }
cp "$campaign_binding" "$public_new/campaign.json"
chown --reference="$public_new/index.html" "$public_new/campaign.json"
chmod 0644 "$public_new/campaign.json"
# CAMPAIGN_STAGE_END

# The existing datadir corpus is retained across deploys; the built runtime's
# Demo sha must match it exactly, or the site would serve a mismatched game.
mkdir -p "$public_new/datadirs"
cp -a "$public_root/datadirs/." "$public_new/datadirs/"
manifest=$public_new/wasm/latest.json
manifest_sha=$(jq -er '.multiplayerContent.demo.sha256' "$manifest")
demo_url=$(jq -er '.multiplayerContent.demo.url' "$manifest")
# The retained corpus names the canonical Demo bytes at demo-local/datadir.bin.
# A new manifest may name the same bytes under a versioned path. Publish an
# alias at that path only when it hashes to the manifest's exact sha; a changed
# content closure must be rebuilt, not aliased.
demo_rel_alias=${demo_url#"$public_origin"/}
case $demo_rel_alias in
    datadirs/*) ;;
    *) demo_rel_alias='' ;;
esac
if [ -n "$demo_rel_alias" ] && [ ! -f "$public_new/$demo_rel_alias" ]; then
    canonical=$public_new/datadirs/demo-local/datadir.bin
    if [ -f "$canonical" ] &&
       [ "$(sha256sum "$canonical" | cut -d' ' -f1)" = "$manifest_sha" ]; then
        mkdir -p "$public_new/$(dirname "$demo_rel_alias")"
        cp -a "$canonical" "$public_new/$demo_rel_alias"
        echo "deploy_web_jautis: aliased canonical Demo datadir to $demo_rel_alias"
    fi
fi
case $demo_url in
    "$public_origin"/*) ;;
    *) echo "deploy_web_jautis: Demo URL $demo_url is not on $public_origin" >&2; exit 1 ;;
esac
demo_rel=${demo_url#"$public_origin"}
case $demo_rel in
    *'?'*|*'#'*|*'..'*)
        echo "deploy_web_jautis: Demo URL $demo_url is not a plain path" >&2; exit 1 ;;
esac
live_sha=$(sha256sum "$public_new$demo_rel" | cut -d' ' -f1)
if [ "$manifest_sha" != "$live_sha" ]; then
    echo "deploy_web_jautis: demo datadir mismatch (manifest $manifest_sha, live $live_sha)" >&2
    echo "deploy_web_jautis: build a new canonical Opus datadir; no swap was done" >&2
    exit 1
fi

# Pre-swap fetch gate. The staged tree is not live yet: serve it on loopback
# and fetch it over HTTP, exactly as the browser will after the swap. Any
# failure exits here, before either root is swapped, leaving the rollback
# trees intact.
if ! command -v python3 >/dev/null 2>&1; then
    echo "deploy_web_jautis: python3 is required for the pre-swap fetch gate" >&2
    exit 1
fi
gate_port_file=$(mktemp)
gate_body=$(mktemp)
gate_headers=$(mktemp)
campaign_parts=$(mktemp)
python3 - "$public_new" "$gate_port_file" <<'PY' &
import http.server
import mimetypes
import sys

mimetypes.add_type("application/wasm", ".wasm")
mimetypes.add_type("application/zstd", ".zst")


class StagedHandler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=sys.argv[1], **kwargs)

    def log_message(self, *_args):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), StagedHandler)
with open(sys.argv[2], "w", encoding="ascii") as port_file:
    port_file.write(str(server.server_address[1]))
server.serve_forever()
PY
gate_pid=$!
trap 'kill "$gate_pid" 2>/dev/null || true; rm -f -- "$gate_port_file" "$gate_body" "$gate_headers" "$campaign_parts"' EXIT
gate_port=
for _ in $(seq 1 50); do
    if [ -s "$gate_port_file" ]; then
        gate_port=$(cat "$gate_port_file")
        break
    fi
    sleep 0.1
done
[ -n "$gate_port" ] || { echo "deploy_web_jautis: staged server did not start" >&2; exit 1; }
gate_base=http://127.0.0.1:$gate_port

gate_status=
gate_local=
gate_get() {
    local path=$1
    gate_local=$public_new$path
    gate_status=$(curl -sS --max-time 30 -o "$gate_body" -D "$gate_headers" \
        -w '%{http_code}' "$gate_base$path") || return 1
    case $gate_status in
        200|304) return 0 ;;
        *) echo "deploy_web_jautis: pre-swap GET $path -> HTTP $gate_status" >&2; return 1 ;;
    esac
}
# A 304 has no body in this loopback fetch; the browser's cache holds the
# staged file itself, which is what we compare against.
gate_verified_sha() {
    if [ "$gate_status" = 304 ]; then
        sha256sum "$gate_local" | cut -d' ' -f1
    else
        sha256sum "$gate_body" | cut -d' ' -f1
    fi
}
gate_same_bytes() {
    [ "$gate_status" = 304 ] && return 0
    cmp -s "$gate_body" "$gate_local"
}
gate_fail() {
    echo "deploy_web_jautis: pre-swap gate: $*" >&2
    exit 1
}

# CAMPAIGN_GATE_BEGIN
# Delimited as one function so the hermetic workflow test executes this exact
# code against generated fixtures: no live deployment and no secret material.
# The browser fetches /campaign.json?build=<short> and resolves the binding's
# relative url against the page origin; validate that same view of the stage.
gate_campaign_binding() {
    local staged_root=$1
    local selector=$2
    local status rel sha bytes count index part

    status=$(curl -sS --max-time 30 -o "$gate_body" -D "$gate_headers" \
        -w '%{http_code}' "$gate_base/campaign.json?build=$selector") \
        || gate_fail "campaign.json fetch failed"
    [ "$status" = 200 ] || gate_fail "campaign.json -> HTTP $status"
    [ -f "$staged_root/campaign.json" ] || gate_fail "campaign.json was not staged"
    cmp -s "$gate_body" "$staged_root/campaign.json" \
        || gate_fail "served campaign.json differs from the staged bytes"

    # Same shape parseHostedReplayContent consumes; extra keys are ignored by
    # the browser, but every field it consumes must be present and well typed.
    # Validate the ORIGINAL JSON strings here: `jq -r` in a command substitution
    # drops trailing newlines, so a shell-only regex would accept a value the
    # browser rejects. No shell extraction happens before this check.
    jq -e '
        type == "object" and
        (.url | type == "string" and index("\n") == null and index("\r") == null
            and test("^/datadirs/full/[0-9a-f]{64}/datadir\\.bin$")) and
        (.sha256 | type == "string" and index("\n") == null and index("\r") == null
            and test("^[0-9a-f]{64}$")) and
        (.byteLength | type == "number" and . == floor and . > 0 and . <= 268435456)
    ' "$gate_body" >/dev/null || gate_fail "campaign.json is not a valid content binding"
    rel=$(jq -er '.url' "$gate_body") || gate_fail "campaign.json has no url"
    sha=$(jq -er '.sha256' "$gate_body") || gate_fail "campaign.json has no sha256"
    bytes=$(jq -er '.byteLength' "$gate_body") || gate_fail "campaign.json has no byteLength"

    # A path-only url is same-origin by construction. Reject schemes, hosts,
    # query strings, fragments and traversal with the client's exact regex.
    [[ $rel =~ ^/datadirs/full/[0-9a-f]{64}/datadir\.bin$ ]] \
        || gate_fail "campaign url $rel is not a same-origin Full datadir path"
    [[ $sha =~ ^[0-9a-f]{64}$ ]] || gate_fail "campaign sha256 is not 64 lowercase hex digits"
    [[ $bytes =~ ^[0-9]+$ ]] || gate_fail "campaign byteLength is not an integer"
    [ "$bytes" -gt 0 ] && [ "$bytes" -le 268435456 ] \
        || gate_fail "campaign byteLength $bytes is outside the client's 256 MiB limit"

    # Derive parts exactly as parseHostedReplayContent does; the binding never
    # names them, so a shifted generation cannot be aliased into this deploy.
    # TODO(#127): these thresholds and the 256 MiB cap duplicate the client's
    # constants; the parity test in static-deployment-workflow.test.mjs fails
    # on drift, but a single shared source would be better.
    if [ "$bytes" -gt 26214400 ]; then
        count=$(( (bytes + 25165823) / 25165824 ))
    else
        count=1
    fi
    : > "$campaign_parts"
    for (( index = 0; index < count; index++ )); do
        if [ "$count" -eq 1 ]; then
            part=$rel
        else
            part=${rel}.part${index}
        fi
        status=$(curl -sS --max-time 60 -o "$gate_body" -D "$gate_headers" \
            -w '%{http_code}' "$gate_base$part") || gate_fail "campaign part $part fetch failed"
        [ "$status" = 200 ] || gate_fail "campaign part $part -> HTTP $status"
        [ -f "$staged_root$part" ] || gate_fail "campaign part $part was not staged"
        cmp -s "$gate_body" "$staged_root$part" \
            || gate_fail "served campaign part $part differs from the staged bytes"
        cat "$gate_body" >> "$campaign_parts"
    done
    [ "$(wc -c < "$campaign_parts")" -eq "$bytes" ] \
        || gate_fail "campaign parts do not total the declared byteLength"
    [ "$(sha256sum "$campaign_parts" | cut -d' ' -f1)" = "$sha" ] \
        || gate_fail "campaign parts do not match the declared sha256"
}
# CAMPAIGN_GATE_END

gate_get /wasm/latest.json || gate_fail "latest.json fetch failed"
gate_same_bytes || gate_fail "served latest.json differs from the staged bytes"
jq -e --arg short "$short" '.short == $short' "$gate_body" >/dev/null \
    || gate_fail "latest.json does not select $short"
# Use the selector latest.json actually publishes, exactly as the client does,
# and refuse to gate a build other than the one this deploy is swapping in.
campaign_selector=$(jq -er '.short' "$gate_body") \
    || gate_fail "latest.json has no build selector"
[ "$campaign_selector" = "$short" ] \
    || gate_fail "build selector $campaign_selector is not $short"
gate_campaign_binding "$public_new" "$campaign_selector"

manifest_file=$public_new/wasm/$short/manifest.json
gate_get "/wasm/$short/manifest.json" || gate_fail "manifest fetch failed"
gate_same_bytes || gate_fail "served manifest differs from the staged bytes"
jq -e --arg commit "$commit" --arg short "$short" \
    '.commit == $commit and .short == $short' "$gate_body" >/dev/null \
    || gate_fail "manifest identity is not $short ($commit)"

wasm_name=$(jq -er '.files.wasm' "$manifest_file")
br_name=$(jq -er '.files.wasmBrotli' "$manifest_file")
js_name=$(jq -er '.files.js' "$manifest_file")
wasm_sha=$(jq -er '.sha256.wasm' "$manifest_file")
br_sha=$(jq -er '.sha256.wasmBrotli' "$manifest_file")

gate_get "/wasm/$short/$js_name" || gate_fail "robin.js fetch failed"
gate_get "/wasm/$short/$wasm_name" || gate_fail "wasm fetch failed"
grep -qi '^Content-Type:[[:space:]]*application/wasm' "$gate_headers" \
    || gate_fail "wasm is not served as application/wasm"
[ "$(gate_verified_sha)" = "$wasm_sha" ] \
    || gate_fail "served wasm sha256 does not match the manifest"
gate_get "/wasm/$short/$br_name" || gate_fail "Brotli sibling fetch failed"
[ "$(gate_verified_sha)" = "$br_sha" ] \
    || gate_fail "served Brotli wasm sha256 does not match the manifest"
gate_get "$demo_rel" || gate_fail "Demo datadir fetch failed"
[ "$(gate_verified_sha)" = "$manifest_sha" ] \
    || gate_fail "served Demo datadir sha256 does not match the manifest"
echo "deploy_web_jautis: pre-swap fetch gate passed for $short"

# Atomic swap, one root at a time; each prior tree is kept for rollback.
[ ! -e "$public_root.prev" ] || as_root rm -rf "$public_root.prev"
[ ! -e "$signer_root.prev" ] || as_root rm -rf "$signer_root.prev"
as_root mv "$public_root" "$public_root.prev"
as_root mv "$public_new" "$public_root"
as_root mv "$signer_root" "$signer_root.prev"
as_root mv "$signer_new" "$signer_root"

echo "deploy_web_jautis: $short live at $public_root and $signer_root"
echo "deploy_web_jautis: rollback: mv $public_root.prev $public_root; mv $signer_root.prev $signer_root"
