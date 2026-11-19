#!/usr/bin/env bash
# Read-only preflight for a checkout shared between agents.
#
#   scripts/doctor.sh              # report on the work tree containing $PWD
#   scripts/doctor.sh --quiet      # only findings that need action
#   scripts/doctor.sh PATH         # report on another checkout
#
# Exit status is non-zero only for states that will bite: uncommitted work on
# the shared `main`, `main` drifted from the integration ref, a build running
# inside this checkout, an archived or detached tip holding commits the
# integration ref does not have, or too little free disk. Sizes are reported,
# never fatal. Nothing here writes to the repository.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/lib/preflight.sh
source "$script_dir/lib/preflight.sh"

quiet=0
target=.
while (( $# > 0 )); do
    case $1 in
        --quiet) quiet=1 ;;
        -h | --help)
            sed -n '2,13p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
            exit 0
            ;;
        --*)
            printf 'doctor: unknown option %s\n' "$1" >&2
            exit 2
            ;;
        *)
            [ "$target" = . ] || {
                printf 'doctor: one directory at most\n' >&2
                exit 2
            }
            target=$1
            ;;
    esac
    shift
done

root=$(preflight_root "$target") || {
    printf 'doctor: %s is not inside a git work tree\n' "$target" >&2
    exit 2
}

min_free_gib=${DOCTOR_MIN_FREE_GIB:-15}
case $min_free_gib in
    '' | *[!0-9]*)
        printf 'doctor: DOCTOR_MIN_FREE_GIB must be a whole number of GiB\n' >&2
        exit 2
        ;;
esac

problems=0
fail() { # ID MESSAGE FIX
    problems=$((problems + 1))
    printf 'FAIL  %s: %s\n      fix: %s\n' "$1" "$2" "$3"
}
note() {
    (( quiet == 1 )) || printf 'note  %s: %s\n' "$1" "$2"
}
hint() { # ID MESSAGE FIX: informational, with the command that acts on it
    (( quiet == 1 )) || printf 'note  %s: %s\n      fix: %s\n' "$1" "$2" "$3"
}
ok() {
    (( quiet == 1 )) || printf 'ok    %s: %s\n' "$1" "$2"
}

branch=$(git -C "$root" symbolic-ref --short -q HEAD) || branch=
head=$(git -C "$root" rev-parse --short=8 HEAD 2>/dev/null) || head='unborn'
upstream=$(preflight_integration_ref "$root")

if (( quiet == 0 )); then
    printf 'doctor: %s\n' "$root"
    printf '        HEAD %s on %s; integration ref %s\n' \
        "$head" "${branch:-detached}" "${upstream:-none}"
fi

dirty_count=$(git -C "$root" status --porcelain --untracked-files=normal | wc -l)
dirty_count=${dirty_count//[[:space:]]/}

# 1. Uncommitted work on the shared main. This is the state that makes the
# branch impossible to reconcile: the changes belong to whoever made them, and
# moving main would move them too.
if [ "$branch" = main ] && (( dirty_count > 0 )); then
    fail main-dirty \
        "main is checked out here with $dirty_count uncommitted path(s)" \
        "leave another agent's edits alone; create 'git worktree add .worktrees/<name> -b <name> $upstream' and start the work there"
elif [ "$branch" = main ]; then
    ok main-clean 'main has no uncommitted changes'
fi

# 2. Drift between main and the integration ref, and whether main's own commits
# are already upstream under other hashes (git cherry marks those '-').
if [ -n "$upstream" ]; then
    read -r behind ahead < <(git -C "$root" rev-list --left-right --count "$upstream...HEAD")
    if [ "$branch" = main ]; then
        read -r unique duplicate < <(preflight_cherry "$root" "$upstream" HEAD)
        if (( behind == 0 && ahead == 0 )); then
            ok main-current "main matches $upstream"
        else
            detail="main is $behind behind $upstream"
            if (( ahead == 0 )); then
                detail+="; it has no commits of its own"
            elif (( unique == 0 )); then
                detail+="; all $ahead of its commits are already upstream as duplicate patches"
            else
                detail+="; $unique of its $ahead commits are not upstream, $duplicate already are"
            fi
            fail main-drift "$detail" \
                "just reconcile-main  (archives the old tip, then fast-forwards main; refuses on a dirty tree or unique commits)"
        fi
    elif [ -n "$branch" ] && (( ahead > 0 || behind > 0 )); then
        note branch-position "$branch is $ahead ahead and $behind behind $upstream (normal for a work branch)"
    fi
else
    note no-integration-ref 'no fork/main, origin/main or branch upstream; drift cannot be checked (git branch --set-upstream-to=<remote>/main)'
fi

# 3. Distance to the project this one forks. `fork/main` is the integration
# branch and `origin/main` is phiresky's; the fork stays worth rebasing only
# while its own delta is small, and nothing else in the loop says when it
# stopped being small.
if [ "$upstream" = fork/main ] &&
    git -C "$root" rev-parse --verify --quiet 'origin/main^{commit}' >/dev/null; then
    read -r upstream_behind upstream_ahead < <(git -C "$root" rev-list --left-right --count 'origin/main...fork/main')
    if (( upstream_behind > 0 )); then
        hint upstream-drift \
            "fork/main is $upstream_behind behind origin/main and $upstream_ahead ahead of it" \
            'rebase the integration branch onto origin/main while the delta is still small: git rebase origin/main fork/main'
    else
        ok upstream-current 'fork/main is not behind origin/main'
    fi
fi

# 4. A build inside this checkout. Compiling against a tree that another agent
# may switch produces artifacts nobody can attribute.
builds=$(preflight_build_processes "$root")
if [ -n "$builds" ]; then
    listing=$(printf '%s\n' "$builds" | awk '{printf "%s%s", separator, $2 "[" $1 "]"; separator = ", "}')
    fail build-running "$listing has a working directory inside this checkout" \
        'let it finish (or stop it) before switching branches; builds belong in a worktree: scripts/remote-check.sh SUITE'
else
    ok no-build 'no build process inside this checkout'
fi

# 5. Archived tips and a detached HEAD: the ways back that nobody watches.
archives=()
while IFS= read -r archived; do
    [ -n "$archived" ] && archives+=("$archived")
done < <(git -C "$root" for-each-ref --format='%(refname:short)' 'refs/heads/archive/*' 'refs/heads/archive')

for archived in "${archives[@]}"; do
    if [ -z "$upstream" ]; then
        note archive-unchecked "$archived: no integration ref, so its commits cannot be compared"
        continue
    fi
    read -r unique duplicate < <(preflight_cherry "$root" "$upstream" "$archived")
    if (( unique > 0 )); then
        fail archive-unique \
            "$archived holds $unique commit(s) $upstream does not have ($duplicate already there); deleting or pruning it loses that work" \
            "git log --oneline $upstream..$archived; land each with 'git cherry-pick <sha>' (or merge the branch), then retry"
    else
        note archive-duplicate "$archived: all $duplicate commit(s) already in $upstream; safe to delete"
    fi
done

if [ -z "$branch" ]; then
    uncontained=$(git -C "$root" rev-list --max-count=1 HEAD --not --branches --remotes 2>/dev/null) || uncontained=
    if [ -n "$uncontained" ]; then
        fail detached-unique \
            "HEAD is detached at $head and holds commits no branch or remote contains" \
            "git switch -c rescue/$head, then land it or archive it deliberately"
    else
        note detached-clean "HEAD is detached at $head; no uncontained commits"
    fi
fi

prunable=$(git -C "$root" worktree list --porcelain | awk '/^worktree / { path = $2 } /^prunable/ { print path }')
if [ -n "$prunable" ]; then
    hint worktree-stale "registered work trees are gone: $(printf '%s\n' "$prunable" | tr '\n' ' ')" \
        'git worktree prune'
fi

# 6. Cargo output is the space hog; report it every run, prune incremental first.
for directory in "$root/target" "$root/target/debug/incremental"; do
    if [ -d "$directory" ]; then
        bytes=$(du -s --block-size=1 "$directory" 2>/dev/null | cut -f1) || bytes=
        if [ -n "${bytes:-}" ]; then
            gib=$(awk -v b="$bytes" 'BEGIN { printf "%.1f", b / 1073741824 }')
            hint target-size "${directory#"$root/"} is ${gib} GiB" \
                'rm -rf target/debug/incremental  (cheapest: only the incremental cache goes; delete a whole target only after that)'
        fi
    fi
done

# 7. Free disk. Levers are ordered by what they cost to rebuild.
available_kib=$(df -Pk "$root" | awk 'NR == 2 { print $4 }')
if [ -n "$available_kib" ] && (( available_kib < min_free_gib * 1024 * 1024 )); then
    available_gib=$(awk -v k="$available_kib" 'BEGIN { printf "%.1f", k / 1048576 }')
    fail disk-low "$available_gib GiB free, below the ${min_free_gib} GiB floor" \
        'in order: (1) rm -rf target/debug/incremental in every checkout, (2) scripts/remote-check.sh --prune, (3) nix-collect-garbage --delete-older-than 14d, never -d before an OS upgrade; (4) delete a whole target last'
fi

if (( problems > 0 )); then
    (( quiet == 1 )) || printf '\ndoctor: %d problem(s); fix the FAIL lines above.\n' "$problems"
    exit 1
fi
(( quiet == 1 )) || printf 'doctor: no problems\n'
exit 0
