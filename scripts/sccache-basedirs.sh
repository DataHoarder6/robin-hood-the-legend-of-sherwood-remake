#!/usr/bin/env bash
# Print the colon-separated absolute paths of every existing build copy root,
# or with --remap the per-root rustc remap flags for those roots.
#
# sccache hashes absolute paths unless basedirs are normalized; each copy must
# be listed, not just their shared parent, for a hit to cross copies. A change
# to the set is a change to the daemon's configuration, so restart it once.
set -euo pipefail

roots=()
for dir in "$HOME"/build/robin/*/; do
    [[ -d $dir ]] || continue
    roots+=("${dir%/}")
done
list=$(IFS=:; printf '%s' "${roots[*]-}")

state_dir="$HOME/.cache"
mkdir -p -- "$state_dir"
state="$state_dir/sccache-basedirs.set"
if [ "$(cat "$state" 2>/dev/null || true)" != "$list" ]; then
    printf '%s' "$list" > "$state"
    sccache --stop-server >/dev/null 2>&1 || true
fi

if [[ ${1:-} == --remap ]]; then
    flags=()
    for root in "${roots[@]}"; do
        flags+=("--remap-path-prefix=$root=/robin")
    done
    (IFS=' '; printf '%s\n' "${flags[*]-}")
else
    printf '%s\n' "$list"
fi
