#!/usr/bin/env bash
# Fixture tests for scripts/reconcile-main.sh: synthetic repositories in a temp
# dir, no network. The recipe must refuse every state where moving main would
# lose work, and fast-forward the one state it exists for.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
reconcile=$source_root/scripts/reconcile-main.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-reconcile.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

export HOME=$work/home
export GIT_CONFIG_GLOBAL=$work/gitconfig
export GIT_CONFIG_NOSYSTEM=1
mkdir -p "$HOME"
git config --file "$GIT_CONFIG_GLOBAL" user.name Fixture
git config --file "$GIT_CONFIG_GLOBAL" user.email fixture@example.invalid
git config --file "$GIT_CONFIG_GLOBAL" init.defaultBranch main

pass_count=0
failures=0
pass() { printf 'ok   %s\n' "$1"; pass_count=$((pass_count + 1)); }
fail() {
    printf 'FAIL %s\n' "$1"
    failures=$((failures + 1))
}
status=0
output=
run_reconcile() { # DIRECTORY
    status=0
    output=$(cd "$1" && "$reconcile" 2>&1) || status=$?
}
expect_status() {
    if [ "$status" -eq "$1" ]; then pass "$2"; else fail "$2 (exit $status, want $1)"; fi
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

git init --quiet --bare "$work/origin.git"
git init --quiet "$work/seed"
printf 'base\n' >"$work/seed/file.txt"
git -C "$work/seed" add file.txt
git -C "$work/seed" commit --quiet -m base
git -C "$work/seed" push --quiet "$work/origin.git" HEAD:main
git -C "$work/origin.git" symbolic-ref HEAD refs/heads/main

clone() { git clone --quiet "$work/origin.git" "$work/$1"; }
commit_file() { # REPO MESSAGE LINE
    printf '%s\n' "$3" >>"$1/file.txt"
    git -C "$1" commit --quiet -am "$2"
}

# Not on main: the recipe only moves main.
clone topic
git -C "$work/topic" switch --quiet -c docs/topic
run_reconcile "$work/topic"
expect_status 1 'a work branch is refused'
expect_contains 'refusal names the branch' "on 'docs/topic', not main"

# Dirty main: refuse and point at doctor.
clone dirty
printf 'someone else\n' >>"$work/dirty/file.txt"
before=$(git -C "$work/dirty" rev-parse HEAD)
run_reconcile "$work/dirty"
expect_status 1 'a dirty main is refused'
expect_contains 'dirty refusal points at doctor' 'just doctor'
if [ "$(git -C "$work/dirty" rev-parse HEAD)" = "$before" ]; then
    pass 'dirty refusal leaves main alone'
else
    fail 'dirty refusal moved main'
fi

# A commit the integration ref does not have: never dropped.
clone unique
commit_file "$work/unique" 'unpushed work' unpushed
before=$(git -C "$work/unique" rev-parse HEAD)
run_reconcile "$work/unique"
expect_status 1 'unique commits are refused'
expect_contains 'unique refusal lists the commit' 'unpushed work'
expect_contains 'unique refusal says how to land it' 'cherry-pick'
if [ "$(git -C "$work/unique" rev-parse HEAD)" = "$before" ]; then
    pass 'unique refusal leaves main alone'
else
    fail 'unique refusal moved main'
fi

# Divergence whose commits are already upstream as duplicate patches: archive
# the tip and move main.
clone diverged
commit_file "$work/diverged" 'diverged work' diverged
old=$(git -C "$work/diverged" rev-parse HEAD)
git clone --quiet "$work/origin.git" "$work/peer"
commit_file "$work/peer" 'landed as another commit' diverged
git -C "$work/peer" push --quiet origin main
run_reconcile "$work/diverged"
expect_status 0 'duplicate divergence reconciles'
expect_contains 'reconcile archives the old tip' 'archived old tip'
expect_contains 'reconcile counts the skipped duplicates' 'duplicate commit(s) skipped'
if [ "$(git -C "$work/diverged" rev-parse HEAD)" = "$(git -C "$work/diverged" rev-parse origin/main)" ]; then
    pass 'main now matches the integration ref'
else
    fail 'main did not move to the integration ref'
fi
archive=$(git -C "$work/diverged" for-each-ref --format='%(refname:short)' 'refs/heads/archive/main-*')
if [ -n "$archive" ] && [ "$(git -C "$work/diverged" rev-parse "$archive")" = "$old" ]; then
    pass 'old tip is reachable from the archive branch'
else
    fail 'old tip was not archived at its commit'
fi

# Running again on a reconciled checkout is a no-op.
run_reconcile "$work/diverged"
expect_status 0 'reconciled checkout is a no-op'
expect_contains 'no-op says main matches' 'already matches'

printf '\n%s checks passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
