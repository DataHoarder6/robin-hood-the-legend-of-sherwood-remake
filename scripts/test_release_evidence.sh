#!/usr/bin/env bash
# Fixture tests for scripts/release-evidence.sh. No build host, no workflow,
# no wine, no gate: ssh/scp/gh/wine/runbg and the repo helper scripts are stubbed
# locally. A real git repo supplies the candidate, the corpus and the commits.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-release-evidence.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

export HOME=$work/home
export GIT_CONFIG_GLOBAL=$work/gitconfig
export GIT_CONFIG_NOSYSTEM=1
mkdir -p "$HOME"
git config --file "$GIT_CONFIG_GLOBAL" user.name Fixture
git config --file "$GIT_CONFIG_GLOBAL" user.email fixture@example.invalid
git config --file "$GIT_CONFIG_GLOBAL" init.defaultBranch main
git config --file "$GIT_CONFIG_GLOBAL" commit.gpgsign false

stubs=$work/stubs
mkdir -p "$stubs"

# ssh runs the remote scripts here. It serves HOME and the gate summary, and
# emulates the two heredocs release-evidence sends: the release build and the
# corpus re-record. Everything it does is observable through SSH_CALLS.
cat >"$stubs/ssh" <<'STUB'
#!/usr/bin/env bash
while [[ $# -gt 0 ]]; do
    case $1 in -o) shift 2 ;; -*) shift ;; *) break ;; esac
done
host=$1; shift
printf '%s host=%s args=%s\n' "$(date +%s)" "$host" "$*" >>"$SSH_CALLS"
case $* in
    printf\ *)
        printf '%s' "$HOME"
        exit 0
        ;;
    cat\ --*)
        cat "$FAKE_SUMMARY_FILE"
        exit 0
        ;;
    rm\ -rf\ *)
        exit 0
        ;;
    wine\ *)
        printf 'robin 0.5.0+%s\n' "${FAKE_WIN_SHA:-$FAKE_CANDIDATE_SHORT}"
        exit 0
        ;;
    rm\ -f\ *)
        exit 0
        ;;
    command\ -v\ wine\ *)
        exit 0
        ;;
esac
script=$FIXTURE_WORK/ssh-script.$$
cat >"$script"
printf '%s\n' '--- rendered remote script ---' >>"$SSH_CALLS"
cat "$script" >>"$SSH_CALLS"
if grep -q 'cargo build' "$script"; then
    if [[ -n ${FAKE_BUILD_FAIL:-} ]]; then
        printf 'fixture build failure\n' >&2
        exit 1
    fi
    sha=$(grep -o '\*[0-9a-f]\{12\}\*)' "$script" | head -1 | tr -d '*)')
    bin=$(sed -n 's/^bin="\(.*\)"$/\1/p' "$script" | head -1)
    mkdir -p "$(dirname "$bin")"
    printf '#!/usr/bin/env bash\nprintf "robin 0.5.0+%s\\n" "%s"\n' "$sha" "$sha" >"$bin"
    chmod +x "$bin"
    exit 0
fi
if grep -q -- '--record' "$script"; then
    record=$(sed -n 's/^mkdir -p -- "\(.*\)\/record"$/\1/p' "$script" | head -1)
    mkdir -p "$record/record"
    printf '{"recording":{"version":%s,"mission_id":"Sherwood"},"chunk":{"file":"00000000.rhrec.jsonl"}}\n' \
        "${FAKE_SCHEMA:-80}" >"$record/record/00000000.rhrec.jsonl"
    printf '{"version":1,"chunks":[{"file":"00000000.rhrec.jsonl","previous":null,"first_ordinal":0,"loaded_save":null}]}\n' \
        >"$record/record/mission.json"
    exit 0
fi
STUB

# scp maps host: paths into the emulated host home, in both directions.
cat >"$stubs/scp" <<'STUB'
#!/usr/bin/env bash
args=()
for arg in "$@"; do [[ $arg == -* ]] || args+=("$arg"); done
src=${args[${#args[@]}-2]}
dst=${args[${#args[@]}-1]}
src=${src#minipc:}; dst=${dst#minipc:}
src=${src/#\~/$HOME}; dst=${dst/#\~/$HOME}
src=${src%/.}
if [[ $dst == */ ]]; then
    mkdir -p "$dst"
    cp -a "$src"/. "$dst"/
else
    mkdir -p "$(dirname "$dst")"
    cp -a "$src" "$dst"
fi
STUB

cat >"$stubs/gh" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$GH_CALLS"
case ${1:-} in
    workflow) exit 0 ;;
    run)
        case ${2:-} in
            list) printf '12345\n' ;;
            view) printf 'completed success\n' ;;
            download)
                dir=''
                while [[ $# -gt 0 ]]; do
                    if [[ $1 == --dir ]]; then dir=$2; shift 2; else shift; fi
                done
                mkdir -p "$dir"
                printf '#!/usr/bin/env bash\n' >"$dir/robin.exe"
                chmod +x "$dir/robin.exe"
                ;;
        esac
        exit 0
        ;;
esac
STUB

cat >"$stubs/wine" <<'STUB'
#!/usr/bin/env bash
printf 'robin 0.5.0+%s\n' "${FAKE_WIN_SHA:-$FAKE_CANDIDATE_SHORT}"
STUB

# runbg really runs the wrapped command so the parity branch is exercised.
cat >"$stubs/runbg" <<'STUB'
#!/usr/bin/env bash
action=$1; name=$2; shift 2
case $action in
    run)
        while [[ $# -gt 0 && $1 != -- ]]; do shift; done
        shift
        "$@" >"$FIXTURE_WORK/runbg-$name.log" 2>&1
        printf '%s\n' "$?" >"$FIXTURE_WORK/runbg-$name.status"
        exit 0
        ;;
    wait) exit "$(cat "$FIXTURE_WORK/runbg-$name.status")" ;;
    log) cat "$FIXTURE_WORK/runbg-$name.log" ;;
esac
STUB

for stub in ssh scp gh wine runbg; do chmod +x "$stubs/$stub"; done

repo=$work/repo
mkdir -p "$repo/scripts/validation/scenarios" \
    "$repo/scripts/validation/parity_corpus/sherwood-hq-entry" \
    "$repo/crates/robin_rs" \
    "$repo/crates/robin_rs/tests/fixtures" \
    "$repo/crates/robin_run_types/src"
cp -- "$source_root/scripts/release-evidence.sh" "$repo/scripts/release-evidence.sh"
cp -- "$source_root/scripts/validation/scenario_corpus.py" "$repo/scripts/validation/scenario_corpus.py"
printf '{}\n' >"$repo/scripts/validation/scenarios/defaults.json"
for row in H01 H23; do
    printf '{"proves":{"layer":"process"}}\n' >"$repo/scripts/validation/scenarios/$row.json"
done
printf '[package]\nname = "robin_rs"\nversion = "0.5.0"\n' >"$repo/crates/robin_rs/Cargo.toml"
printf '{}\n' >"$repo/crates/robin_rs/tests/fixtures/scenario_spec_sherwood_hq.json"
printf 'pub const REPLAY_SCHEMA_VERSION: u32 = 80;\n' \
    >"$repo/crates/robin_run_types/src/lib.rs"

cat >"$repo/scripts/remote-check.sh" <<'RC'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$RC_CALLS"
if [[ ${1:-} == --sync ]]; then
    printf '~/build/robin/release-evidence-fixture\n'
    exit 0
fi
for var in PARITY_LINUX_BIN PARITY_WINDOWS_EXE PARITY_DATA_DIR \
    PARITY_LINUX_DIR PARITY_WINDOWS_DIR PARITY_REPLAY; do
    printf '%s=%s\n' "$var" "${!var-}" >>"$PARITY_CALLS"
done
[[ -n ${FAKE_PARITY_FAIL:-} ]] && exit 1
while IFS= read -r -d '' rec; do printf 'parity-cross: %s\n' "${rec##*/}"; done \
    < <(find scripts/validation/parity_corpus -name '*.rhrec.jsonl' -print0)
RC

cat >"$repo/scripts/release-gate.sh" <<'GATE'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$GATE_CALLS"
[[ -n ${FAKE_GATE_FAIL:-} ]] && exit 1
cat >"$FAKE_SUMMARY_FILE" <<'JSON'
{"suite":"multiplayer-e2e","layer":"process","verdict":"observed",
 "verdicts":{"H01":"observed","H23":"observed"},"selected":["H01","H23"],
 "snapshot":"0123456789ab","binary_sha256":"abc","refused":[],
 "harness_gaps":[],"elapsed_s":1,"scenarios":{"H23":{"evidence":"~/e2e/H23"}}}
JSON
printf 'release-gate: H23 observed in the full run\n'
GATE

cat >"$repo/scripts/release-notes.sh" <<'NOTES'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$NOTES_CALLS"
version=$2
mkdir -p docs/releases
printf '# %s release evidence\nGenerated from summary.json.\n' "$version" \
    >"docs/releases/$version.md"
NOTES
chmod +x "$repo/scripts/"*.sh

write_corpus() { # VERSION
    local rec=$repo/scripts/validation/parity_corpus/sherwood-hq-entry/00000000.rhrec.jsonl
    printf '{"recording":{"version":%s,"mission_id":"Sherwood"},"chunk":{"file":"00000000.rhrec.jsonl"}}\n' \
        "$1" >"$rec"
    printf '{"version":1,"chunks":[{"file":"00000000.rhrec.jsonl","previous":null,"first_ordinal":0,"loaded_save":null}]}\n' \
        >"${rec%/*}/mission.json"
}
write_corpus 78

cd -- "$repo"
git init --quiet -b main .
git add .
git commit --quiet -m base
head=$(git rev-parse HEAD)
candidate_short=${head:0:12}
cp -a -- "$repo" "$work/template"
mkdir -p "$work/no-gh-bin"
for tool in bash git jq python3 awk sed grep find head mktemp date sleep dirname cat rm tr wc sort; do
    tool_path=$(command -v "$tool") || continue
    ln -sf "$tool_path" "$work/no-gh-bin/$tool"
done
for stub in ssh scp wine runbg; do ln -sf "$stubs/$stub" "$work/no-gh-bin/$stub"; done

export SSH_CALLS=$work/ssh-calls GH_CALLS=$work/gh-calls RC_CALLS=$work/rc-calls
export FIXTURE_WORK=$work
export PARITY_CALLS=$work/parity-calls GATE_CALLS=$work/gate-calls NOTES_CALLS=$work/notes-calls
export FAKE_SUMMARY_FILE=$work/summary.json
export FAKE_CANDIDATE_SHORT=$candidate_short
export FAKE_SCHEMA=80
export ROBIN_BUILD_HOST=minipc
export PATH="$stubs:$PATH"

pass_count=0
failures=0
pass() { printf 'ok   %s\n' "$1"; pass_count=$((pass_count + 1)); }
fail() { printf 'FAIL %s\n' "$1"; failures=$((failures + 1)); }
expect_status() {
    if [ "$status" -eq "$1" ]; then pass "$2"; else fail "$2 (exit $status, want $1): $output"; fi
}
expect_contains() {
    case $output in *"$2"*) pass "$1" ;; *) fail "$1 (missing '$2'): $output" ;; esac
}
fixture() {
    : >"$SSH_CALLS"; : >"$GH_CALLS"; : >"$RC_CALLS"; : >"$PARITY_CALLS"
    : >"$GATE_CALLS"; : >"$NOTES_CALLS"
    unset FAKE_BUILD_FAIL FAKE_GATE_FAIL FAKE_PARITY_FAIL FAKE_WIN_SHA FAKE_CORPUS_VERSION
    rm -rf -- "$repo"
    cp -a -- "$work/template" "$repo"
    cd -- "$repo"
    write_corpus "${1:-78}"
}
run_evidence() {
    status=0
    output=$(cd "$repo" && bash scripts/release-evidence.sh "$@" 2>&1) || status=$?
}
corpus_version() {
    python3 -B - "$1" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    print(json.loads(handle.readline())["recording"]["version"])
PY
}
recording=$repo/scripts/validation/parity_corpus/sherwood-hq-entry/00000000.rhrec.jsonl

# 1. Missing and non-commit candidates fail closed with a named reason.
fixture
run_evidence
expect_status 2 'no candidate is a usage error'
run_evidence not-a-commit-sha
expect_status 1 'a non-commit is refused'
expect_contains 'the reason is named' 'not-a-commit'

# 2. gh unavailable is a named refusal, never a silent skip.
fixture
real_bash=$(command -v bash)
output=$(cd "$repo" && env PATH="$work/no-gh-bin" \
    "$real_bash" scripts/release-evidence.sh "$head" 0.5.0 2>&1) || status=$?
expect_status 1 'a missing gh is refused'
expect_contains 'the capability is named' 'gh-unavailable'
export PATH="$stubs:$PATH"

# 3. Stale corpus is detected, re-recorded, committed and the release succeeds.
fixture 78
run_evidence "$head" 0.5.0
expect_status 0 'a stale corpus is repaired and the evidence succeeds'
expect_contains 'binary path is printed' 'ROBIN_RELEASE_BINARY='
expect_contains 'summary path is printed' 'ROBIN_RELEASE_SUMMARY='
expect_contains 'candidate is printed' 'ROBIN_RELEASE_CANDIDATE='
expect_contains 'verdict is printed' 'VERDICT observed'
[ "$(corpus_version "$recording")" = 80 ] && pass 'the corpus was re-recorded at schema 80' || fail 'corpus is still stale'
if git log --oneline -n 1 -- scripts/validation/parity_corpus | grep -q 're-record parity corpus at replay schema 80'; then
    pass 'the corpus commit names the schema maintenance'
else
    fail 'no clear re-record commit'
fi
if git log --name-only -1 --format= -- scripts/validation/parity_corpus |
    grep -q '00000000.rhrec.jsonl'; then
    pass 'the re-recorded corpus is committed by pathspec'
else
    fail 'the corpus files were not committed'
fi
[ -f docs/releases/0.5.0.md ] && pass 'the note was generated' || fail 'no generated note'
if git log --oneline -n 1 -- docs/releases/0.5.0.md | grep -q 'add 0.5.0 release evidence'; then
    pass 'the note is committed'
else
    fail 'the note was not committed'
fi
if grep -q 'workflow run native-release.yml --ref main' "$GH_CALLS"; then
    pass 'the Windows build is dispatched from main'
else
    fail 'no native-release dispatch'
fi
if [ "$(grep -c '^PARITY_' "$PARITY_CALLS")" -eq 6 ]; then
    pass 'all six PARITY_* variables reach parity-cross'
else
    fail "only $(grep -c '^PARITY_' "$PARITY_CALLS") parity variables were forwarded"
fi
if grep -q 'SCCACHE_DISABLE=1' "$SSH_CALLS"; then
    pass 'the Linux build disables sccache'
else
    fail 'SCCACHE_DISABLE is not set for the build'
fi
if grep -q 'nix develop --command cargo build' "$SSH_CALLS"; then
    pass 'the build uses the devShell for LIBCLANG_PATH'
else
    fail 'the build does not run inside the devShell'
fi
if grep -E '^[^#]*(--target-dir|CARGO_TARGET_DIR=)' "$SSH_CALLS" >/dev/null; then
    fail 'a target directory override leaked into the build'
else
    pass 'no target directory override'
fi

# 4. No double tag: release-evidence never tags or pushes a tag itself.
if [ -z "$(git tag --list 'v*')" ] && ! grep -qE 'git (tag|push)' "$SSH_CALLS"; then
    pass 'no tag or tag push is created by release-evidence'
else
    fail 'release-evidence created a tag'
fi

# 5. A refused gate names the reason and generates no note.
fixture 80
rm -f -- docs/releases/0.5.0.md
FAKE_GATE_FAIL=1 run_evidence "$head" 0.5.0
expect_status 1 'a refused gate stops the release'
expect_contains 'the gate reason is named' 'release-gate-refused'
[ -f docs/releases/0.5.0.md ] && fail 'a note was written after a refused gate' || pass 'no note after a refused gate'

# 6. A mismatched Windows exe is refused (no shipped asset from another sha).
fixture 80
FAKE_WIN_SHA=deadbeefdead run_evidence "$head" 0.5.0
expect_status 1 'a wrong-sha Windows exe is refused'
expect_contains 'the mismatch is named' 'windows-exe-commit-mismatch'

# 7. A red parity-cross stops the release before the gate.
fixture 80
: >"$GATE_CALLS"
FAKE_PARITY_FAIL=1 run_evidence "$head" 0.5.0
expect_status 1 'a red parity suite stops the release'
expect_contains 'the parity reason is named' 'parity-cross-red'
[ -s "$GATE_CALLS" ] && fail 'the gate ran after red parity' || pass 'the gate is skipped after red parity'

printf '\n%s checks passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
