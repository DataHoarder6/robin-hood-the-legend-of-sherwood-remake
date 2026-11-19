#!/usr/bin/env bash
# Fixture tests for scripts/push-gate.sh's tree checks: a synthetic repository in
# a temp dir, no network. The suites themselves need the build host, so every
# case here stops before `scripts/remote-check.sh` runs.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-push-gate.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

export HOME=$work/home
export GIT_CONFIG_GLOBAL=$work/gitconfig
export GIT_CONFIG_NOSYSTEM=1
mkdir -p "$HOME"
git config --file "$GIT_CONFIG_GLOBAL" user.name Fixture
git config --file "$GIT_CONFIG_GLOBAL" user.email fixture@example.invalid
git config --file "$GIT_CONFIG_GLOBAL" init.defaultBranch main
git config --file "$GIT_CONFIG_GLOBAL" commit.gpgsign false

pass_count=0
failures=0
pass() { printf 'ok   %s\n' "$1"; pass_count=$((pass_count + 1)); }
fail() {
    printf 'FAIL %s\n' "$1"
    failures=$((failures + 1))
}
expect_status() {
    if [ "$status" -eq "$1" ]; then pass "$2"; else fail "$2 (exit $status, want $1)"; fi
}
expect_contains() {
    case $output in
        *"$2"*) pass "$1" ;;
        *)
            fail "$1 (missing '$2')"
            printf '%s\n' "$output" | sed 's/^/     /' >&2
            ;;
    esac
}

git init --quiet "$work/repo"
cd -- "$work/repo"
mkdir -p scripts
cp -- "$source_root/scripts/push-gate.sh" scripts/push-gate.sh
printf 'base\n' >file.txt
git add file.txt scripts/push-gate.sh
git commit --quiet -m base
head=$(git rev-parse HEAD)

run_gate() {
    status=0
    output=$(bash scripts/push-gate.sh "$head" "$head" 2>&1) || status=$?
}

# The suites compile the working tree; the push lands the commit. A tree they
# disagree on is a gate that proves what nobody else will build.
printf 'uncommitted\n' >>file.txt
run_gate
expect_status 1 'a worktree that differs from the pushed commit is refused'
expect_contains 'the refusal names the difference' 'worktree differs from the commit being pushed'

# An untracked source file is the same lie: compiled here, absent there.
git checkout -- file.txt
printf 'new module\n' >untracked_module.rs
run_gate
expect_status 1 'an untracked file is refused'
expect_contains 'the refusal names the untracked file' 'untracked_module.rs'

# Nothing to refuse: a tree with no gated path runs no suite and passes.
rm -- untracked_module.rs
run_gate
expect_status 0 'a clean tree with no gated path changes passes'
expect_contains 'the pass says no suite was selected' 'no gated path changed'

printf '\n%s checks passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
