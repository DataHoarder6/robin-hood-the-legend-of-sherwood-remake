#!/usr/bin/env bash
# Verify a landed commit with the suites its landing deferred.
#
#   scripts/post-land-verify.sh SHA SUITE...
#
# land.sh starts this under runbg right after its push is confirmed. The suites
# run on the stable `post-land` copy (warm, incremental, never shared with the
# pre-push gate), from a throwaway checkout of SHA, so the landing worktree can
# be removed at once. A red result is recorded with scripts/main-red.sh, which
# makes every following landing run the full gate until one proves it green
# (docs/adr/0014-two-tier-landing-gate.md).
set -uo pipefail
(( $# >= 2 )) || { echo 'usage: post-land-verify.sh SHA SUITE...' >&2; exit 2; }
sha=$1
shift
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
checkout=$(mktemp -d "${TMPDIR:-/tmp}/robin-post-land.XXXXXX")
cleanup() {
    git -C "$repository" worktree remove --force "$checkout" >/dev/null 2>&1 || true
    rm -rf -- "$checkout"
}
trap cleanup EXIT
rmdir -- "$checkout"
git -C "$repository" worktree add --quiet --detach "$checkout" "$sha"

printf 'post-land: verifying %s with %s\n' "$sha" "$*" >&2
if (cd "$checkout" && ROBIN_GATE_TREE=$(git rev-parse "$sha^{tree}") ROBIN_GATE_COPY=post-land \
        bash scripts/remote-check.sh --suites "$@"); then
    printf 'post-land: %s green\n' "$sha" >&2
else
    bash "$repository/scripts/main-red.sh" set "$sha" "runbg log post-land-${sha:0:12}" "$@"
    printf 'post-land: %s RED; landings now run the full gate (scripts/main-red.sh show)\n' "$sha" >&2
    exit 1
fi
