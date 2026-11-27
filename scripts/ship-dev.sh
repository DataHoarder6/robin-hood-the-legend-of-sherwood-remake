#!/usr/bin/env bash
# Build this worktree's dev game on the build host and copy it to the laptop.
#
#   scripts/ship-dev.sh
#
# Syncs the worktree with `remote-check.sh --sync`, builds `.#robin` there,
# copies the closure through this host (the build host's key does not reach the
# laptop), and prints the store path and the `nix run` line. Nothing is
# installed: dev runs from its own store path, so it sits beside the stable
# profile (docs/PLAY.md).
set -euo pipefail

repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
host=${ROBIN_BUILD_HOST:-minipc}
target=${SHIP_DEV_TARGET:-laptop}

remote_dir=$(bash "$repository/scripts/remote-check.sh" --sync)

printf 'ship-dev: building on %s\n' "$host" >&2
out=$(ssh "$host" bash -s <<EOF
set -euo pipefail
cd $remote_dir
nix build .#robin --print-out-paths
EOF
)

printf 'ship-dev: %s\n' "$(ssh "$host" bash -s <<EOF
set -euo pipefail
cd $remote_dir
./result/bin/robin --version
EOF
)" >&2

nix copy --from "ssh://$host" --to "ssh://$target" "$out"

printf '%s\n' "$out"
printf 'ship-dev: run on %s with `nix run %s`\n' "$target" "$out" >&2
