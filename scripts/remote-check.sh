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

host=${ROBIN_BUILD_HOST:-minipc}
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
remote_root=build/robin
# The gate copy is infrastructure, not an abandoned worktree cache. Keep it in
# its own root: every prune scans $remote_root only, so no agent's prune can
# delete a gate's cwd mid-build (observed 2026-11-20 with gate-shared).
gate_root=build/robin-gate
gate_copy_name=gate-shared
copy_root=$remote_root
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
        bash "$repository/scripts/gate-copy-name.sh" "$repository" "${ROBIN_GATE_TREE:-}"
    fi
}

# The copy is shared by every worktree that names it. Hold its lock from
# before the first rsync until the last suite finishes: the lock used to be
# taken only around the suites, so a second invocation could sync a different
# tree into the copy while the first suite was compiling it — one gate, two
# verdicts, and a push of a tree the gate never saw. All agents run on this
# host, so the lock lives here, keyed by the copy's identity; the build host's
# own flock still serialises the suites against writers from other machines.
lock_copy() {
    local lock_dir=${XDG_STATE_HOME:-$HOME/.local/state}/robin-remote-check
    local safe="${host//[^A-Za-z0-9._-]/_}-${name//[^A-Za-z0-9._-]/_}"
    mkdir -p -- "$lock_dir"
    exec 9>"$lock_dir/$safe.lock"
    flock -w 7200 9
}

# Gate copies have no worktree owner, so --prune keeps them only until this old.
gate_ttl=${GATE_COPY_TTL_DAYS:-7}

require_host() {
    if ssh -n -o BatchMode=yes -o ConnectTimeout=8 "$host" true 2>/dev/null; then
        return 0
    fi
    if [ "$host" = minipc ]; then
        if ! command -v wake-host >/dev/null; then
            printf 'remote-check: %s is unreachable and wake-host is not on PATH.\n' "$host" >&2
            exit 1
        fi
        printf 'remote-check: %s is off; waking it with `wake-host minipc`.\n' "$host" >&2
        wake-host minipc || true
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

# Each worktree's copy keeps its own target/ tree. On a 12-core host a dozen
# copies fill the disk, and Nix then fails with an opaque SQLite I/O error.
# Refuse a sync below the floor instead: the runner says what to prune, and the
# daily timer on the runtime keeps the floor from being reached again.
min_free_gb=${ROBIN_MIN_FREE_GB:-40}
require_disk() {
    local available
    available=$(ssh "$host" "df -B1 --output=avail / 2>/dev/null | tail -1")
    case $available in
        ''|*[!0-9]*) printf 'remote-check: cannot read free space on %s\n' "$host" >&2; return 1 ;;
    esac
    local min_bytes=$((min_free_gb * 1024 * 1024 * 1024))
    if (( available < min_bytes )); then
        printf 'remote-check: %s has %d GB free, below the %d GB floor; sync refused\n' \
            "$host" "$((available / 1024 / 1024 / 1024))" "$min_free_gb" >&2
        printf 'remote-check: run `%s --prune` on the runtime to remove copies whose worktree is gone, then retry\n' \
            "$repository/scripts/remote-check.sh" >&2
        return 1
    fi
}

sync_worktree() {
    remote_dir="~/$copy_root/$name"
    printf 'remote-check: sync %s -> %s:%s\n' "$repository" "$host" "$remote_dir" >&2
    require_disk

    # --delete keeps a copy faithful to this worktree; the Cargo target directory
    # is excluded so the copy keeps its build cache across runs. The `.git` on the
    # build host is the copy's own (see below); a worktree's is a file naming a
    # gitdir that only exists here, so it must not travel.
    ssh "$host" bash -s <<EOF
set -euo pipefail
mkdir -p ~/$copy_root
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
# rsync -a preserved the source tree's old mtime. --prune retires gate copies by
# directory age, so a copy synced today must look born today or a concurrent
# prune deletes it mid-build (observed 2026-11-20).
touch .
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
        days=${2:-$gate_ttl}
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
    # The shared gate copy is infrastructure, not an abandoned worktree cache.
    # Deleting it mid-gate leaves the scope with a vanished cwd and a failed gate.
    [ "\$d" = gate-shared ] && continue
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
            { cat; copy_name "${2:-}"; printf 'gate-shared\n'; } | sort -u |
            sed 's/.*/"&"/' | paste -sd' ' -)
        # Only real copies: the root is shared, and a cache or scratch directory
        # someone keeps beside the copies is not this script's to delete.
        # The script lives in the checkout that invoked us, not in the build
        # host's home: `--prune` is also called by the runtime's timer, which
        # runs from a different directory. Copy it with the stdin script and
        # execute the copy, so no remote CWD is assumed.
        ssh "$host" bash -s <<EOF
set -euo pipefail
pruner=\$(mktemp)
cat >"\$pruner" <<'PRUNER'
$(cat "$repository/scripts/prune-copies.sh")
PRUNER
chmod +x "\$pruner"
bash "\$pruner" ~/$remote_root $gate_ttl $keep
rm -f "\$pruner"
du -sh ~/$remote_root 2>/dev/null || true
EOF
        exit 0
        ;;
    --sync)
        name=$(copy_name "${2:-}")
        require_host
        lock_copy
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

if [ "$name" = "$gate_copy_name" ]; then
    copy_root=$gate_root
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

# sccache normalizes absolute paths before hashing. One basedir per live copy
# is required: a single parent leaves `<copy>` in the key, so two identical
# trees still miss. The server reads this list at startup, so restart it when
# the set changes. The cache lock is taken before any copy lock and released
# immediately: no path ever takes them in the other order.
prepare_cache() {
    local state_dir=${XDG_STATE_HOME:-$HOME/.local/state}/robin-remote-check
    local list_file=$state_dir/sccache-basedirs
    local lock_file=$state_dir/sccache-basedirs.lock
    mkdir -p -- "$state_dir"
    exec 8>"$lock_file"
    flock -w 300 8
    local list
    list=$(ssh "$host" "find \$HOME/$remote_root \$HOME/$gate_root -mindepth 1 -maxdepth 1 -type d -printf '%p:' 2>/dev/null | sed 's/:\$//'")
    case ":$list:" in
        *":$HOME/$copy_root/$name:"*) ;;
        *) list="${list:+$list:}$HOME/$copy_root/$name" ;;
    esac
    if [ ! -f "$list_file" ] || [ "$(cat "$list_file")" != "$list" ]; then
        printf '%s\n' "$list" >"$list_file"
        # Stopping is enough: the next cargo invocation starts a server with
        # the list below. A stopped server cannot keep an old key space alive.
        ssh "$host" 'sccache --stop-server >/dev/null 2>&1 || true' || true
    fi
    export ROBIN_SCCACHE_BASEDIRS=$list
    flock -u 8
}

lock_copy
prepare_cache
sync_worktree
lock="$remote_dir.lock"
if ((multi)); then
    printf 'remote-check: %s -> %s:%s\n' "${requested[*]}" "$host" "$remote_dir" >&2
    # One synced tree, one lock, one devShell: a second sync between two suites
    # would swap the copy out from under the first, and the suites share its
    # build cache. `\$suite` reaches the remote shell untouched.
    ssh "$host" bash -s <<EOF
set -euo pipefail
export XDG_RUNTIME_DIR=/run/user/\$(id -u)
cd $remote_dir
systemd-run --user --scope --unit=build-${name}-$$ \
    -p RuntimeMaxSec=7200 -p TimeoutStopSec=30 \
    systemd-inhibit --what=idle --why='check-quality ${requested[*]}' \\
    flock -w 7200 $lock \\
    env SCCACHE_BASEDIRS=${ROBIN_SCCACHE_BASEDIRS@Q} \
    CARGO_INCREMENTAL=1 \
    SCCACHE_ALLOW_INCREMENTAL=1 \
    SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS=CARGO_MANIFEST_DIR,CARGO_MANIFEST_PATH \
    SCCACHE_IDLE_TIMEOUT=0 \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS='--remap-path-prefix=\$(pwd)=/robin' \
    CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS='--remap-path-prefix=\$(pwd)=/robin' \
    bash scripts/build-slot.sh $slot_dir $slots \\
    nix develop --command bash -c 'set -e; for suite in ${requested[*]}; do bash scripts/check-quality.sh "\$suite"; done'
EOF
else
    printf 'remote-check: %s -> %s:%s\n' "${requested[0]}" "$host" "$remote_dir" >&2
    ssh "$host" bash -s <<EOF
set -euo pipefail
export XDG_RUNTIME_DIR=/run/user/\$(id -u)
cd $remote_dir
systemd-run --user --scope --unit=build-${name}-$$ \
    -p RuntimeMaxSec=7200 -p TimeoutStopSec=30 \
    systemd-inhibit --what=idle --why='check-quality ${requested[0]}' \\
    flock -w 7200 $lock \\
    env SCCACHE_BASEDIRS=${ROBIN_SCCACHE_BASEDIRS@Q} \
    CARGO_INCREMENTAL=1 \
    SCCACHE_ALLOW_INCREMENTAL=1 \
    SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS=CARGO_MANIFEST_DIR,CARGO_MANIFEST_PATH \
    SCCACHE_IDLE_TIMEOUT=0 \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS='--remap-path-prefix=\$(pwd)=/robin' \
    CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS='--remap-path-prefix=\$(pwd)=/robin' \
    bash scripts/build-slot.sh $slot_dir $slots \\
    nix develop --command bash scripts/check-quality.sh ${requested[0]}
EOF
fi
