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
#   scripts/remote-check.sh --suites tooling format
#                                           # several suites, one synced tree
#   scripts/remote-check.sh --sync          # copy this worktree, print the remote path
#   scripts/remote-check.sh --list          # copies on the host, with sizes
#   scripts/remote-check.sh --prune         # delete copies whose worktree is gone
#   scripts/remote-check.sh --prune-stale [DAYS]
#                                           # delete copies untouched for DAYS (default 7)
#
# The suites run under flock, so two agents on one copy queue instead of
# interleaving, and under systemd-inhibit so an idle build host stays awake.
# They also take one of the host's build slots (ROBIN_BUILD_SLOTS, default 3),
# which is what keeps four copies from running forty-eight compilers on twelve
# cores; each slot hands Cargo its share of the cores.
set -euo pipefail

# Nothing below reads stdin, and callers feed scripts to `bash -s`: a child that
# inherits a script's stdin (ssh, rsync, git over ssh) drains the lines the
# caller has not executed yet, and the script still exits 0.
exec </dev/null

host=${ROBIN_BUILD_HOST:-build-host}
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
remote_root=build/robin
slots=${ROBIN_BUILD_SLOTS:-3}
slot_dir="~/$remote_root/.slots"

usage() {
    awk 'NR > 1 && /^[^#]/{exit} NR > 1{sub(/^# ?/, ""); print}' "${BASH_SOURCE[0]}" >&2
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
    if ssh -n -o BatchMode=yes -o ConnectTimeout=8 "$host" true 2>/dev/null; then
        return 0
    fi
    if [ "$host" = build-host ]; then
        if ! command -v wake-host >/dev/null; then
            printf 'remote-check: %s is unreachable and wake-host is not on PATH.\n' "$host" >&2
            exit 1
        fi
        printf 'remote-check: %s is off; waking it with `wake-host build-host`.\n' "$host" >&2
        wake-host build-host || true
        # WOL is fire-and-forget, so poll the ssh port for the boot budget.
        for _ in $(seq 60); do
            if ssh -n -o BatchMode=yes -o ConnectTimeout=3 "$host" true 2>/dev/null; then
                printf 'remote-check: %s is up.\n' "$host" >&2
                return 0
            fi
            sleep 2
        done
    fi
    printf 'remote-check: %s is unreachable. Wake it with `wake-host %s` and retry.\n' "$host" "$host" >&2
    exit 1
}

sync_worktree() {
    remote_dir="~/$remote_root/$name"
    printf 'remote-check: sync %s -> %s:%s\n' "$repository" "$host" "$remote_dir" >&2

    # --delete keeps a copy faithful to this worktree; the Cargo target directory
    # is excluded so the copy keeps its build cache across runs. The `.git` on the
    # build host is the copy's own (see below); a worktree's is a file naming a
    # gitdir that only exists here, so it must not travel.
    ssh "$host" bash -s <<EOF
set -euo pipefail
mkdir -p ~/$remote_root
EOF

    # `--no-times` after `-a`: a changed file must arrive with a mtime *now*, not
    # the mtime it had in the worktree. `--checksum` keeps the quick-check exact,
    # so unchanged files keep their old mtime and are not recompiled, and the
    # itemized output names exactly the files whose content changed.
    #
    # Cargo decides freshness by mtime, and this workspace keeps a copy of each
    # crate's rmeta under `target/<profile>/build/<crate>/<hash>/out`. A file
    # whose content is new but whose time is older than the build that followed
    # it — a merge commit's checkout — therefore reads as fresh and the stale
    # copy is linked: measured 2026-11-18, the engine file defined
    # `transition_budget` while the build insisted it did not, on three gates in
    # a row. So the copy's build state for every crate whose tree changed is
    # purged, and mtime can no longer decide.
    changed_crates=$(rsync -a --no-times --checksum --delete --itemize-changes \
        --exclude target \
        --exclude .worktrees \
        --exclude .git \
        "$repository/" "$host:$remote_dir/" \
        | awk '/^>f/ {print $2}' \
        | awk -F/ '/^crates\// && NF > 1 {print $2}' \
        | sort -u)
    if [ -n "$changed_crates" ]; then
        ssh "$host" bash -s <<EOF
set -euo pipefail
cd $remote_dir
for crate in $changed_crates; do
  rm -rf "target/debug/build/\$crate" target/debug/.fingerprint/"\$crate"-*
done
EOF
    fi

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
    # still what the build compiles. `read-tree` refreshes the index from the new
    # HEAD: without it every file would look deleted and the flake would hash an
    # empty tree.
    git -C "$repository" push -q --force "$host:$remote_dir/.git" HEAD:refs/robin/check

    ssh "$host" bash -s <<EOF
set -euo pipefail
cd $remote_dir
git update-ref refs/heads/main refs/robin/check
git symbolic-ref HEAD refs/heads/main
git read-tree HEAD
# stdout is the copy path alone, so callers can capture it.
git rev-parse HEAD >&2
EOF
}

case "${1:-}" in
    -h | --help | "")
        usage
        ;;
    --list)
        require_host
        ssh -n "$host" "du -sh ~/$remote_root/*/ 2>/dev/null | sort -k2"
        exit 0
        ;;
    --prune-stale)
        require_host
        keep=$(copy_name "")
        days=${2:-7}
        case $days in
            '' | *[!0-9]*)
                printf 'remote-check: --prune-stale takes a day count, not %s\n' "$days" >&2
                exit 2
                ;;
        esac
        # The login shell on the build host is not POSIX, so the loop runs under
        # bash over stdin. Only directories that look like a copy are candidates,
        # and a copy is stale when this host has not written into it for DAYS.
        ssh "$host" bash -s <<EOF
set -euo pipefail
cd ~/$remote_root 2>/dev/null || exit 0
for d in */; do
    d=\${d%/}
    [ "\$d" = '$keep' ] && continue
    [ -f "\$d/Cargo.toml" ] || continue
    [ -n "\$(find "\$d" -maxdepth 0 -mtime +$days)" ] || continue
    printf 'pruned %s\n' "\$d" >&2
    rm -rf -- "\$d" "\$d.lock"
done
du -sh ~/$remote_root 2>/dev/null || true
EOF
        exit 0
        ;;
    --prune)
        require_host
        # A copy whose worktree is gone is a cache nothing can reach. Keep every
        # copy that still has an owner — the live worktrees, plus this run's
        # copy — and delete the rest. Age is the wrong test: a copy can be a
        # week old and still be the only build cache its worktree has.
        keep=$(bash "$repository/scripts/live-worktrees.sh" |
            { cat; copy_name "${2:-}"; } | sort -u |
            sed 's/.*/"&"/' | paste -sd' ' -)
        # Only real copies: the root is shared, and a cache or scratch directory
        # someone keeps beside the copies is not this script's to delete.
        ssh "$host" bash -s <<EOF
set -euo pipefail
cd ~/$remote_root 2>/dev/null || exit 0
keep=($keep)
for d in */; do
    d=\${d%/}
    [ -f "\$d/Cargo.toml" ] || continue
    kept=0
    for k in "\${keep[@]}"; do [ "\$d" = "\$k" ] && kept=1; done
    [ "\$kept" = 1 ] && continue
    rm -rf -- "\$d" "\$d.lock"
done
du -sh ~/$remote_root 2>/dev/null || true
EOF
        exit 0
        ;;
    --sync)
        name=$(copy_name "${2:-}")
        require_host
        sync_worktree
        printf '%s\n' "$remote_dir"
        exit 0
        ;;
esac

multi=0
if [ "${1:-}" = --suites ]; then
    multi=1
    shift
    (( $# > 0 )) || usage
    # One copy, one name: the suites share a tree, so there is nothing for a
    # second name to separate.
    name=$(copy_name "")
    requested=("$@")
else
    (( $# > 0 )) || usage
    name=$(copy_name "${2:-}")
    requested=("$1")
fi

# Validate against the script's own suite list, so nothing user-controlled is
# interpolated into the remote command.
known=$(bash "$repository/scripts/check-quality.sh" 2>&1 || true)
known=${known#*suites: }
for suite in "${requested[@]}"; do
    case " $known " in
        *" $suite "*) ;;
        *)
            printf 'remote-check: unknown suite %s\nsuites: %s\n' "$suite" "$known" >&2
            exit 2
            ;;
    esac
done

require_host
sync_worktree
lock="$remote_dir.lock"
if ((multi)); then
    printf 'remote-check: %s -> %s:%s\n' "${requested[*]}" "$host" "$remote_dir" >&2
    # One synced tree, one lock, one devShell: a second sync between two suites
    # would swap the copy out from under the first, and the suites share its
    # build cache. `\$suite` reaches the remote shell untouched.
    ssh "$host" bash -s <<EOF
set -euo pipefail
cd $remote_dir
exec systemd-inhibit --what=idle --why='check-quality ${requested[*]}' \\
    flock -w 7200 $lock \\
    env SCCACHE_BASEDIRS=\$(pwd) \\
    bash scripts/build-slot.sh $slot_dir $slots \\
    nix develop --command bash -c 'set -e; for suite in ${requested[*]}; do bash scripts/check-quality.sh "\$suite"; done'
EOF
else
    printf 'remote-check: %s -> %s:%s\n' "${requested[0]}" "$host" "$remote_dir" >&2
    ssh "$host" bash -s <<EOF
set -euo pipefail
cd $remote_dir
exec systemd-inhibit --what=idle --why='check-quality ${requested[0]}' \\
    flock -w 7200 $lock \\
    env SCCACHE_BASEDIRS=\$(pwd) \\
    bash scripts/build-slot.sh $slot_dir $slots \\
    nix develop --command bash scripts/check-quality.sh ${requested[0]}
EOF
fi
