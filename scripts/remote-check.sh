#!/usr/bin/env bash
# Run one scripts/check-quality.sh suite on the build host.
#
# The runtime is the wrong place to compile this workspace, and the laptop is
# the owner's machine: builds happen on the build host (ROBIN_BUILD_HOST,
# default `build-host`). Every worktree gets its own copy there, because a second
# agent's rsync into a shared copy would swap the source out from under a
# running compile.
#
#   scripts/remote-check.sh engine          # this worktree, engine suite
#   scripts/remote-check.sh engine feat/x   # explicit copy name
#   scripts/remote-check.sh --list          # copies on the host, with sizes
#   scripts/remote-check.sh --prune         # delete every copy but this one
#
# The suite runs under flock, so two agents on one copy queue instead of
# interleaving, and under systemd-inhibit so an idle build host stays awake.
set -euo pipefail

host=${ROBIN_BUILD_HOST:-build-host}
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
remote_root=build/robin

usage() {
    sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

# The copy's name defaults to this worktree's directory, which is what differs
# between concurrent agents.
copy_name() {
    if [ $# -ge 1 ] && [ -n "$1" ]; then
        printf '%s' "$1"
    else
        basename "$repository"
    fi
}

require_host() {
    if ! ssh -o BatchMode=yes -o ConnectTimeout=8 "$host" true 2>/dev/null; then
        printf 'remote-check: %s is unreachable. Wake it with `wake-host build-host` and retry.\n' "$host" >&2
        exit 1
    fi
}

case "${1:-}" in
    -h | --help | "")
        usage
        ;;
    --list)
        require_host
        ssh "$host" "du -sh ~/$remote_root/*/ 2>/dev/null | sort -k2"
        exit 0
        ;;
    --prune)
        require_host
        keep=$(copy_name "${2:-}")
        ssh "$host" "cd ~/$remote_root 2>/dev/null || exit 0; for d in */; do d=\${d%/}; [ \"\$d\" = '$keep' ] || rm -rf -- \"\$d\" \"\$d.lock\"; done; du -sh ~/$remote_root 2>/dev/null || true"
        exit 0
        ;;
esac

suite=$1
name=$(copy_name "${2:-}")

# Validate against the script's own suite list, so nothing user-controlled is
# interpolated into the remote command.
suites=$(bash "$repository/scripts/check-quality.sh" 2>&1 || true)
suites=${suites#*suites: }
case " $suites " in
    *" $suite "*) ;;
    *)
        printf 'remote-check: unknown suite %s\nsuites: %s\n' "$suite" "$suites" >&2
        exit 2
        ;;
esac

require_host

remote_dir="~/$remote_root/$name"
lock="$remote_dir.lock"
printf 'remote-check: %s %s -> %s:%s\n' "$suite" "$repository" "$host" "$remote_dir" >&2

# --delete keeps a copy faithful to this worktree; the Cargo target directory
# is excluded so the copy keeps its build cache across runs. The `.git` on the
# build host is the copy's own (see below); a worktree's is a file naming a
# gitdir that only exists here, so it must not travel.
ssh "$host" bash -s <<EOF
set -euo pipefail
mkdir -p ~/$remote_root
EOF

rsync -a --delete \
    --exclude target \
    --exclude .worktrees \
    --exclude .git \
    "$repository/" "$host:$remote_dir/"

ssh "$host" bash -s <<EOF
set -euo pipefail
cd $remote_dir
# build-support/robin_build.rs stamps ROBIN_GIT_HASH from "git rev-parse HEAD",
# and the replay format rejects anything that is not 12 lowercase hex. A copy
# with no usable repository stamps "unknown", and the replay and leaderboard
# suites then fail version validation (27 tests, 2026-11-15). The copy needs a
# real repository rather than a stub ref, because "nix develop" resolves this
# directory as a git flake and hashes its tree.
if ! git rev-parse --git-dir >/dev/null 2>&1; then
    rm -rf .git
    git init -q -b main .
fi
EOF

# Send this checkout's objects, incrementally, and point the copy's HEAD at
# them. The rsync'd working tree stays untouched, so uncommitted changes are
# still what the suite compiles. `read-tree` refreshes the index from the new
# HEAD: without it every file would look deleted and the flake would hash an
# empty tree.
git -C "$repository" push -q --force "$host:$remote_dir/.git" HEAD:refs/robin/check

ssh "$host" bash -s <<EOF
set -euo pipefail
cd $remote_dir
git update-ref refs/heads/main refs/robin/check
git symbolic-ref HEAD refs/heads/main
git read-tree HEAD
git rev-parse HEAD
exec systemd-inhibit --what=idle --why='check-quality $suite' \
    flock -w 7200 $lock \
    nix develop --command bash scripts/check-quality.sh $suite
EOF
