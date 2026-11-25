#!/usr/bin/env bash
# Human-authored, narrowly scoped web-promote entry point. The director may
# execute it; it may not edit it. Credentials and the Caddy roots live on the
# web host, never in this repo.
#
#   ROBIN_WEB_HOST=oci-vm ROBIN_WEB_REPO=/path/to/checkout \
#       bash scripts/release/promote-web.sh vX.Y.Z
set -euo pipefail

tag=${1:?usage: promote-web.sh vX.Y.Z}
host=${ROBIN_WEB_HOST:-}
repo=${ROBIN_WEB_REPO:-}
[ -n "$host" ] || { printf 'promote-web: missing ROBIN_WEB_HOST\n' >&2; exit 1; }
[ -n "$repo" ] || { printf 'promote-web: missing ROBIN_WEB_REPO\n' >&2; exit 1; }

printf 'promote-web: publishing %s from %s on %s\n' "$tag" "$repo" "$host" >&2
ssh "$host" bash -s <<EOF
set -euo pipefail
cd -- '$repo'
bash scripts/deploy_web_jautis.sh '$tag'
EOF
