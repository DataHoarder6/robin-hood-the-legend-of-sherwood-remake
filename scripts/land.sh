#!/usr/bin/env bash
# Land this worktree's HEAD on the fork's integration branch.
#
#   scripts/land.sh     # fetch, rebase if behind, gate the tree that lands, push
#
# A round is: fetch the integration tip; rebase onto it when HEAD is not already
# ahead of it; gate the tree the push will land; push; read the remote back. The
# gate proves a tree, and a rebase rewrites the commits and changes what they sit
# on, so the rebase is followed by another gate rather than by a push. If the
# remote moves between the gate and the push, the push is refused and the round
# repeats from a fresh fetch, three rounds at most. Nothing here force-pushes, and
# a rebase conflict stops the run with the paths listed and the branch untouched.
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
rounds=3

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

# FETCH_HEAD is one file for the whole repository, and every worktree fetches, so
# a read after a fetch can return someone else's ref. A ref named after this
# worktree is written only here. It is not FETCH_HEAD's replacement for a human,
# only for the one read below, and it is removed on exit.
landing_ref="refs/robin/land/$(printf '%s' "$repository" | git hash-object --stdin)"
trap 'git update-ref -d "$landing_ref" 2>/dev/null || true' EXIT

fetch_integration() {
    git fetch --quiet --no-tags "$remote" "+refs/heads/$integration:$landing_ref" >/dev/null
    git rev-parse --verify "$landing_ref^{commit}"
}

# Move HEAD onto BASE, or stop with the branch exactly as it was. `head` is the
# commit the push would carry, so it is re-read after a successful rebase.
integrate() {
    local base=$1 conflicts
    if git rebase --quiet "$base"; then
        head=$(git rev-parse --verify HEAD^{commit})
        printf 'land: rebased onto %s; the gate now proves %s\n' "$base" "$head" >&2
        return 0
    fi
    conflicts=$(git diff --name-only --diff-filter=U || true)
    if [ -d "$(git rev-parse --git-path rebase-merge)" ] ||
        [ -d "$(git rev-parse --git-path rebase-apply)" ]; then
        git rebase --abort
    fi
    printf 'land: rebasing %s onto %s failed; the branch and %s/%s are unchanged.\n' \
        "$head" "$base" "$remote" "$integration" >&2
    if [ -n "$conflicts" ]; then
        printf 'land: resolve these paths, then land again:\n' >&2
        printf '%s\n' "$conflicts" | sed 's/^/land:   /' >&2
    else
        printf 'land: git refused the rebase; its reason is above. A dirty worktree\n' >&2
        printf 'land: cannot be rebased, and the gate requires a clean one.\n' >&2
    fi
    exit 1
}

for _round in $(seq 1 "$rounds"); do
    base=$(fetch_integration)

    if git merge-base --is-ancestor "$head" "$base"; then
        printf 'land: nothing to do; %s/%s already contains %s\n' "$remote" "$integration" "$head" >&2
        exit 0
    fi

    rebased=0
    if ! git merge-base --is-ancestor "$base" "$head"; then
        printf 'land: %s/%s is at %s; HEAD %s is not built on it\n' \
            "$remote" "$integration" "$base" "$head" >&2
        integrate "$base"
        rebased=1
    fi

    # The first round always gates; a later round gates only because it rebased,
    # since that is the only thing that changes the tree or its base.
    if [ "$rebased" = 1 ] || [ "$_round" = 1 ]; then
        bash scripts/push-gate.sh "$base" "$head"
    fi

    if ! SKIP_PUSH_GATE=1 git push --quiet "$remote" "HEAD:refs/heads/$integration"; then
        printf 'land: %s/%s refused the push; fetching and integrating again\n' \
            "$remote" "$integration" >&2
        continue
    fi

    landed=$(git ls-remote "$remote" "refs/heads/$integration" | cut -f1)
    if [ "$landed" = "$head" ]; then
        printf 'land: %s/%s = %s\n' "$remote" "$integration" "$landed" >&2
        exit 0
    fi
    printf 'land: %s/%s is %s after the push, not %s; integrating again\n' \
        "$remote" "$integration" "$landed" "$head" >&2
done

printf 'land: %s/%s did not reach %s after %s rounds\n' \
    "$remote" "$integration" "$head" "$rounds" >&2
exit 1
