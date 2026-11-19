#!/usr/bin/env bash
# Land this worktree's HEAD on the fork's integration branch.
#
#   scripts/land.sh            # gate the diff since fork/main, then push HEAD
#
# The gate is `scripts/push-gate.sh`, the same one `.githooks/pre-push` runs, so
# there is one definition of which suites a landing owes. This command runs it
# *before* opening the connection to the remote: git starts its receive-pack
# conversation before the pre-push hook, and a seven-minute gate holds that
# conversation idle long enough that the push dies with SIGPIPE after the suites
# pass (measured 2026-11-19 on every long gated push). A green gate is the
# precondition here, and the hook stays in place for a hand-run `git push`.
#
# It prints the remote tip it landed and refuses to claim success until the
# remote reports the commit: a push's exit status is not evidence.
set -euo pipefail

exec </dev/null

repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$repository"

remote=${ROBIN_LAND_REMOTE:-fork}
integration=main

usage() {
    awk 'NR > 1 && /^[^#]/{exit} NR > 1{sub(/^# ?/, ""); print}' "${BASH_SOURCE[0]}" >&2
    exit 2
}

case "${1:-}" in
    -h | --help) usage ;;
    "")
        ;;
    *)
        usage
        ;;
esac

head=$(git rev-parse --verify HEAD^{commit})
base=$(git ls-remote "$remote" "refs/heads/$integration" | cut -f1)
if [ -n "$base" ] && git merge-base --is-ancestor "$base" "$head"; then
    printf 'land: nothing to do; %s already contains %s\n' "$remote/$integration" "$head" >&2
    exit 0
fi

bash scripts/push-gate.sh "$base" "$head"

SKIP_PUSH_GATE=1 git push --quiet "$remote" "HEAD:refs/heads/$integration"

landed=$(git ls-remote "$remote" "refs/heads/$integration" | cut -f1)
if [ "$landed" != "$head" ]; then
    printf 'land: %s/%s is %s after the push, not %s\n' "$remote" "$integration" "$landed" "$head" >&2
    exit 1
fi
printf 'land: %s/%s = %s\n' "$remote" "$integration" "$landed" >&2
