#!/usr/bin/env bash
# Print the build-copy name for a repository or a pushed tree.
#
#   gate-copy-name.sh REPOSITORY [TREE_SHA]
#
# Gates all use ONE stable copy: a tree-keyed copy gets a fresh target/ for
# every landing, so Cargo recompiles every dependency (measured sccache Rust hit
# rate 0.36%). Interactive checks use the warm pool (`pool`, resolved by
# remote-check.sh to the first free `pool-N`): a copy per worktree name started
# cold for every new worktree (60 and 16 minute builds, 2026-11-21). The copy
# lock is held from rsync through the suites, so no copy swaps trees under a
# running build, and Cargo's own freshness check reuses unchanged artifacts.
set -euo pipefail

(($# >= 1)) || exit 2

if (($# >= 2)) && [[ -n $2 ]]; then
    printf 'gate-shared\n'
else
    printf 'pool\n'
fi
