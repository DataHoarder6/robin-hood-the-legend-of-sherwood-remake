#!/usr/bin/env bash
# Fixture tests for the build host's client scripts: the slot pool that bounds
# how many worktree copies compile at once, and the worktree list `--prune`
# keeps. No ssh, no build host — a temp dir and a synthetic repository.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-build-host.XXXXXX")
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

slot=$source_root/scripts/build-slot.sh
cores=$(nproc)

# --- a held slot is not shared -------------------------------------------------
mkdir -p "$work/slots-one"
bash "$slot" "$work/slots-one" 1 bash -c '
    touch "$1/held"
    while [ ! -e "$1/release" ]; do sleep 0.05; done
' _ "$work/slots-one" &
holder=$!
for _attempt in {1..100}; do [ -e "$work/slots-one/held" ] && break; sleep 0.05; done
bash "$slot" "$work/slots-one" 1 true &
waiter=$!
sleep 0.3
if kill -0 "$waiter" 2>/dev/null; then
    pass "a second build waits while the only slot is held"
else
    fail "a second build ran while the only slot was held"
fi
touch "$work/slots-one/release"
wait "$holder" "$waiter"

# --- a contended host divides the cores ---------------------------------------
mkdir -p "$work/slots-two"
bash "$slot" "$work/slots-two" 2 bash -c '
    touch "$1/held"
    while [ ! -e "$1/release" ]; do sleep 0.05; done
' _ "$work/slots-two" &
holder=$!
for _attempt in {1..100}; do [ -e "$work/slots-two/held" ] && break; sleep 0.05; done
shared=$(bash "$slot" "$work/slots-two" 2 bash -c 'printf %s "$CARGO_BUILD_JOBS"')
touch "$work/slots-two/release"
wait "$holder"
expected=$(( cores / 2 ))
if [ "$shared" = "$expected" ]; then
    pass "a contended host gives each build $expected of $cores cores"
else
    fail "contended jobs was '$shared', expected '$expected'"
fi

# --- a lone build gets the whole machine --------------------------------------
lone=$(bash "$slot" "$work/slots-two" 2 bash -c 'printf %s "$CARGO_BUILD_JOBS"')
if [ "$lone" = "$cores" ]; then
    pass "a lone build gets all $cores cores"
else
    fail "lone jobs was '$lone', expected '$cores'"
fi

# --- --prune keeps the copies that still have a worktree ----------------------
repo=$work/repo
git init --quiet "$repo"
git -C "$repo" commit --quiet --allow-empty -m "fixture"
git -C "$repo" worktree add --quiet -b other "$work/other-worktree" HEAD
live=$(cd "$repo" && bash "$source_root/scripts/live-worktrees.sh")
if [ "$live" = "$(printf '%s\n' "$(basename "$repo")" "other-worktree" | sort)" ]; then
    pass "live-worktrees names every worktree and nothing else"
else
    fail "live-worktrees printed: $live"
fi

printf '\n%s passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
