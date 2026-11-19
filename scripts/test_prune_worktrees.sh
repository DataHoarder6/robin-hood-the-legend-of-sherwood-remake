#!/usr/bin/env bash
# Fixture tests for scripts/prune-worktrees.sh: synthetic repositories in a
# temp dir, no network, no shared checkout, no writes outside the fixture.
# Every scenario is a worktree the command must tell apart: merged, unmerged,
# dirty, detached, building, outside the repository, and the one it runs from.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
prune=$source_root/scripts/prune-worktrees.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-prune.XXXXXX")
build_pid=
stop_build() {
    if [ -n "$build_pid" ]; then
        kill "$build_pid" 2>/dev/null || true
        wait "$build_pid" 2>/dev/null || true
        build_pid=
    fi
}
trap 'stop_build; rm -rf -- "$work"' EXIT

export HOME=$work/home
export GIT_CONFIG_GLOBAL=$work/gitconfig
export GIT_CONFIG_NOSYSTEM=1
mkdir -p "$HOME" "$work/bin"
git config --file "$GIT_CONFIG_GLOBAL" user.name Fixture
git config --file "$GIT_CONFIG_GLOBAL" user.email fixture@example.invalid
git config --file "$GIT_CONFIG_GLOBAL" init.defaultBranch main
git config --file "$GIT_CONFIG_GLOBAL" advice.detachedHead false

pass_count=0
failures=0
pass() { printf 'ok   %s\n' "$1"; pass_count=$((pass_count + 1)); }
fail() {
    printf 'FAIL %s\n' "$1"
    failures=$((failures + 1))
}
status=0
output=
run_prune() { # DIRECTORY [ARG...]
    local directory=$1
    shift
    status=0
    output=$(cd "$directory" && "$prune" "$@" 2>&1) || status=$?
}
expect_status() {
    if [ "$status" -eq "$1" ]; then
        pass "$2"
    else
        fail "$2 (exit $status, want $1)"
        printf '%s\n' "$output" | sed 's/^/     /'
    fi
}
expect_contains() {
    case $output in
        *"$2"*) pass "$1" ;;
        *)
            fail "$1 (missing '$2')"
            printf '%s\n' "$output" | sed 's/^/     /'
            ;;
    esac
}
expect_absent() {
    case $output in
        *"$2"*) fail "$1 (unexpected '$2')" ;;
        *) pass "$1" ;;
    esac
}
expect_exists() {
    if [ -e "$2" ]; then pass "$1"; else fail "$1 (missing $2)"; fi
}
expect_gone() {
    if [ -e "$2" ]; then fail "$1 (still there: $2)"; else pass "$1"; fi
}
expect_branch() { # NAME BRANCH
    if git -C "$work/repo" rev-parse --verify --quiet "refs/heads/$2" >/dev/null; then
        pass "$1"
    else
        fail "$1 (branch $2 is gone)"
    fi
}

git init --quiet --bare "$work/origin.git"
git init --quiet "$work/seed"
printf 'base\n' >"$work/seed/file.txt"
git -C "$work/seed" add file.txt
git -C "$work/seed" commit --quiet -m base
git -C "$work/seed" push --quiet "$work/origin.git" HEAD:main
git -C "$work/origin.git" symbolic-ref HEAD refs/heads/main
git clone --quiet "$work/origin.git" "$work/repo"
# The integration ref the fixture prunes against. A local branch is enough:
# preflight_integration_ref accepts fork/main without a remote of that name.
git -C "$work/repo" branch fork/main origin/main

repo=$work/repo
add_worktree() { # NAME [--detach]
    local name=$1
    shift
    git -C "$repo" worktree add --quiet "$@" "$repo/.worktrees/$name" -b "$name" fork/main
}

add_worktree merged
add_worktree unmerged
printf 'unique\n' >"$repo/.worktrees/unmerged/unique.txt"
git -C "$repo/.worktrees/unmerged" add unique.txt
git -C "$repo/.worktrees/unmerged" commit --quiet -m unique
add_worktree dirty
printf 'x\n' >>"$repo/.worktrees/dirty/file.txt"
git -C "$repo" worktree add --quiet --detach "$repo/.worktrees/detached" fork/main
add_worktree building
add_worktree outside-check
git -C "$repo" worktree move "$repo/.worktrees/outside-check" "$work/outside"

# A compiler whose working directory is inside the worktree. The symlink name
# is what /proc reports as the command, so this stands in for a real tool.
ln -s "$(command -v python3)" "$work/bin/cargo"
(cd "$repo/.worktrees/building" && exec "$work/bin/cargo" -c 'import time; time.sleep(300)') &
build_pid=$!
for _ in {1..50}; do
    [ "$(readlink -f "/proc/$build_pid/cwd" 2>/dev/null)" = "$(readlink -f "$repo/.worktrees/building")" ] && break
    sleep 0.1
done

# The plan removes nothing.
run_prune "$repo"
expect_status 0 'the plan exits 0'
expect_contains 'merged branch is planned' 'remove merged'
expect_contains 'worktree outside the repository is planned' 'remove outside-check'
expect_contains 'unmerged branch is kept' 'commits not in fork/main'
expect_contains 'dirty tree is kept' 'uncommitted changes'
expect_contains 'detached worktree is kept' 'detached HEAD'
expect_contains 'running build is kept' 'a build is running here'
expect_contains 'the plan says nothing was removed' 'rerun with --apply'
expect_exists 'the plan leaves the merged worktree' "$repo/.worktrees/merged"
expect_branch 'the plan leaves the merged branch' merged

# --apply retires exactly the planned worktrees.
run_prune "$repo" --apply
expect_status 0 '--apply exits 0'
expect_gone 'the merged worktree is removed' "$repo/.worktrees/merged"
expect_gone 'the outside worktree is removed' "$work/outside"
expect_exists 'the unmerged worktree stays' "$repo/.worktrees/unmerged"
expect_exists 'the dirty worktree stays' "$repo/.worktrees/dirty"
expect_exists 'the detached worktree stays' "$repo/.worktrees/detached"
expect_exists 'the building worktree stays' "$repo/.worktrees/building"
expect_exists 'the main checkout stays' "$repo/file.txt"
expect_branch 'the unmerged branch stays' unmerged
if git -C "$repo" rev-parse --verify --quiet refs/heads/merged >/dev/null; then
    fail 'the merged branch is deleted with its worktree'
else
    pass 'the merged branch is deleted with its worktree'
fi
stop_build

# The checkout the command runs from is never a candidate, even when its
# branch is merged.
git -C "$repo" worktree add --quiet "$repo/.worktrees/current" -b current fork/main
run_prune "$repo/.worktrees/current" --apply
expect_status 0 'running from a merged worktree exits 0'
expect_exists 'the running checkout stays' "$repo/.worktrees/current"
expect_contains 'the running checkout is named' 'this checkout'

# A repository without an integration ref is refused, not guessed at.
git init --quiet "$work/lonely"
git -C "$work/lonely" commit --quiet --allow-empty -m base
run_prune "$work/lonely"
expect_status 2 'a repository with no integration ref is refused'
expect_contains 'the refusal names the missing ref' 'no fork/main, origin/main or branch upstream'

if (( failures > 0 )); then
    printf '\n%d passed, %d failed\n' "$pass_count" "$failures"
    exit 1
fi
printf '\n%d passed\n' "$pass_count"
