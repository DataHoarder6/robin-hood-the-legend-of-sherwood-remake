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

# --- basedir changes restart sccache and remap each copy root ------------------
mkdir -p "$HOME/bin" "$HOME/.cache"
cat > "$HOME/bin/sccache" <<'STUB'
#!/usr/bin/env bash
[ "${1:-}" = --stop-server ] && touch "$HOME/sccache-stopped"
exit 0
STUB
chmod +x "$HOME/bin/sccache"
mkdir -p "$HOME/build/robin/a" "$HOME/build/robin/b"
rm -f "$HOME/.cache/sccache-basedirs.set" "$HOME/sccache-stopped"
PATH="$HOME/bin:$PATH" bash "$source_root/scripts/sccache-basedirs.sh" >/dev/null
if [ -e "$HOME/sccache-stopped" ]; then
    pass "a changed basedir set restarts the sccache daemon"
else
    fail "sccache was not restarted when the basedir set changed"
fi
rm -f "$HOME/sccache-stopped"
PATH="$HOME/bin:$PATH" bash "$source_root/scripts/sccache-basedirs.sh" >/dev/null
if [ -e "$HOME/sccache-stopped" ]; then
    fail "an unchanged basedir set restarted sccache again"
else
    pass "an unchanged basedir set leaves the daemon running"
fi
remap=$(bash "$source_root/scripts/sccache-basedirs.sh" --remap)
case "$remap" in
    *"--remap-path-prefix=$HOME/build/robin/a=/robin"*"--remap-path-prefix=$HOME/build/robin/b=/robin"*)
        pass "remap flags use each copy root, not the shared parent" ;;
    *) fail "remap flags were: $remap" ;;
esac

# --- sccache basedirs name every copy root ------------------------------------
mkdir -p "$HOME/build/robin/a" "$HOME/build/robin/b"
: > "$HOME/build/robin/not-a-copy"
basedirs=$(bash "$source_root/scripts/sccache-basedirs.sh")
expected="$HOME/build/robin/a:$HOME/build/robin/b"
if [ "$basedirs" = "$expected" ]; then
    pass "sccache basedirs list only existing copy roots"
else
    fail "sccache basedirs printed '$basedirs', expected '$expected'"
fi
if grep -F 'SCCACHE_BASEDIRS=$(pwd)' "$source_root/scripts/remote-check.sh" >/dev/null; then
    fail "remote-check still overrides SCCACHE_BASEDIRS per invocation"
else
    pass "remote-check does not override SCCACHE_BASEDIRS per invocation"
fi
if [ "$(grep -c 'CARGO_INCREMENTAL=1' "$source_root/scripts/remote-check.sh")" -eq 2 ] &&
   [ "$(grep -c 'SCCACHE_ALLOW_INCREMENTAL=1' "$source_root/scripts/remote-check.sh")" -eq 2 ]; then
    pass "both remote commands use Cargo incremental with sccache's allowance"
else
    fail "remote-check does not enable incremental compilation in both commands"
fi
for flag in CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS; do
    if grep -F "$flag" "$source_root/scripts/remote-check.sh" >/dev/null && \
       grep -F 'remap-path-prefix=' "$source_root/scripts/remote-check.sh" >/dev/null; then
        pass "remote gates inject per-root remap flags for $flag"
    else
        fail "remote gates do not inject remap flags for $flag"
    fi
done
if grep -F 'SCCACHE_BASEDIRS=\"' "$source_root/scripts/remote-check.sh" >/dev/null; then
    fail "the remote heredoc has literal backslashes before quotes"
else
    pass "the remote heredoc quotes the basedirs command substitution correctly"
fi

# --- every remote gate runs in a bounded transient scope ----------------------
rc=0
for needle in 'systemd-run --user --scope' 'XDG_RUNTIME_DIR=/run/user/' '--unit=build-${name}-$$' 'RuntimeMaxSec=7200' 'TimeoutStopSec=30'; do
    count=$(grep -F -c -- "$needle" "$source_root/scripts/remote-check.sh" || true)
    if [ "$count" -ge 2 ]; then
        pass "remote-check has two scoped commands with $needle"
    else
        fail "remote-check has $count occurrences of $needle, expected at least two"
    fi
done

# --- every gate reuses one locked copy so its target cache survives ------------
tree_sha=0123456789abcdef0123456789abcdef01234567
other_sha=fedcba9876543210fedcba9876543210fedcba98
name=$(bash "$source_root/scripts/gate-copy-name.sh" "$repo")
if [ "$name" = "$(basename "$repo")" ]; then
    pass "an interactive copy keeps the worktree name"
else
    fail "interactive copy was '$name'"
fi
name=$(bash "$source_root/scripts/gate-copy-name.sh" "$repo" "$tree_sha")
if [ "$name" = "gate-shared" ]; then
    pass "a gate copy has one stable name"
else
    fail "gate copy was '$name', expected gate-shared"
fi
same=$(bash "$source_root/scripts/gate-copy-name.sh" "$repo" "$tree_sha")
other=$(bash "$source_root/scripts/gate-copy-name.sh" "$repo" "$other_sha")
if [ "$same" = "$name" ] && [ "$other" = "$name" ]; then
    pass "different trees reuse the same gate copy and its build cache"
else
    fail "gate naming is not stable across trees"
fi

git -C "$repo" worktree add --quiet -b gate-check "$work/gate-check" HEAD
live=$(cd "$repo" && bash "$source_root/scripts/live-worktrees.sh")
case "$live" in
    *gate-check*) fail "live-worktrees printed a gate copy: $live" ;;
    *) pass "live-worktrees ignores gate copies" ;;
esac

if grep -F 'GATE_COPY_TTL_DAYS' "$source_root/scripts/remote-check.sh" >/dev/null && \
   grep -F 'gate-' "$source_root/scripts/remote-check.sh" >/dev/null; then
    pass "gate copies have a bounded retention rule"
else
    fail "remote-check has no gate-copy retention rule"
fi

# --- slot admission is FIFO and bounded ---------------------------------------
mkdir -p "$work/slots-fifo"
bash "$slot" "$work/slots-fifo" 1 bash -c '
    touch "$1/held"
    while [ ! -e "$1/release" ]; do sleep 0.05; done
' _ "$work/slots-fifo" &
holder=$!
for _attempt in {1..100}; do [ -e "$work/slots-fifo/held" ] && break; sleep 0.05; done
: > "$work/slots-fifo/order"
bash "$slot" "$work/slots-fifo" 1 bash -c 'printf "A\n" >> "$1/order"' _ "$work/slots-fifo" &
first_waiter=$!
for _attempt in {1..100}; do
    [ -n "$(find "$work/slots-fifo/queue" -maxdepth 1 -type f -print -quit 2>/dev/null)" ] && break
    sleep 0.05
done
bash "$slot" "$work/slots-fifo" 1 bash -c 'printf "B\n" >> "$1/order"' _ "$work/slots-fifo" &
second_waiter=$!
sleep 0.3
touch "$work/slots-fifo/release"
wait "$holder" "$first_waiter" "$second_waiter"
if [ "$(cat "$work/slots-fifo/order")" = "$(printf 'A\nB')" ]; then
    pass "waiters enter in arrival order"
else
    fail "waiter order was '$(cat "$work/slots-fifo/order")', expected A then B"
fi

mkdir -p "$work/slots-timeout"
bash "$slot" "$work/slots-timeout" 1 bash -c '
    touch "$1/held"
    while [ ! -e "$1/release" ]; do sleep 0.05; done
' _ "$work/slots-timeout" &
holder=$!
for _attempt in {1..100}; do [ -e "$work/slots-timeout/held" ] && break; sleep 0.05; done
set +e
timeout 3 env BUILD_SLOT_WAIT_SECS=1 bash "$slot" "$work/slots-timeout" 1 true \
    > "$work/timeout.out" 2> "$work/timeout.err"
timeout_rc=$?
set -e
touch "$work/slots-timeout/release"
wait "$holder"
if [ "$timeout_rc" -ne 0 ] && grep -F 'timed out' "$work/timeout.err" >/dev/null; then
    pass "a blocked waiter exits with a named timeout"
else
    fail "timeout rc=$timeout_rc, stderr: $(cat "$work/timeout.err")"
fi

# --- prune removes only expired gate copies -----------------------------------
mkdir -p "$work/prune-root/gate-old" "$work/prune-root/gate-new" "$work/prune-root/live"
for d in gate-old gate-new live; do : > "$work/prune-root/$d/Cargo.toml"; done
touch -d '30 days ago' "$work/prune-root/gate-old"
bash "$source_root/scripts/prune-copies.sh" "$work/prune-root" 7 'live' >/dev/null 2>&1 || true
if [ ! -e "$work/prune-root/gate-old" ] && \
   [ -e "$work/prune-root/gate-new" ] && \
   [ -e "$work/prune-root/live" ]; then
    pass "prune removes an expired gate copy and keeps fresh and owned copies"
else
    fail "prune left old=$(test -e "$work/prune-root/gate-old" && echo yes || echo no) new=$(test -e "$work/prune-root/gate-new" && echo yes || echo no) live=$(test -e "$work/prune-root/live" && echo yes || echo no)"
fi

# --- a fresh sync keeps a fresh mtime so prune cannot delete it mid-build -----
if grep -Fq 'touch .' "$source_root/scripts/remote-check.sh"; then
    pass "sync refreshes the gate copy mtime"
else
    fail "rsync -a leaves the source mtime; a fresh gate copy looks expired to prune"
fi

# --- the gate copy lives outside the root that every prune scans -------------
if grep -Fq 'gate_root=build/robin-gate' "$source_root/scripts/remote-check.sh" &&
   grep -Fq 'remote_dir="~/$copy_root/$name"' "$source_root/scripts/remote-check.sh" &&
   grep -Fq '"\$d" = gate-shared' "$source_root/scripts/remote-check.sh"; then
    pass "gate copy has its own root and prune skips the shared copy"
else
    fail "gate copy can be deleted by a prune mid-build"
fi

printf '\n%s passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
