#!/usr/bin/env bash
# Retire the worktrees a merge already finished.
#
#   scripts/prune-worktrees.sh            # print the plan, remove nothing
#   scripts/prune-worktrees.sh --apply    # remove the worktrees the plan names
#
# A worktree is retired only when all of these hold: its branch is an ancestor
# of the integration ref, its tree is clean, it is not detached, no process runs
# inside it, and it is neither the repository's main checkout nor the checkout
# this command runs from. Everything else is kept and printed with the reason,
# because an unmerged branch is a decision, not litter.
#
# `git worktree remove` without --force refuses a dirty tree, and `git branch
# -d` refuses an unmerged branch, so a race with another agent fails the run
# instead of deleting their work.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/lib/preflight.sh
source "$script_dir/lib/preflight.sh"

apply=0
while (( $# > 0 )); do
    case $1 in
        --apply) apply=1 ;;
        -h | --help)
            sed -n '2,14p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
            exit 0
            ;;
        *)
            printf 'prune-worktrees: unknown argument %s\n' "$1" >&2
            exit 2
            ;;
    esac
    shift
done

root=$(preflight_root .) || {
    printf 'prune-worktrees: not inside a git work tree\n' >&2
    exit 2
}
root=$(readlink -f -- "$root")
integration=$(preflight_integration_ref "$root")
if [ -z "$integration" ]; then
    printf 'prune-worktrees: no fork/main, origin/main or branch upstream to compare against\n' >&2
    exit 2
fi

# The main checkout is the first worktree git prints, and it is the one a
# shared `main` lives in: never a candidate, whatever its branch.
main_checkout=$(git -C "$root" worktree list --porcelain -z | { IFS= read -r -d '' first; printf '%s\n' "${first#worktree }"; })
current=$(readlink -f -- "$root")

removed=0
kept=0
failed=0

report() { # VERDICT BRANCH PATH REASON
    printf '%-6s %-34s %s\n' "$1" "${2:-detached}" "$3${4:+  ($4)}"
}

# processes_inside PATH: "PID COMMAND" for every process whose working
# directory is inside PATH. A build is the case doctor fails on; for a removal
# any process counts, because deleting the directory under a running shell or
# agent breaks it even when the branch has already landed.
processes_inside() {
    local root=$1 pid cwd comm
    for pid in /proc/[0-9]*; do
        pid=${pid#/proc/}
        [ "$pid" != "$$" ] || continue
        cwd=$(readlink -f "/proc/$pid/cwd" 2>/dev/null) || continue
        case $cwd in
            "$root" | "$root"/*) ;;
            *) continue ;;
        esac
        comm=$(cat "/proc/$pid/comm" 2>/dev/null) || continue
        printf '%s %s\n' "$pid" "$comm"
    done
}

retire() { # BRANCH PATH REASON
    local branch=$1 path=$2 reason=$3
    if (( apply == 0 )); then
        report remove "$branch" "$path" "$reason"
        removed=$((removed + 1))
        return
    fi
    if ! git -C "$root" worktree remove -- "$path"; then
        printf 'prune-worktrees: could not remove %s\n' "$path" >&2
        failed=$((failed + 1))
        return
    fi
    if ! git -C "$root" branch -d -- "$branch"; then
        printf 'prune-worktrees: removed %s but kept branch %s\n' "$path" "$branch" >&2
        failed=$((failed + 1))
        return
    fi
    report remove "$branch" "$path" "$reason"
    removed=$((removed + 1))
}

keep() { # BRANCH PATH REASON
    report keep "$1" "$2" "$3"
    kept=$((kept + 1))
}

consider() { # PATH BRANCH DETACHED LOCKED
    local path=$1 branch=$2 detached=$3 locked=$4
    path=$(readlink -f -- "$path")

    [ "$path" != "$main_checkout" ] || return 0
    if [ "$path" = "$current" ]; then
        keep "$branch" "$path" 'this checkout'
        return 0
    fi
    if [ "$locked" = 1 ]; then
        keep "$branch" "$path" 'locked'
        return 0
    fi
    if [ "$detached" = 1 ]; then
        keep "$branch" "$path" 'detached HEAD'
        return 0
    fi
    if [ -z "$branch" ]; then
        keep "$branch" "$path" 'no branch'
        return 0
    fi
    if preflight_dirty "$path"; then
        keep "$branch" "$path" 'uncommitted changes'
        return 0
    fi
    if [ -n "$(preflight_build_processes "$path")" ]; then
        keep "$branch" "$path" 'a build is running here'
        return 0
    fi
    if [ -n "$(processes_inside "$path")" ]; then
        keep "$branch" "$path" 'a process is running here'
        return 0
    fi
    if ! git -C "$root" merge-base --is-ancestor "refs/heads/$branch" "$integration" 2>/dev/null; then
        keep "$branch" "$path" "commits not in $integration"
        return 0
    fi
    retire "$branch" "$path" "merged into $integration"
}

path=
branch=
detached=0
locked=0
flush() {
    [ -n "$path" ] || return 0
    consider "$path" "$branch" "$detached" "$locked"
    path= branch= detached=0 locked=0
}

while IFS= read -r -d '' field; do
    case $field in
        '') flush ;;
        'worktree '*) path=${field#worktree } ;;
        'branch '*) branch=${field#branch refs/heads/} ;;
        detached) detached=1 ;;
        locked | locked\ *) locked=1 ;;
    esac
done < <(git -C "$root" worktree list --porcelain -z)
flush

printf 'prune-worktrees: %s, integration ref %s: %d to remove, %d kept\n' \
    "$root" "$integration" "$removed" "$kept"
if (( apply == 0 )) && (( removed > 0 )); then
    printf 'prune-worktrees: nothing removed; rerun with --apply\n'
fi
if (( failed > 0 )); then
    exit 1
fi
