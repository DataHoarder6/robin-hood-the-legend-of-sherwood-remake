#!/usr/bin/env bash
# Print the build-copy name for a repository or a pushed tree.
#
#   gate-copy-name.sh REPOSITORY [TREE_SHA]
#
# Interactive checks keep the worktree name so `live-worktrees.sh` and --prune
# can still find their owner. Gates all use ONE stable copy: a tree-keyed copy
# gets a fresh target/ for every landing, so Cargo recompiles every dependency
# (measured sccache Rust hit rate 0.36%). The copy lock is held from rsync
# through the suites, so a single copy cannot swap trees under a running build.
# Cargo's own freshness check then reuses unchanged artifacts across landings.
set -euo pipefail

(($# >= 1)) || exit 2
repository=$1

if (($# >= 2)) && [[ -n $2 ]]; then
    printf 'gate-shared\n'
else
    basename "$repository"
fi
