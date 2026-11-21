#!/usr/bin/env bash
# Two overlapping remote-check invocations on ONE copy name must never swap
# the tree a running suite is reading. No build host is involved: ssh, rsync,
# the git push into the copy, nix and the slot wrapper are stubbed locally, and
# a fixture suite reads its tree token twice around a sleep. A sync that lands
# between those two reads is the race this file refuses.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-remote-check.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

export HOME=$work/home
export GIT_CONFIG_GLOBAL=$work/gitconfig
export GIT_CONFIG_NOSYSTEM=1
mkdir -p "$HOME"
git config --file "$GIT_CONFIG_GLOBAL" user.name Fixture
git config --file "$GIT_CONFIG_GLOBAL" user.email fixture@example.invalid
git config --file "$GIT_CONFIG_GLOBAL" init.defaultBranch main
git config --file "$GIT_CONFIG_GLOBAL" commit.gpgsign false

real_git=$(command -v git)
stubs=$work/stubs
mkdir -p "$stubs"

# ssh runs the remote bash here, under the same HOME the copy path expands into.
cat >"$stubs/ssh" <<'STUB'
#!/usr/bin/env bash
while [[ $# -gt 0 ]]; do
    case $1 in
        -o) shift 2 ;;
        -*) shift ;;
        *) break ;;
    esac
done
host=$1; shift
if [[ $* == 'bash -s' ]]; then
    # Keep the exact heredoc body the outer remote-check rendered (variables
    # expanded, remote-side `\$` escaped) so the fixture can syntax-check the
    # generated script, not the source that produced it.
    if [[ -n ${FIXTURE_RENDERED:-} ]]; then
        rendered=$FIXTURE_RENDERED/rendered-$$.sh
        cat > "$rendered"
        exec bash -s < "$rendered"
    fi
    exec bash -s
fi
exec bash -c "$*"
STUB

# rsync copies the source into the emulated copy, delayed so a second sync
# lands while the first invocation's suite is still reading its tree.
cat >"$stubs/rsync" <<'STUB'
#!/usr/bin/env bash
args=("$@")
src=${args[${#args[@]}-2]}
dest=${args[${#args[@]}-1]}
dest=${dest#*:}
dest=${dest/#\~/$HOME}
sleep 0.3
mkdir -p "$dest"
cp -a "$src"/. "$dest"/
STUB

# The real remote-check pushes this checkout into the copy's .git. Emulate it
# with one empty commit and the ref the remote script then reads.
cat >"$stubs/git" <<'STUB'
#!/usr/bin/env bash
if [[ ${3:-} == push ]]; then
    target=${6#*:}
    target=${target%/.git}
    target=${target/#\~/$HOME}
    mkdir -p "$target"
    (
        cd -- "$target"
        git init -q -b main . 2>/dev/null || true
        git commit -q --allow-empty -m fixture 2>/dev/null || true
        git update-ref refs/robin/check HEAD
    )
    exit 0
fi
exec "$REAL_GIT" "$@"
STUB

cat >"$stubs/nix" <<'STUB'
#!/usr/bin/env bash
while [[ $# -gt 0 && ${1:-} != --command ]]; do shift; done
[[ ${1:-} == --command ]] && shift
exec "$@"
STUB

cat >"$stubs/systemd-inhibit" <<'STUB'
#!/usr/bin/env bash
while [[ ${1:-} == --* ]]; do shift; done
exec "$@"
STUB

cat >"$stubs/df" <<'STUB'
#!/usr/bin/env bash
free=${FIXTURE_FREE_GB:-9999}
printf '%s\n' "$((free * 1024 * 1024 * 1024))"
STUB

# The devShell toolchain preflight runs cargo/rustc through the nix stub, so
# stub the versions too: the fixture overrides HOME and has no rustup toolchain.
for tool in cargo rustc; do
    cat >"$stubs/$tool" <<STUB
#!/usr/bin/env bash
printf '%s 1.0.0-fixture\n' '$tool'
STUB
done

for stub in ssh rsync git nix systemd-inhibit df cargo rustc; do chmod +x "$stubs/$stub"; done
export PATH="$stubs:$PATH"
export REAL_GIT=$real_git

export FIXTURE_RESULTS=$work/results
export FIXTURE_STARTED=$work/started
export FIXTURE_RENDERED=$work/rendered
mkdir -p "$FIXTURE_RENDERED"

make_repo() {
    local dir=$1 token=$2
    mkdir -p "$dir/scripts"
    cp -- "$source_root/scripts/remote-check.sh" "$dir/scripts/remote-check.sh"
    printf '[package]\nname = "fixture"\nversion = "0.0.0"\n' >"$dir/Cargo.toml"
    printf '%s\n' "$token" >"$dir/tree-token"
    cat >"$dir/scripts/check-quality.sh" <<'SUITE'
#!/usr/bin/env bash
set -euo pipefail
if (( $# == 0 )); then
    printf 'suites: format wasm\n' >&2
    exit 2
fi
suite=$1
before=$(cat tree-token)
printf '%s\n' "$suite" >"$FIXTURE_STARTED.$suite"
sleep 2
after=$(cat tree-token)
if [[ $before == "$after" ]]; then
    printf 'STABLE %s %s\n' "$suite" "$before" >>"$FIXTURE_RESULTS"
else
    printf 'UNSTABLE %s %s %s\n' "$suite" "$before" "$after" >>"$FIXTURE_RESULTS"
fi
SUITE
    printf '#!/usr/bin/env bash\nshift 2\nexec "$@"\n' >"$dir/scripts/build-slot.sh"
    chmod +x "$dir/scripts/check-quality.sh" "$dir/scripts/build-slot.sh" \
        "$dir/scripts/remote-check.sh"
}

make_repo "$work/repoA" AAA
make_repo "$work/repoB" BBB

# Both invocations target the SAME copy name. A starts first; once its suite
# has read the tree, B syncs a different token into that copy.
(cd "$work/repoA" && bash scripts/remote-check.sh format race-copy) &
a_pid=$!
for _attempt in $(seq 100); do
    [ -e "$FIXTURE_STARTED.format" ] && break
    sleep 0.05
done
(cd "$work/repoB" && bash scripts/remote-check.sh wasm race-copy) &
b_pid=$!
wait "$a_pid" "$b_pid"

results=$(sort "$FIXTURE_RESULTS")
printf '%s\n' "$results"

failures=0
for expected in 'STABLE format AAA' 'STABLE wasm BBB'; do
    if printf '%s\n' "$results" | grep -qFx "$expected"; then
        printf 'ok   %s\n' "$expected"
    else
        printf 'FAIL %s\n' "$expected"
        failures=$((failures + 1))
    fi
done

# A full build host must refuse before rsync touches the copy.
set +e
(cd "$work/repoA" && FIXTURE_FREE_GB=1 bash scripts/remote-check.sh format disk-floor) \
    >"$work/disk.out" 2>&1
disk_status=$?
set -e
if [[ $disk_status -ne 0 ]] && grep -q 'below the .* GB floor' "$work/disk.out"; then
    printf 'ok   full disk refuses the sync\n'
else
    printf 'FAIL full disk did not refuse the sync (status %s)\n' "$disk_status"
    failures=$((failures + 1))
fi

# A gate invoked directly (not through push-gate) has no ROBIN_DOCS_CHECK_PATHS.
# `${var@Q}` on an unset variable aborts under set -u while the heredoc renders.
set +e
(cd "$work/repoA" && env -u ROBIN_DOCS_CHECK_PATHS \
    bash scripts/remote-check.sh wasm docs-unset) >"$work/unset.out" 2>&1
unset_status=$?
set -e
if (( unset_status == 0 )) && ! grep -q 'unbound variable' "$work/unset.out"; then
    printf 'ok   a gate with no docs-path variable still runs\n'
else
    printf 'FAIL direct gate aborts when ROBIN_DOCS_CHECK_PATHS is unset (status %s)\n' "$unset_status"
    failures=$((failures + 1))
fi

# The copy root must have been touched after sync even though rsync copied
# files with --no-times and left the directory with an old mtime.
copy_dir="$HOME/build/robin/race-copy"
# Every heredoc remote-check rendered over ssh must be a well-formed shell
# script. `bash -n` parses the captured output, so a broken escaped variable
# or an unterminated construct in the generated text fails here rather than
# on the build host.
rendered_count=0
for script in "$FIXTURE_RENDERED"/rendered-*.sh; do
    [[ -f $script ]] || continue
    rendered_count=$((rendered_count + 1))
    if bash -n "$script" 2>"$work/syntax.err"; then
        printf 'ok   rendered heredoc %s parses\n' "$(basename "$script")"
    else
        printf 'FAIL rendered heredoc %s does not parse: %s\n' \
            "$(basename "$script")" "$(cat "$work/syntax.err")"
        failures=$((failures + 1))
    fi
done
if (( rendered_count == 0 )); then
    printf 'FAIL no remote heredocs were captured for the syntax check\n'
    failures=$((failures + 1))
fi

printf '\n%s checks failed\n' "$failures"
(( failures == 0 ))
