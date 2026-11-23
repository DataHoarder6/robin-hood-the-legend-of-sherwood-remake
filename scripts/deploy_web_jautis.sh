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

git -C "$repo" fetch -q fork --tags
commit=$(git -C "$repo" rev-parse "$tag^{commit}")
short=$(git -C "$repo" rev-parse --short=12 "$tag")
echo "deploy_web_jautis: $tag -> $short ($commit)"

stage_remote=/home/admin/build/robin/jautis-deploy-$short
ssh -o BatchMode=yes -o ConnectTimeout=8 "$build_host" bash -s <<REMOTE
set -euo pipefail
tag=$tag
short=$short
stage=$stage_remote
repo=/home/admin/git/robin-hood-remake
wt=/home/admin/build/robin/jautis-src-$short
toolchain=/home/admin/.local/share/robin_hood/deployment-toolchain
bindgen=/home/admin/build/robin/.tools/wasm-bindgen-0.2.129/bin/wasm-bindgen

git -C "\$repo" fetch -q fork --tags
if [ ! -d "\$wt/.git" ]; then
    git -C "\$repo" worktree add --detach "\$wt" "\$tag" >/dev/null
fi
cd "\$wt"

export PATH="\$toolchain/bin:\$PATH"
export ROBINHOOD_WASM_BINDGEN="\$bindgen"
export ROBIN_OPUS_TOOLS_DIR="\$toolchain/opus-tools-0.2"
# Origin-bound build: the estate signer and game origin, never upstream's.
export VITE_IDENTITY_SIGNER_URL=https://robin-id.jautis.lt/identity-signer/
export VITE_ROBIN_IDENTITY_SIGNER_ORIGIN=https://robin-id.jautis.lt
export ROBIN_SIGNER_ORIGIN=https://robin-id.jautis.lt
export VITE_GAME_ORIGIN=https://robin.jautis.lt
export ROBINHOOD_LEADERBOARD_WEB_ORIGIN=https://robin.jautis.lt
export ROBINHOOD_IDENTITY_SIGNER_ORIGIN=https://robin-id.jautis.lt

if [ ! -d wasm-www/node_modules ]; then
    pnpm --dir wasm-www install --frozen-lockfile
fi
pnpm --dir wasm-www build:site
pnpm --dir wasm-www verify:public
pnpm --dir wasm-www build:signer-shell
pnpm --dir wasm-www verify:signer
pnpm --dir wasm-www verify:signer-bridge

rm -rf "\$stage" "\$stage-signer"
mkdir -p "\$stage/wasm/\$short" "\$stage-signer"
cp -a wasm-www/dist/. "\$stage/"
cp -a wasm-www/signer-dist/. "\$stage-signer/"

node wasm-www/scripts/build-runtime.mjs --threads --bindgen "\$bindgen" --out-dir "\$stage/wasm/\$short"
node wasm-www/scripts/stage-engine-preload-assets.mjs assets/core-datadir "\$stage/wasm/\$short"
node wasm-www/scripts/runtime-javascript-modules.mjs "\$stage/wasm/\$short" > "\$stage/js-modules.json"
node wasm-www/scripts/verify-runtime-multiplayer.mjs --pkg "\$stage/wasm/\$short" --expect-ticket-schema 3
cp "\$stage/wasm/\$short/manifest.json" "\$stage/wasm/latest.json"

echo "deploy_web_jautis: staged \$short on \$HOSTNAME"
REMOTE

public_new=${public_root}.$short.new
signer_new=${signer_root}.$short.new
rm -rf "$public_new" "$signer_new"
mkdir -p "$public_new" "$signer_new"
scp -q -r "$build_host:$stage_remote/." "$public_new/"
scp -q -r "$build_host:${stage_remote}-signer/." "$signer_new/"

# The existing datadir corpus is retained across deploys; the built runtime's
# Demo sha must match it exactly, or the site would serve a mismatched game.
cp -a "$public_root/datadirs" "$public_new/datadirs"
manifest_sha=$(jq -er '.multiplayerContent.demo.sha256' "$public_new/wasm/latest.json")
live_sha=$(sha256sum "$public_new/datadirs/demo-local/datadir.bin" | cut -d' ' -f1)
if [ "$manifest_sha" != "$live_sha" ]; then
    echo "deploy_web_jautis: demo datadir mismatch (manifest $manifest_sha, live $live_sha)" >&2
    echo "deploy_web_jautis: build a new canonical Opus datadir; no swap was done" >&2
    exit 1
fi

# Atomic swap, one root at a time; each prior tree is kept for rollback.
[ -e "$public_root.prev" ] && rm -rf "$public_root.prev"
[ -e "$signer_root.prev" ] && rm -rf "$signer_root.prev"
mv "$public_root" "$public_root.prev"
mv "$public_new" "$public_root"
mv "$signer_root" "$signer_root.prev"
mv "$signer_new" "$signer_root"

echo "deploy_web_jautis: $short live at $public_root and $signer_root"
echo "deploy_web_jautis: rollback: mv $public_root.prev $public_root; mv $signer_root.prev $signer_root"
