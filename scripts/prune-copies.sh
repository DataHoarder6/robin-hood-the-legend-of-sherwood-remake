#!/usr/bin/env bash
# Remove build copies with no owner, and gate copies past their retention.
#
#   scripts/prune-copies.sh ROOT TTL_DAYS [KEEP_NAME ...]
#
# Runs on the build host. Interactive copies are kept when their worktree is
# still live (KEEP_NAME); gate copies have no worktree, so only their age keeps
# them. Same rule, one place, so the TTL branch has a behavioural fixture.
set -euo pipefail

(($# >= 2)) || exit 2
root=$1
ttl=$2
shift 2
keep=("$@")

cd -- "$root" 2>/dev/null || exit 0
for d in */; do
    d=${d%/}
    [[ -f $d/Cargo.toml ]] || continue
    owned=0
    for name in "${keep[@]}"; do
        [[ $d == "$name" ]] && owned=1 && break
    done
    (( owned == 1 )) && continue
    case $d in
        gate-*)
            [[ -n $(find "$d" -maxdepth 0 -mtime +"$ttl") ]] || continue
            ;;
    esac
    printf 'pruned %s\n' "$d" >&2
    rm -rf -- "$d" "$d.lock"
done
