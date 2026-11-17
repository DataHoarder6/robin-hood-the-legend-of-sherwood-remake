#!/usr/bin/env bash
# Move this checkout's `main` to the integration ref, keeping the old tip.
#
# `main` here is a read mirror: work happens in `git worktree add`. When main
# has drifted, the old tip becomes a branch under `archive/`, so nothing is
# lost, and main fast-forwards to the integration ref.
#
# Refuses when the tree is dirty, when a build is running in this checkout, when
# this is not `main`, and when main holds commits the integration ref does not
# have (land those first; this recipe never drops work).
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/lib/preflight.sh
source "$script_dir/lib/preflight.sh"

refuse() {
    printf 'reconcile-main: %s\n' "$*" >&2
    exit 1
}

root=$(preflight_root .) || refuse 'not inside a git work tree'
cd -- "$root"

branch=$(git symbolic-ref --short -q HEAD) || branch=
[ "$branch" = main ] || refuse "this checkout is on '${branch:-a detached HEAD}', not main"

if preflight_dirty "$root"; then
    printf 'reconcile-main: the tree has uncommitted changes; they are someone'\''s work and moving main would move them.\n' >&2
    printf '  Run: just doctor\n' >&2
    exit 1
fi

if builds=$(preflight_build_processes "$root") && [ -n "$builds" ]; then
    printf 'reconcile-main: a build is running inside this checkout (%s).\n' \
        "$(printf '%s\n' "$builds" | awk '{printf "%s%s[%s]", separator, $2, $1; separator = ", "}')" >&2
    printf '  Wait for it, or stop it, then retry; a build outlives the branch it started on.\n' >&2
    exit 1
fi

upstream=$(preflight_integration_ref "$root")
[ -n "$upstream" ] || refuse 'no integration ref (fork/main, origin/main, or this branch upstream); nothing to reconcile against'

head=$(git rev-parse HEAD) || refuse 'main has no commits yet'
target=$(git rev-parse "$upstream")

remote=${upstream%%/*}
if git remote get-url "$remote" >/dev/null 2>&1; then
    git fetch --quiet "$remote" || refuse "git fetch $remote failed; the refs below are local only"
    target=$(git rev-parse "$upstream")
fi

if [ "$head" = "$target" ]; then
    printf 'reconcile-main: main already matches %s at %s\n' "$upstream" "${head:0:8}"
    exit 0
fi

read -r unique duplicate < <(preflight_cherry "$root" "$upstream" HEAD)
if (( unique > 0 )); then
    printf 'reconcile-main: %s holds %d commit(s) %s does not have (%d already there as duplicate patches):\n' \
        "${head:0:8}" "$unique" "$upstream" "$duplicate" >&2
    git -C "$root" log --oneline --reverse "$upstream..HEAD" >&2
    printf '  Land them first (git cherry-pick -x <sha> in a worktree), or archive knowingly; this recipe does not drop work.\n' >&2
    exit 1
fi

archive="archive/main-$(date +%Y-%m-%d)-${head:0:8}"
if git show-ref --verify --quiet "refs/heads/$archive"; then
    if [ "$(git rev-parse "$archive")" != "$head" ]; then
        refuse "$archive exists at another commit; resolve it by hand"
    fi
    printf 'reconcile-main: old tip %s is already archived as %s\n' "${head:0:8}" "$archive"
else
    git branch "$archive" "$head"
    printf 'reconcile-main: archived old tip %s as %s\n' "${head:0:8}" "$archive"
fi

git reset --hard --quiet "$target"
printf 'reconcile-main: main is now %s (%s); %d duplicate commit(s) skipped\n' \
    "${target:0:8}" "$upstream" "$duplicate"
printf '  Run just doctor to confirm.\n'
