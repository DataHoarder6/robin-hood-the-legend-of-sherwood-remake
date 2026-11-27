#!/usr/bin/env bash
# Run one scripts/check-quality.sh suite on the build host.
#
# The runtime is the wrong place to compile this workspace, and the laptop is
# the owner's machine: builds happen on the build host (ROBIN_BUILD_HOST,
# default `minipc`). A check takes a warm copy from the pool (`pool-1` ..
# `pool-$ROBIN_BUILD_SLOTS`) and holds its lock from rsync to the last suite, so
# no second agent can swap the source out from under a running compile; gates
# use the one `gate-shared` copy.
#
#   scripts/remote-check.sh engine          # this worktree, engine suite
#   scripts/remote-check.sh engine feat/x   # explicit copy name
#   scripts/remote-check.sh --suites tooling format
#                                           # several suites, one synced tree
#   scripts/remote-check.sh --sync          # copy this worktree to its own copy, print the path
#   scripts/remote-check.sh --list          # copies on the host, with sizes
#   scripts/remote-check.sh --prune         # delete copies whose worktree is gone
#   scripts/remote-check.sh --prune-stale [DAYS]
#                                           # delete copies untouched for DAYS (default 7)
#
# The suites run under flock, so two agents on one copy queue instead of
# interleaving, and under systemd-inhibit so an idle build host stays awake.
# They also take one of the host's build slots (ROBIN_BUILD_SLOTS, default 2),
# which is what keeps four copies from running forty-eight compilers on twelve
# cores; each slot hands Cargo its share of the cores.
set -euo pipefail

# Nothing below reads stdin, and callers feed scripts to `bash -s`: a child that
# inherits a script's stdin (ssh, rsync, git over ssh) drains the lines the
# caller has not executed yet, and the script still exits 0.
exec </dev/null

# push-gate exports this for doc-only landings; a direct remote-check has no
# such variable. Default it here: `${var@Q}` on an unset name aborts under
# set -u while the heredoc renders.
docs_paths=${ROBIN_DOCS_CHECK_PATHS:-}
# push-gate exports the landing's diff so `tooling` runs only the fixtures it
# can affect (scripts/run-fixtures.sh); interactive checks run them all.
changed_paths=${ROBIN_CHANGED_PATHS:-}
# The process-layer e2e suite needs these on the build host. Forward them
# explicitly instead of making the verifier hand-write an ssh gate: a bare
# check-quality.sh multiplayer-e2e fails on a missing ROBIN_E2E_BINARY, which
# cost the loop a verifier round on D10. Paths are the BUILD HOST's.
e2e_binary=${ROBIN_E2E_BINARY:-}
e2e_data=${ROBINHOOD_DATA_DIR:-}
e2e_evidence=${ROBIN_E2E_EVIDENCE:-}
e2e_jobs=${ROBIN_E2E_JOBS:-2}
# The cross-OS replay parity suite hard-fails on these three (check-quality.sh
# uses ${var:?}), and the two optional directory/replay overrides change which
# binary layout and corpus it checks. Paths are the BUILD HOST's. Without all
# six, `remote-check.sh parity-cross` can never pass, however it is invoked.
parity_linux_bin=${PARITY_LINUX_BIN:-}
parity_windows_exe=${PARITY_WINDOWS_EXE:-}
parity_data_dir=${PARITY_DATA_DIR:-}
parity_linux_dir=${PARITY_LINUX_DIR:-}
parity_windows_dir=${PARITY_WINDOWS_DIR:-}
parity_replay=${PARITY_REPLAY:-}

host=${ROBIN_BUILD_HOST:-minipc}
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
remote_root=build/robin
# The gate copy is infrastructure, not an abandoned worktree cache. Keep it in
# its own root: every prune scans $remote_root only, so no agent's prune can
# delete a gate's cwd mid-build (observed 2026-11-20 with gate-shared).
gate_root=build/robin-gate
gate_copy_name=gate-shared
copy_root=$remote_root
slots=${ROBIN_BUILD_SLOTS:-2}
slot_dir="~/$remote_root/.slots"
# A cold copy has no flake to check before sync, so every devShell invocation
# also proves cargo/rustc can exec as its first command. A GC'd devShell then
# fails with a named cause instead of deep in a compile.
gate_toolchain_guard='set -e; if ! cargo --version >/dev/null || ! rustc --version >/dev/null; then printf "remote-check: gate toolchain cannot exec; a Nix GC may have removed a devShell input.\n" >&2; exit 1; fi; '

# minipc powers itself off when idle (nixos-config services.build-host-idle-
# poweroff). Before the remote heredoc registers systemd-inhibit, a build has no
# cargo/rustc process and no logind session on the host, so with aged
# last-activity the timer could power it off mid-rsync into gate-shared. The
# host's scope-marker directory is the only signal that covers that gap: every
# caller drops one per-invocation marker from the moment it owns a copy slot
# until it exits, and the idle predicate treats a non-empty directory as active.
# The name carries the epoch so a marker left by a killed caller can be retired.
# The override is a test seam (see scripts/test_remote_check.sh): fixtures stub
# ssh locally and cannot write /var/lib, so they point this at a temp dir. The
# idle predicate reads the same path in production, configured by NixOS.
scope_dir=${ROBIN_BUILD_SCOPE_DIR:-/var/lib/build-host-idle-poweroff/build-active.d}
scope_marker=

# EXIT cleanup: drop the local queue ticket and the host's scope marker. Both
# are globals — Bash evaluates EXIT traps after a function's locals are gone.
cleanup_remote_check() {
    [ -z "${ticket:-}" ] || rm -f -- "$ticket" 2>/dev/null || true
    if [ -n "$scope_marker" ]; then
        ssh -n "$host" "rm -f -- '$scope_marker'" 2>/dev/null || true
    fi
}
trap cleanup_remote_check EXIT

# Called once the copy slot is held: every ssh/rsync before the gate now runs
# inside the host's idle-poweroff scope. Fail early if the marker cannot be
# written — an unprotected sync is worse than an aborted one.
begin_build_scope() {
    scope_marker="$scope_dir/$(date +%s)-$(hostname)-$$-$RANDOM"
    ssh "$host" "mkdir -p '$scope_dir' && : > '$scope_marker'"
}

usage() {
    awk 'NR > 1 && /^[^#]/{exit} NR > 1{sub(/^# ?/, ""); print}' "${BASH_SOURCE[0]}" >&2
    exit 2
}

# A check without an explicit name takes the warm pool; a gate (ROBIN_GATE_TREE)
# takes gate-shared.
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
#
# Admission is FIFO: a ticket per caller, ordered by arrival under the copy's
# identity. The local flock is only tried when this ticket is first, so twenty
# gates queue in order instead of racing for the lock and timing out at 2 h.
# ROBIN_GATE_QUEUE_WAIT_SECS (default 7200) bounds the wait; a killed caller's
# ticket is reaped by deadline or owner PID.
lock_copy() {
    local lock_dir=${XDG_STATE_HOME:-$HOME/.local/state}/robin-remote-check
    local safe="${host//[^A-Za-z0-9._-]/_}-${name//[^A-Za-z0-9._-]/_}"
    local queue_dir="$lock_dir/queue/$safe"
    local wait=${ROBIN_GATE_QUEUE_WAIT_SECS:-7200}
    local first position count now old old_name old_deadline old_pid announced=
    if [[ ! $wait =~ ^[1-9][0-9]*$ ]]; then
        printf 'remote-check: ROBIN_GATE_QUEUE_WAIT_SECS must be a positive integer, got %s\n' \
            "$wait" >&2
        exit 2
    fi
    mkdir -p -- "$lock_dir" "$queue_dir"
    local deadline=$(( $(date +%s) + wait ))
    # The ticket is global so the EXIT cleanup (set up at the top of this script)
    # retires it after a kill; a dead caller must not hold the queue. BASHPID,
    # not $$: a caller in a subshell must be reaped by its own death.
    ticket="$queue_dir/$(printf '%020d' "$deadline")-$(date +%s%N)-$BASHPID-$RANDOM"
    : >"$ticket"

    while :; do
        now=$(date +%s)

        # A killed harness leaves its ticket behind; its deadline or its dead
        # owner retires it, the same way land.sh reaps landing tickets.
        for old in "$queue_dir"/*; do
            [ -f "$old" ] || continue
            old_name=${old##*/}
            old_deadline=${old_name%%-*}
            case $old_deadline in
                *[!0-9]*) continue ;;
            esac
            old_pid=${old_name#*-}
            old_pid=${old_pid#*-}
            old_pid=${old_pid%%-*}
            if (( 10#$old_deadline < now )) ||
               { [[ $old_pid =~ ^[0-9]+$ ]] && ! kill -0 "$old_pid" 2>/dev/null; }; then
                rm -f -- "$old"
            fi
        done

        first=$(find "$queue_dir" -maxdepth 1 -type f -printf '%f\n' | sort | head -1)
        if [ "$first" = "${ticket##*/}" ] && acquire_copy_lock "$lock_dir" "$safe"; then
            # The held lock now excludes others; the ticket's work is done. Left
            # in the queue it would stay first until this caller exits, and the
            # next caller could not take a free pool copy beside it.
            rm -f -- "$ticket"
            ticket=
            printf 'remote-check: gate copy slot acquired for %s\n' "$name" >&2
            # Cover the pre-gate ssh/rsync work (prepare_cache, toolchain
            # check, sync_worktree) with the host's idle-poweroff scope.
            begin_build_scope
            return 0
        fi

        count=$(find "$queue_dir" -maxdepth 1 -type f -printf '%f\n' | wc -l)
        position=$(find "$queue_dir" -maxdepth 1 -type f -printf '%f\n' | sort |
            awk -v t="${ticket##*/}" '$0 == t { print NR }')
        if [ "${position:-?}" != "$announced" ]; then
            printf 'remote-check: waiting in gate copy queue (%s/%s) for %s\n' \
                "${position:-?}" "$count" "$name" >&2
            announced=${position:-?}
        fi
        if (( now >= deadline )); then
            printf 'remote-check: timed out waiting for a gate copy slot after %s seconds\n' \
                "$wait" >&2
            exit 1
        fi
        sleep 0.2
    done
}

# Take the copy's lock on fd 9. The pool is one queue over ROBIN_BUILD_SLOTS
# copies: the first caller in line takes the first free `pool-N` and `name`
# becomes that copy, so every later step (cache, sync, remote lock) names a real
# directory. Any other name is its own single copy.
acquire_copy_lock() {
    local lock_dir=$1 safe=$2 index
    if [ "$name" != pool ]; then
        exec 9>"$lock_dir/$safe.lock"
        flock -n 9 && return 0
        exec 9>&-
        return 1
    fi
    for index in $(seq "$slots"); do
        exec 9>"$lock_dir/$safe-$index.lock"
        if flock -n 9; then
            name=pool-$index
            return 0
        fi
        exec 9>&-
    done
    return 1
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

# The gate proves a committed tree (push-gate refuses anything else), so the gate
# copy is synced by git: push the commit and check it out. Only files whose
# content changed are written, with a fresh mtime, so Cargo's freshness check is
# exact and nothing reads the whole tree; rsync --checksum read all 463 MB on
# both sides of every gate (about 15 s). Lane worktrees (scripts/gate-lanes.sh)
# check the same commit out from this repository.
sync_gate() {
    remote_dir="~/$gate_root/$name"
    printf 'remote-check: sync %s -> %s:%s (git)\n' "$repository" "$host" "$remote_dir" >&2
    require_disk
    ssh "$host" bash -s <<EOF
set -euo pipefail
mkdir -p $remote_dir
cd $remote_dir
git rev-parse --git-dir >/dev/null 2>&1 || git init -q -b main .
EOF
    git -C "$repository" push -q --force "$host:$remote_dir/.git" HEAD:refs/robin/check
    ssh "$host" bash -s <<EOF
set -euo pipefail
cd $remote_dir
git checkout --quiet --force --detach refs/robin/check
# --prune retires copies by directory age; a copy synced now must look born now.
touch .
git rev-parse HEAD >&2
EOF
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
# crates/robin_build_identity/build.rs stamps the commit from "git rev-parse HEAD",
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
            { cat; [ -z "${2:-}" ] || printf '%s\n' "$2"; seq -f 'pool-%g' "$slots"; printf 'gate-shared\n'; } | sort -u |
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
        # The caller works in this copy by hand after the lock is released, so
        # it is the worktree's own copy, never one the pool hands to an agent.
        name=${2:-$(basename "$repository")}
        require_host
        lock_copy
        sync_worktree
        printf '%s\n' "$remote_dir"
        exit 0
        ;;
esac

if [ "${1:-}" = --suites ]; then
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
# immediately: no path ever takes them in the other order. The remap prefix is
# written with \$PWD inside double quotes so the remote shell expands it to the
# copy root after `cd`; single quotes left the literal text `$(pwd)` in rustc's
# arguments, making the remap a no-op. SCCACHE_CLIENT_SIDE=1 is upstream's
# recommended mode (sccache 0.19), keeping the daemon as a cache gateway only.
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
# The gate copy is one stable tree: Cargo's own fingerprints and incremental
# state make a warm rebuild cheap, and sccache has nothing to add there (its
# Rust hits on gate-shared were Cargo artifacts it had just overwritten). It also
# refuses CARGO_INCREMENTAL=1, so the gate runs without the wrapper the build
# host configures (an empty RUSTC_WRAPPER resets `build.rustc-wrapper`) and with
# the profiles' incremental default (docs/adr/0013-gate-incremental-without-sccache.md).
# Pool copies are swapped between worktrees, so they keep sccache with one
# basedir per copy and Cargo incremental off, which sccache cannot cache.
if [ "$copy_root" = "$gate_root" ]; then
    cache_env="RUSTC_WRAPPER="
else
    prepare_cache
    cache_env="SCCACHE_BASEDIRS=${ROBIN_SCCACHE_BASEDIRS@Q} CARGO_INCREMENTAL=0 SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS=CARGO_MANIFEST_DIR,CARGO_MANIFEST_PATH SCCACHE_IDLE_TIMEOUT=0 SCCACHE_CLIENT_SIDE=1"
fi
remote_dir="~/$copy_root/$name"
# The toolchain is proved as the first command inside the gate's devShell
# (gate_toolchain_guard), which names a GC'd devShell input before any compile.
if [ "$copy_root" = "$gate_root" ]; then
    sync_gate
    runner=gate-lanes.sh
else
    sync_worktree
    runner=run-suites.sh
fi
lock="$remote_dir.lock"
printf 'remote-check: %s -> %s:%s\n' "${requested[*]}" "$host" "$remote_dir" >&2
# One synced tree, one lock, one devShell: a second sync between two suites
# would swap the copy out from under the first, and the suites share its build
# cache. `\$suite` reaches the remote shell untouched.
ssh "$host" bash -s <<EOF
set -euo pipefail
export XDG_RUNTIME_DIR=/run/user/\$(id -u)
# A private TMPDIR per gate keeps `nix develop --command` scratch files from
# accumulating in the shared /tmp; the trap removes them when the gate ends.
gate_tmp=\$(mktemp -d /tmp/robin-gate.XXXXXX)
trap 'rm -rf -- "\$gate_tmp"' EXIT
cd $remote_dir
systemd-run --user --scope --unit=build-${name}-$$ \
    -p RuntimeMaxSec=7200 -p TimeoutStopSec=30 \
    systemd-inhibit --what=idle:sleep --why='check-quality ${requested[*]}' \\
    flock -w 7200 $lock \\
    env $cache_env \
    TMPDIR=\$gate_tmp \
    ROBIN_DOCS_CHECK_PATHS=${docs_paths@Q} \
    ROBIN_CHANGED_PATHS=${changed_paths@Q} \
    ROBIN_E2E_BINARY=${e2e_binary@Q} \
    ROBINHOOD_DATA_DIR=${e2e_data@Q} \
    ROBIN_E2E_EVIDENCE=${e2e_evidence@Q} \
    ROBIN_E2E_JOBS=${e2e_jobs@Q} \
    PARITY_LINUX_BIN=${parity_linux_bin@Q} \
    PARITY_WINDOWS_EXE=${parity_windows_exe@Q} \
    PARITY_DATA_DIR=${parity_data_dir@Q} \
    PARITY_LINUX_DIR=${parity_linux_dir@Q} \
    PARITY_WINDOWS_DIR=${parity_windows_dir@Q} \
    PARITY_REPLAY=${parity_replay@Q} \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="--remap-path-prefix=\$PWD=/robin" \
    CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="--remap-path-prefix=\$PWD=/robin" \
    bash scripts/build-slot.sh $slot_dir $slots \\
    nix develop --command bash -c '${gate_toolchain_guard}bash scripts/$runner ${requested[*]}'
EOF
