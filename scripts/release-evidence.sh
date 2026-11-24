#!/usr/bin/env bash
# Produce the release evidence the autonomous DIRECTOR cannot produce today.
#
#   scripts/release-evidence.sh <candidate-commit> [version]
#
# Steps, each of which fails closed with a named reason:
#   1. build the Linux release binary from the candidate on the build host;
#   2. obtain robin.exe from native-release.yml and verify its --version sha;
#   3. detect a stale parity corpus, re-record it with that binary and commit
#      the replacement by pathspec (schema maintenance, not gate gaming);
#   4. run parity-cross with all six PARITY_* variables forwarded;
#   5. run release-gate.sh, fetch its summary.json, generate the note with
#      release-notes.sh and commit it by pathspec;
#   6. print the machine-readable paths the driver consumes.
#
# This script NEVER tags, pushes a tag or deploys web. auto-release.sh keeps
# those actions. LIBCLANG_PATH comes from the devShell (`nix develop`), never
# from a value invented here, and Cargo's target directory is never redirected.
set -euo pipefail

exec </dev/null

repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
cd -- "$repository"

usage() {
    printf 'usage: scripts/release-evidence.sh <candidate-commit> [version]\n' >&2
    exit 2
}

refused() { # REASON
    printf 'VERDICT refused reason=%s\n' "$1"
    printf 'release-evidence: %s\n' "$1" >&2
    exit 1
}
die() { refused "$1"; }

[ $# -ge 1 ] || usage
[ $# -le 2 ] || usage
candidate_input=$1
version=${2:-}

scripts=${ROBIN_RELEASE_SCRIPTS:-$repository/scripts}
host=${ROBIN_BUILD_HOST:-minipc}
gh_repo=${ROBIN_RELEASE_REPO:-DataHoarder6/robin-hood-the-legend-of-sherwood-remake}
data_dir=${ROBIN_RELEASE_DATA_DIR:-"~/Games/robin-hood/game"}
corpus=scripts/validation/parity_corpus
corpus_schema=crates/robin_run_types/src/lib.rs
fixture_sherwood=crates/robin_rs/tests/fixtures/scenario_spec_sherwood_hq.json

for tool in git jq python3 ssh scp; do
    command -v "$tool" >/dev/null || die "missing-tool:$tool"
done

# The candidate must be the checked-out commit: release-gate.sh enforces the
# same equality, and a binary built from a different tree would carry a sha
# nobody is tagging.
git rev-parse --verify -q "$candidate_input^{commit}" >/dev/null ||
    die "not-a-commit:$candidate_input"
candidate=$(git rev-parse --verify "$candidate_input^{commit}")
short=${candidate:0:12}
[[ $short =~ ^[0-9a-f]{12}$ ]] || die 'candidate-has-no-12-hex-sha'
head=$(git rev-parse HEAD)
[ "$head" = "$candidate" ] || die "candidate-not-head:$short"

if [ -z "$version" ]; then
    version=$(
        awk '
            /^\[package\]/ { in_package = 1; next }
            /^\[/ { in_package = 0 }
            in_package && /^version = "/ { gsub(/"/, "", $3); print $3; exit }
        ' crates/robin_rs/Cargo.toml
    )
fi
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "bad-version:$version"

for item in "$data_dir"; do
    case $item in '~/'* | /*) ;; *) die "remote-path-not-absolute:$item" ;; esac
    case $item in *[!A-Za-z0-9._/~+-]*) die "unsupported-remote-path:$item" ;; esac
done

# gate-shared is the one name the build-host prune timer always keeps. A
# per-sha copy is deleted mid-build when the timer fires during a long build.
copy_name=gate-shared
binary=''
win_exe_host=''
gate_candidate=$candidate
summary_file=${TMPDIR:-/tmp}/robin-release-summary-$short.json
parity_log=${TMPDIR:-/tmp}/robin-release-parity-$short.log
gate_log=${TMPDIR:-/tmp}/robin-release-gate-$short.log

# The hourly prune keeps only build copies whose name matches a live local
# worktree. Without one, it deletes this script's private copy mid-compile
# (observed 2026-11-24: "No such file or directory" while building robin_rs).
# Register the copy's name as a worktree for the run's lifetime, then remove it.
keep_worktree=$repository/.worktrees/$copy_name
created_keep=0
if [ ! -d "$keep_worktree/.git" ]; then
    git -C "$repository" worktree add --detach --quiet "$keep_worktree" "$candidate" ||
        die 'keep-worktree-failed'
    created_keep=1
fi
cleanup() {
    rm -f -- "$summary_file"
    if [ "$created_keep" -eq 1 ]; then
        git -C "$repository" worktree remove --force "$keep_worktree" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

# ---- 1. Build the Linux release binary from the candidate on the build host.
sync_copy() {
    local out
    out=$(bash "$scripts/remote-check.sh" --sync "$copy_name" 2>/dev/null) ||
        die 'linux-sync-failed'
    [ -n "$out" ] || die 'linux-sync-empty'
    case $out in '~/'* | /*) ;; *) die "linux-sync-bad-path:$out" ;; esac
    case $out in *[!A-Za-z0-9._/~+-]*) die "linux-sync-bad-path:$out" ;; esac
    printf '%s\n' "$out"
}
remote_dir=$(sync_copy)
# remote-check prints a ~/-relative host path. Turn it into an absolute one so
# the six PARITY_* values survive systemd-run's quoting and reach parity_check
# as real paths on the build host.
host_home=$(ssh "$host" 'printf "%s" "$HOME"') || die 'host-home-failed'
[ -n "$host_home" ] || die 'host-home-empty'
remote_abs=${remote_dir/#\~/$host_home}
data_abs=${data_dir/#\~/$host_home}
binary=$remote_abs/target/release/robin

build_linux() { # SHA
    local sha=$1
    ssh "$host" bash -s <<EOF
set -euo pipefail
cd -- "$remote_abs"
# SCCACHE_DISABLE=1 keeps provenance local to this tree; LIBCLANG_PATH and the
# linker are inherited from the pinned devShell, and Cargo's target dir is the
# copy's own (no --target-dir, no CARGO_TARGET_DIR).
export SCCACHE_DISABLE=1
nix develop --command cargo build --locked --release --no-default-features \
    --features release -p robin_rs --bin robin
bin="$binary"
[ -x "\$bin" ] || { printf 'release-evidence: no binary at \$bin\n' >&2; exit 1; }
rendered=\$("\$bin" --version)
case "\$rendered" in
    *$sha*) printf 'release-evidence: linux --version %s\n' "\$rendered" >&2 ;;
    *) printf 'release-evidence: linux binary reports %s, expected $sha\n' "\$rendered" >&2; exit 1 ;;
esac
EOF
}
if ! build_linux "$short" >"$gate_log" 2>&1; then
    cat -- "$gate_log" >&2
    die 'linux-build-failed'
fi

# ---- 2. Obtain robin.exe built from the candidate and verify it under wine.
obtain_windows() { # SHA
    local sha=$1 ref=${ROBIN_RELEASE_GH_REF:-main} download run_id state deadline
    local interval=${ROBIN_RELEASE_POLL_SECS:-10} rendered exe
    command -v gh >/dev/null || die 'gh-unavailable'
    # Wine lives on the build host that replays the Windows binary; the runtime
    # only downloads the artifact and ships it there.
    ssh -n "$host" 'command -v wine >/dev/null 2>&1' || die 'wine-unavailable'
    download=$(mktemp -d "${TMPDIR:-/tmp}/robin-release-exe.XXXXXX")
    gh workflow run native-release.yml --repo "$gh_repo" --ref "$ref" >/dev/null ||
        die 'gh-dispatch-failed'
    deadline=$(( $(date +%s) + ${ROBIN_RELEASE_RUN_TIMEOUT:-3600} ))
    run_id=
    while [ "$(date +%s)" -lt "$deadline" ]; do
        run_id=$(gh run list --repo "$gh_repo" --workflow native-release.yml --event workflow_dispatch \
            --branch "$ref" --limit 1 --json databaseId \
            --jq '.[0].databaseId' 2>/dev/null || true)
        [ -n "$run_id" ] && [ "$run_id" != null ] && break
        run_id=
        sleep "$interval"
    done
    [ -n "$run_id" ] || die 'gh-run-not-found'
    state=
    while [ "$(date +%s)" -lt "$deadline" ]; do
        state=$(gh run view "$run_id" --repo "$gh_repo" --json status,conclusion \
            --jq '.status + " " + (.conclusion // "")' 2>/dev/null || true)
        case $state in completed*) break ;; esac
        sleep "$interval"
    done
    case $state in completed*success*) ;; *) die "gh-run-failed:${state:-unknown}" ;; esac
    gh run download "$run_id" --repo "$gh_repo" --name robin-windows-x86_64-exe --dir "$download" ||
        die 'gh-download-failed'
    exe=$(find "$download" -type f -name robin.exe -print -quit)
    [ -n "$exe" ] || die 'windows-exe-missing'
    # The Windows exe is replayed on the build host, so verify its --version
    # there with that host's wine, not with a runtime-side wine that does not
    # exist. scp first, then ask the build host to render the banner.
    probe=$remote_abs/verify-$run_id.exe
    scp -q -- "$exe" "$host:$probe" || die 'windows-exe-scp-failed'
    # The build host is headless. wine without a DISPLAY fails with
    # "no driver could be loaded", which used to be misreported as a commit
    # mismatch. Run it under a private Xvfb and keep the banner on stdout.
    rendered=$(
        ssh -n "$host" bash -s <<EOF2
set -e
ROBIN_PROBE='$probe' nix shell nixpkgs#xorg.xorgserver nixpkgs#xvfb-run --command \\
    bash -c 'WINEDEBUG=-all xvfb-run -a wine "\$ROBIN_PROBE" --version' 2>/dev/null
EOF2
    ) || die 'wine-version-failed'
    ssh -n "$host" "rm -f -- '$probe'" || true
    case $rendered in
        *"$sha"*) ;;
        *) die "windows-exe-commit-mismatch:$rendered" ;;
    esac
    OBTAINED_WIN_EXE=$exe
}
obtain_windows "$short"
win_exe_local=$OBTAINED_WIN_EXE
win_exe_host=$remote_abs/robin.exe
scp -q -- "$win_exe_local" "$host:$win_exe_host" || die 'windows-exe-scp-failed'

# ---- 3. Detect a stale parity corpus and re-record every stale case.
schema=$(
    sed -n 's/.*REPLAY_SCHEMA_VERSION: u32 = \([0-9][0-9]*\).*/\1/p' \
        "$corpus_schema" | head -1
)
[ -n "$schema" ] || die 'schema-version-missing'

corpus_version() { # FILE
    python3 -B - "$1" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    first = json.loads(handle.readline())
print(first["recording"]["version"])
PY
}

stale=0
for case_dir in "$corpus"/*/; do
    [ -d "$case_dir" ] || continue
    for rec in "$case_dir"*.rhrec.jsonl; do
        [ -f "$rec" ] || continue
        recorded_version=$(corpus_version "$rec") || die "corpus-header-unreadable:$rec"
        [ "$recorded_version" = "$schema" ] || stale=1
    done
done

if (( stale )); then
    changed=()
    for case_dir in "$corpus"/*/; do
        [ -d "$case_dir" ] || continue
        case_name=${case_dir%/}; case_name=${case_name##*/}
        needs=0
        for rec in "$case_dir"*.rhrec.jsonl; do
            [ -f "$rec" ] || continue
            [ "$(corpus_version "$rec")" = "$schema" ] || needs=1
        done
        (( needs )) || continue
        case $case_name in
            sherwood-hq-entry) mission=Sherwood; spec=$fixture_sherwood ;;
            *) die "unknown-parity-case:$case_name" ;;
        esac
        record_dir=$remote_abs/.release-evidence-record-$case_name
        ssh "$host" bash -s <<EOF
set -euo pipefail
cd -- "$remote_abs"
export ROBINHOOD_DATA_DIR=$data_abs
rm -rf -- "$record_dir"
mkdir -p -- "$record_dir/record"
# --spec stops at the frame budget with exit 3 by design; the recorder has
# already flushed a complete, replayable recording (parity_corpus/README.md).
set +e
"$binary" --headless --no-sound --mission "$mission" --spec "$spec" \
    --record "$record_dir/record"
rc=\$?
set -e
[ "\$rc" -eq 0 ] || [ "\$rc" -eq 3 ] || {
    printf 'release-evidence: recorder exited \$rc for $case_name\n' >&2
    exit 1
}
EOF
        scp -q -r -- "$host:$record_dir/record/." "$case_dir" ||
            die "corpus-copy-failed:$case_name"
        ssh "$host" "rm -rf -- '$record_dir'" || die 'corpus-cleanup-failed'
        found=0
        for rec in "$case_dir"*.rhrec.jsonl; do
            [ -f "$rec" ] || continue
            [ "$(corpus_version "$rec")" = "$schema" ] ||
                die "corpus-rerecord-schema-wrong:$rec"
            changed+=("$rec")
            found=1
        done
        [ -f "$case_dir/mission.json" ] || die "corpus-rerecord-missing-chronology:$case_name"
        changed+=("$case_dir/mission.json")
        (( found )) || die "corpus-rerecord-empty:$case_name"
    done
    (( ${#changed[@]} > 0 )) || die 'corpus-stale-but-no-cases'
    git add -- "${changed[@]}" || die 'corpus-stage-failed'
    git commit -m "validation: re-record parity corpus at replay schema $schema" \
        -- "${changed[@]}" || die 'corpus-commit-failed'
    # The corpus commit moves HEAD; the binary and the gate must name the tree
    # that is actually being released. Only the non-Rust corpus files changed,
    # so rebuilding is provenance maintenance, not a new release mechanism.
    gate_candidate=$(git rev-parse HEAD)
    gate_short=${gate_candidate:0:12}
    if ! build_linux "$gate_short" >>"$gate_log" 2>&1; then
        cat -- "$gate_log" >&2
        die 'linux-rebuild-failed'
    fi
fi

# ---- 4. Run parity-cross with all six PARITY_* variables forwarded.
if command -v runbg >/dev/null && [ -z "${ROBIN_RELEASE_FOREGROUND:-}" ]; then
    runbg_name=release-parity-$short
    runbg run "$runbg_name" -- \
        env SCCACHE_DISABLE=1 \
        PARITY_LINUX_BIN="$binary" \
        PARITY_WINDOWS_EXE="$win_exe_host" \
        PARITY_DATA_DIR="$data_abs" \
        PARITY_LINUX_DIR="$remote_abs/target/release" \
        PARITY_WINDOWS_DIR="$remote_abs" \
        PARITY_REPLAY= \
        bash "$scripts/remote-check.sh" parity-cross "$copy_name" \
        >"$parity_log" 2>&1 || die 'parity-launch-failed'
    if ! runbg wait "$runbg_name" >>"$parity_log" 2>&1; then
        runbg log "$runbg_name" >&2 || true
        die 'parity-cross-red'
    fi
    runbg log "$runbg_name" >"$parity_log" || die 'parity-log-missing'
else
    env SCCACHE_DISABLE=1 \
        PARITY_LINUX_BIN="$binary" \
        PARITY_WINDOWS_EXE="$win_exe_host" \
        PARITY_DATA_DIR="$data_abs" \
        PARITY_LINUX_DIR="$remote_abs/target/release" \
        PARITY_WINDOWS_DIR="$remote_abs" \
        PARITY_REPLAY= \
        bash "$scripts/remote-check.sh" parity-cross "$copy_name" \
        >"$parity_log" 2>&1 || {
            cat -- "$parity_log" >&2
            die 'parity-cross-red'
        }
fi
# Exit zero alone is not green: the log must show every tracked recording was
# replayed and carry none of the known red markers.
green=1
for case_dir in "$corpus"/*/; do
    [ -d "$case_dir" ] || continue
    found=0
    for rec in "$case_dir"*.rhrec.jsonl; do
        [ -f "$rec" ] || continue
        if grep -qF -- "${rec##*/}" "$parity_log"; then found=1; fi
    done
    (( found )) || green=0
done
if grep -qE 'FAIL|unsupported replay schema|hash chain|Traceback' "$parity_log"; then
    green=0
fi
(( green )) || {
    cat -- "$parity_log" >&2
    die 'parity-cross-not-green'
}

# ---- 5. Release gate, summary fetch, generated note and its commit.
gate_short=${gate_candidate:0:12}
evidence=${ROBIN_E2E_EVIDENCE:-"~/e2e-gate/release-$gate_short"}
case $evidence in '~/'* | /*) ;; *) die "evidence-bad-path:$evidence" ;; esac
case $evidence in *[!A-Za-z0-9._/~+-]*) die "evidence-bad-path:$evidence" ;; esac
if ! ROBIN_E2E_EVIDENCE="$evidence" \
    bash "$scripts/release-gate.sh" "$gate_candidate" "$binary" \
    >"$gate_log" 2>&1; then
    cat -- "$gate_log" >&2
    die 'release-gate-refused'
fi

ssh "$host" "cat -- $evidence/summary.json" >"$summary_file" ||
    die 'summary-fetch-failed'
jq -e '.verdict == "observed"' "$summary_file" >/dev/null 2>&1 ||
    die 'gate-verdict-not-observed'
jq -e '.verdicts.H23 == "observed"' "$summary_file" >/dev/null 2>&1 ||
    die 'gate-h23-not-observed'
expected_rows=$(
    python3 -B - "$repository/scripts/validation" <<'PY'
import json
import sys
sys.path.insert(0, sys.argv[1])
import scenario_corpus
_, scenarios = scenario_corpus.load_corpus()
rows = sorted(identifier for identifier, (_, document) in scenarios.items()
              if (document.get("proves") or {}).get("layer") == "process")
if not rows:
    print("no process-layer rows", file=sys.stderr)
    raise SystemExit(1)
print(json.dumps(rows))
PY
) || die 'gate-process-rows-unavailable'
jq -e --argjson expected "$expected_rows" \
    '. as $s | all($expected[]; . as $row | (($s.selected // []) | index($row)) != null)' \
    "$summary_file" >/dev/null 2>&1 || die 'gate-selected-incomplete'

if ! bash "$scripts/release-notes.sh" "$summary_file" "$version" >/dev/null; then
    die 'release-notes-failed'
fi
note=docs/releases/$version.md
[ -s "$note" ] || die 'release-note-missing'
git add -- "$note" || die 'release-note-stage-failed'
git commit -m "docs: add $version release evidence" -- "$note" ||
    die 'release-note-commit-failed'
final_candidate=$(git rev-parse HEAD)

# ---- 6. Machine-readable outputs consumed by autonomy-driver.sh.
printf 'ROBIN_RELEASE_BINARY=%s\n' "$binary"
printf 'ROBIN_RELEASE_SUMMARY=%s\n' "$summary_file"
printf 'ROBIN_RELEASE_CANDIDATE=%s\n' "$final_candidate"
printf 'VERDICT observed candidate=%s version=%s\n' "$final_candidate" "$version"
