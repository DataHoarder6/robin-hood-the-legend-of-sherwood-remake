#!/usr/bin/env bash
# Print the build-copy name for a repository or a pushed tree.
#
#   gate-copy-name.sh REPOSITORY [TREE_SHA]
#
# Interactive checks keep the worktree name so `live-worktrees.sh` and --prune
# can still find their owner. A push gate supplies the pushed tree's sha; every
# push of that tree then shares one copy, one lock and one build cache.
set -euo pipefail

(($# >= 1)) || exit 2
repository=$1
tree_sha=${2:-}

if [[ -n $tree_sha ]]; then
    printf 'gate-%s\n' "${tree_sha:0:12}"
else
    basename "$repository"
fi
