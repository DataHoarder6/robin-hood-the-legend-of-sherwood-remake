#!/usr/bin/env bash
# Run one scripts/check-quality.sh suite on the build host.
#
# The runtime is the wrong place to compile this workspace, and the laptop is
# the owner's machine: builds happen on the build host (ROBIN_BUILD_HOST,
# default `minipc`). Every worktree gets its own copy there, because a second
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

host=${ROBIN_BUILD_HOST:-minipc}
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
        printf 'remote-check: %s is unreachable. Wake it with `minipc-wake` and retry.\n' "$host" >&2
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
suites=$(bash "$repository/scripts/check-quality.sh" 2>&1 | sed -n 's/^suites: //p')
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
# is excluded so the copy keeps its build cache across runs.
rsync -a --delete \
    --exclude target \
    --exclude .worktrees \
    "$repository/" "$host:$remote_dir/"

ssh "$host" "mkdir -p ~/$remote_root && cd $remote_dir && exec systemd-inhibit --what=idle:sleep --why='check-quality $suite' flock -w 7200 $lock nix develop --command bash scripts/check-quality.sh $suite"
