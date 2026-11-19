#!/usr/bin/env bash
# Fixture tests for scripts/doctor.sh: synthetic repositories in a temp dir, no
# network, no shared checkout, no writes outside the fixture. Every scenario is
# a state doctor must tell apart.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
doctor=$source_root/scripts/doctor.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-doctor.XXXXXX")
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
# Fixtures are about repository states, not the host's disk. The disk-floor
# cases set this explicitly; every other scenario must pass on a build host
# that happens to be full.
export DOCTOR_MIN_FREE_GIB=0
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
run_doctor() { # DIRECTORY [ARG...]
    local directory=$1
    shift
    status=0
    output=$(cd "$directory" && "$doctor" "$@" 2>&1) || status=$?
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

# A clean clone, current with the integration ref.
clone clean
run_doctor "$work/clean"
expect_status 0 'clean checkout passes'
expect_contains 'clean checkout says so' 'doctor: no problems'

# Uncommitted work on the shared main: the state that blocked reconciliation.
clone dirty
printf 'someone else\n' >>"$work/dirty/file.txt"
run_doctor "$work/dirty"
expect_status 1 'dirty main fails'
expect_contains 'dirty main names the worktree rule' 'worktree add'
expect_contains 'dirty main counts the paths' '1 uncommitted path'

# main behind the integration ref.
clone behind
git clone --quiet "$work/origin.git" "$work/peer"
commit_file "$work/peer" next later
git -C "$work/peer" push --quiet origin main
git -C "$work/behind" fetch --quiet origin
run_doctor "$work/behind"
expect_status 1 'behind main fails'
expect_contains 'behind main reports the count' '1 behind'
expect_contains 'behind main says it holds no unique work' 'no commits of its own'
expect_contains 'behind main points at the recipe' 'just reconcile-main'

# main carrying a commit whose patch is already upstream (the 2026-11-17 shape).
clone duplicate
commit_file "$work/duplicate" 'duplicated locally' orphan
commit_file "$work/peer" 'landed under another message' orphan
git -C "$work/peer" push --quiet origin main
git -C "$work/duplicate" fetch --quiet origin
run_doctor "$work/duplicate"
expect_status 1 'duplicate-only main fails'
expect_contains 'duplicate commit is named as already upstream' 'already upstream as duplicate patches'
expect_absent 'duplicate-only main has nothing unique' 'not upstream,'

# An archived branch that holds a commit upstream does not have.
clone archive_unique
git -C "$work/archive_unique" switch --quiet -c archive/session
commit_file "$work/archive_unique" 'unique work' unique
git -C "$work/archive_unique" switch --quiet main
run_doctor "$work/archive_unique"
expect_status 1 'archive branch with unique work fails'
expect_contains 'unique archive names the branch' 'archive/session'
expect_contains 'unique archive says upstream lacks it' 'does not have'
expect_absent 'unique archive is never called safe to drop' 'safe to delete'

# An archived branch whose commits are all upstream is a note, not a failure.
clone archive_duplicate
git -C "$work/archive_duplicate" branch archive/old
run_doctor "$work/archive_duplicate"
expect_status 0 'duplicate-only archive passes'
expect_contains 'duplicate archive is called safe to delete' 'safe to delete'

# A detached HEAD holding work no branch contains.
clone detached
git -C "$work/detached" checkout --quiet --detach
commit_file "$work/detached" 'detached work' detached
run_doctor "$work/detached"
expect_status 1 'detached unique work fails'
expect_contains 'detached unique work is named' 'no branch or remote contains'

# A compiler whose working directory is inside the checkout. The symlink name
# is what /proc reports as the command, so this stands in for a real tool.
clone building
ln -s "$(command -v python3)" "$work/bin/cargo"
(cd "$work/building" && exec "$work/bin/cargo" -c 'import time; time.sleep(300)') &
build_pid=$!
for _ in {1..50}; do
    [ "$(readlink -f "/proc/$build_pid/cwd" 2>/dev/null)" = "$(readlink -f "$work/building")" ] && break
    sleep 0.1
done
run_doctor "$work/building"
expect_status 1 'running build fails'
expect_contains 'running build names the process' 'cargo['
stop_build

# No remote at all: the drift check is skipped, not crashed.
git init --quiet "$work/local"
printf 'base\n' >"$work/local/file.txt"
git -C "$work/local" add file.txt
git -C "$work/local" commit --quiet -m base
run_doctor "$work/local"
expect_status 0 'repository without remotes passes'
expect_contains 'missing integration ref is stated' 'no fork/main, origin/main or branch upstream'

# --quiet keeps failures and drops the report.
clone quiet
printf 'x\n' >>"$work/quiet/file.txt"
run_doctor "$work/quiet" --quiet
expect_status 1 'quiet mode still fails'
expect_contains 'quiet mode prints the finding' 'FAIL  main-dirty'
expect_absent 'quiet mode drops the header' 'doctor: '
expect_absent 'quiet mode drops notes' 'note  '

# Cargo output sizes are informational and never fatal.
clone sized
printf 'target/\n' >>"$work/sized/.git/info/exclude"
mkdir -p "$work/sized/target/debug/incremental"
printf 'x\n' >"$work/sized/target/debug/incremental/blob"
run_doctor "$work/sized"
expect_status 0 'cargo output does not fail the check'
expect_contains 'target size is reported' 'target is 0.0 GiB'
expect_contains 'incremental size is reported' 'target/debug/incremental is 0.0 GiB'

# The disk floor fails with the ordered levers.
export DOCTOR_MIN_FREE_GIB=999999
run_doctor "$work/clean"
export DOCTOR_MIN_FREE_GIB=0
expect_status 1 'low disk fails'
expect_contains 'disk failure names the incremental prune first' 'rm -rf target/debug/incremental in every checkout'
expect_contains 'disk failure names the safe garbage collection' 'nix-collect-garbage --delete-older-than 14d'
expect_absent 'disk failure never suggests the destructive collect' 'nix-collect-garbage -d'

# Distance to the upstream project: a fork/main behind origin/main is a note
# with the numbers, never a failure, and it never fires without a fork ref.
git init --quiet --bare "$work/upstream.git"
git init --quiet "$work/upstream-seed"
printf 'base\n' >"$work/upstream-seed/file.txt"
git -C "$work/upstream-seed" add file.txt
git -C "$work/upstream-seed" commit --quiet -m base
git -C "$work/upstream-seed" push --quiet "$work/upstream.git" HEAD:main
git -C "$work/upstream.git" symbolic-ref HEAD refs/heads/main
git clone --quiet "$work/upstream.git" "$work/forked"
git -C "$work/forked" branch fork/main origin/main
printf 'upstream\n' >"$work/upstream-seed/upstream.txt"
git -C "$work/upstream-seed" add upstream.txt
git -C "$work/upstream-seed" commit --quiet -m upstream
git -C "$work/upstream-seed" push --quiet "$work/upstream.git" HEAD:main
git -C "$work/forked" fetch --quiet origin
run_doctor "$work/forked"
expect_status 0 'upstream drift does not fail the check'
expect_contains 'upstream drift names the distance' 'fork/main is 1 behind origin/main'
expect_contains 'upstream drift names the fix' 'git rebase origin/main fork/main'

# Read-only: a doctor run leaves HEAD and the working tree alone.
before=$(git -C "$work/clean" rev-parse HEAD; git -C "$work/clean" status --porcelain)
run_doctor "$work/clean"
after=$(git -C "$work/clean" rev-parse HEAD; git -C "$work/clean" status --porcelain)
if [ "$before" = "$after" ]; then pass 'doctor leaves HEAD and the tree untouched'; else fail 'doctor modified the repository'; fi

# A directory outside any work tree is a usage error, not a crash.
mkdir -p "$work/plain"
run_doctor "$work/plain"
expect_status 2 'non-repository directory is a usage error'
expect_contains 'non-repository directory is named' 'not inside a git work tree'

printf '\n%s checks passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
