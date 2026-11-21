#!/usr/bin/env bash
# Fixture tests for scripts/release-gate.sh. ssh and jq are stubbed and a
# fixture summary.json stands in for the build host's evidence: no network, no
# build host, no suite run. The process-row list comes from a tiny corpus in a
# synthetic repo, so a truncated `.selected` is a real failure here.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-release-gate.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

export HOME=$work/home
export GIT_CONFIG_GLOBAL=$work/gitconfig
export GIT_CONFIG_NOSYSTEM=1
export PYTHONDONTWRITEBYTECODE=1
mkdir -p "$HOME"
git config --file "$GIT_CONFIG_GLOBAL" user.name Fixture
git config --file "$GIT_CONFIG_GLOBAL" user.email fixture@example.invalid
git config --file "$GIT_CONFIG_GLOBAL" init.defaultBranch main
git config --file "$GIT_CONFIG_GLOBAL" commit.gpgsign false

real_jq=$(command -v jq)
stubs=$work/stubs
mkdir -p "$stubs"

# ssh either streams the fixture summary (the gate's fetch call) or drains the
# rendered preflight/suite heredoc and reports a controllable suite status.
cat >"$stubs/ssh" <<'STUB'
#!/usr/bin/env bash
no_stdin=0
while [[ $# -gt 0 ]]; do
    case $1 in
        -n) no_stdin=1; shift ;;
        -o) shift 2 ;;
        -*) shift ;;
        *) break ;;
    esac
done
host=$1; shift
if [[ ${1:-} == cat\ * ]]; then
    exec cat "$FAKE_SUMMARY_FILE"
fi
printf '%s\n' "$*" >>"$FAKE_CALLS"
# Real ssh -n reopens its stdin from /dev/null, swallowing a heredoc. Emulate
# that here: a gate that passes -n alongside a heredoc loses the remote script,
# so the marker the host was meant to run never reaches FAKE_REMOTE_SCRIPT.
if (( no_stdin )); then
    : >"$FAKE_REMOTE_SCRIPT"
else
    cat >"$FAKE_REMOTE_SCRIPT"
fi
printf '%s\n' "${FAKE_SUITE_OUTPUT:-}"
exit "${FAKE_SUITE_EXIT:-0}"
STUB

# jq still parses for real (that is the filter under test); the stub only records
# what the gate asked it to check.
cat >"$stubs/jq" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$FAKE_CALLS"
exec "$REAL_JQ" "$@"
STUB
chmod +x "$stubs/ssh" "$stubs/jq"
export REAL_JQ=$real_jq

repo=$work/repo
git init --quiet "$repo"
cd -- "$repo"
mkdir -p scripts/validation/scenarios
cp -- "$source_root/scripts/release-gate.sh" scripts/release-gate.sh
cp -- "$source_root/scripts/validation/scenario_corpus.py" scripts/validation/scenario_corpus.py
printf '{}\n' >scripts/validation/scenarios/defaults.json
for row in H01 H02 H23; do
    printf '{"proves":{"layer":"process"}}\n' >"scripts/validation/scenarios/$row.json"
done
printf '{"proves":{"layer":"core"}}\n' >scripts/validation/scenarios/H99.json
printf 'base\n' >file.txt
git add .
git commit --quiet -m base
head=$(git rev-parse HEAD)

python3 - "$work" <<'PY'
import json
import pathlib
import sys

out = pathlib.Path(sys.argv[1])
all_rows = ["H01", "H02", "H23"]
observed = {row: "observed" for row in all_rows}
cases = {
    "all": ("observed", observed, all_rows),
    # The runner would exit 1 for both of these; forcing 0 proves the summary
    # assertion is independent of the suite's exit code.
    "h23-failed": ("failed", {**observed, "H23": "failed"}, all_rows),
    "verdict-failed": ("failed", observed, all_rows),
    # `--only H23` style: every verdict is observed but the full run was not made.
    "truncated": ("observed", observed, ["H23"]),
}
for name, (verdict, verdicts, selected) in cases.items():
    document = {"suite": "multiplayer-e2e", "verdict": verdict,
                "verdicts": verdicts, "selected": selected,
                "snapshot": "0" * 12}
    (out / f"summary-{name}.json").write_text(json.dumps(document, indent=2) + "\n")
PY

pass_count=0
failures=0
pass() { printf 'ok   %s\n' "$1"; pass_count=$((pass_count + 1)); }
fail() { printf 'FAIL %s\n' "$1"; failures=$((failures + 1)); }
expect_status() {
    if [ "$status" -eq "$1" ]; then pass "$2"; else fail "$2 (exit $status, want $1): $output"; fi
}
expect_contains() {
    case $output in
        *"$2"*) pass "$1" ;;
        *) fail "$1 (missing '$2'): $output" ;;
    esac
}

run_gate() {
    local fixture=$1 suite_exit=$2 candidate=$3
    : >"$work/calls"
    status=0
    output=$(PATH="$stubs:$PATH" FAKE_CALLS="$work/calls" \
        FAKE_REMOTE_SCRIPT="$work/remote.sh" \
        FAKE_SUMMARY_FILE="$fixture" FAKE_SUITE_EXIT="$suite_exit" \
        FAKE_SUITE_OUTPUT='{"verdict":"observed"}' \
        bash scripts/release-gate.sh "$candidate" "~/target/release/robin" 2>&1) || status=$?
}

# 1. All three process rows observed, clean verdict: pass, and print the
#    persistent evidence path for the ledger.
run_gate "$work/summary-all.json" 0 "$head"
expect_status 0 'all observed passes'
expect_contains 'the pass prints the evidence path' '~/e2e-gate/release-'
expect_contains 'the pass names H23' 'H23 observed in the full run'
if grep -q -- '--argjson expected' "$work/calls"; then
    pass 'the gate checks the selected rows against the corpus'
else
    fail 'the gate did not pass the corpus rows to jq'
fi
remote=$work/remote.sh
if grep -q 'ROBIN_E2E_JOBS=2' "$remote" &&
    grep -q 'ROBIN_E2E_BINARY=' "$remote" &&
    grep -q 'ROBINHOOD_DATA_DIR=' "$remote" &&
    grep -q 'ROBIN_E2E_EVIDENCE=' "$remote" &&
    grep -q 'check-quality.sh multiplayer-e2e' "$remote" &&
    ! grep -q -- '--only' "$remote"; then
    pass 'the host run exports all four vars at jobs=2 with no --only'
else
    fail "the host run has the wrong invocation: $(cat "$remote")"
fi

# 2. H23 failed, even with a zero suite exit: the summary must still refuse it.
run_gate "$work/summary-h23-failed.json" 0 "$head"
expect_status 1 'H23 failed is refused'
expect_contains 'the failure says the run is not clean' 'not a clean full run'

# 3. Every row observed but the suite verdict failed: still refused.
run_gate "$work/summary-verdict-failed.json" 0 "$head"
expect_status 1 'a failed suite verdict is refused'

# 4. Every verdict observed but `.selected` lost the other process rows:
#    a future `--only` cannot shrink the gate into a pass.
run_gate "$work/summary-truncated.json" 0 "$head"
expect_status 1 'a truncated selected list is refused'

# 5. A green summary cannot override a failing suite exit.
run_gate "$work/summary-all.json" 1 "$head"
expect_status 1 'a failing suite exit is refused'
expect_contains 'the refusal names the failed suite' 'multiplayer-e2e failed'

# 6. The suites compile the tree the tag lands; a dirty or untracked tree is not
#    that commit.
printf 'dirty\n' >>file.txt
run_gate "$work/summary-all.json" 0 "$head"
expect_status 1 'a dirty tree is refused'
printf 'base\n' >file.txt
printf 'stray\n' >untracked.txt
run_gate "$work/summary-all.json" 0 "$head"
expect_status 1 'an untracked file is refused'
rm -- untracked.txt

# 7. HEAD must be the candidate the tag names.
git checkout --quiet -b other
git commit --quiet --allow-empty -m other
other=$(git rev-parse HEAD)
git checkout --quiet main
run_gate "$work/summary-all.json" 0 "$other"
expect_status 1 'a HEAD that is not the candidate is refused'

# 8. The host-side preflight/suite script is delivered on ssh's stdin. An
#    `ssh -n` reopens stdin from /dev/null, so the heredoc is swallowed and the
#    remote script is empty: no binary preflight, no evidence dir, no suite.
#    This guards the exact regression where the gate failed with no summary.json.
run_gate "$work/summary-all.json" 0 "$head"
if grep -q 'ROBIN_E2E_JOBS=2' "$work/remote.sh"; then
    pass 'the remote script reaches the host on stdin'
else
    fail "the remote script was not delivered on stdin: $(cat "$work/remote.sh")"
fi

printf '\n%s checks passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
