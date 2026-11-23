#!/usr/bin/env bash
# Print the basename of every worktree of the repository this directory is in.
#
#   scripts/live-worktrees.sh
#
# `scripts/remote-check.sh` names a build copy after its worktree, so this is
# the set of copies that still have an owner. A copy whose worktree is gone is
# a cache nothing can reach: `--prune` deletes exactly those.
set -euo pipefail

# Resolve from this script, not the caller's cwd: remote-check.sh is invoked by
# a systemd unit whose WorkingDirectory is $HOME, where `git rev-parse` failed
# with "not a git repository" and left stale build copies unpruned for days.
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
git -C "$repository" worktree list --porcelain |
    sed -n 's/^worktree //p' |
    while IFS= read -r path; do basename -- "$path"; done |
    grep -v '^gate-' |
    sort -u
