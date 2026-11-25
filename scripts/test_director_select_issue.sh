#!/usr/bin/env bash
# Fixture tests for machine-checked director selection: stub gh, local ledgers.
set -euo pipefail
source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
selector=$source_root/scripts/director-select-issue.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-select.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

stubs=$work/stubs
mkdir -p "$stubs"
cat >"$stubs/gh" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$GH_LOG"
if [ -n "${GH_FAIL:-}" ]; then
    printf 'stub: gh authentication failed\n' >&2
    exit 1
fi
cat "${ROBIN_ISSUES_JSON:?}"
STUB
chmod +x "$stubs/gh"

cat >"$work/defects.md" <<'DEFECTS'
| id | class | symptom | reproduction | check | status |
| --- | --- | --- | --- | --- | --- |
| D09 | release | known flake | corpus | isolated recheck | deferred |
| D14 | release | real defect | repro | named_check | open |
| D15 | data | out of scope | repro | named_check | out-of-scope |
DEFECTS
cat >"$work/gaps.md" <<'GAPS'
- Linux co-op is the target. Windows cross-play and the Windows installer
  are `HUMAN_GATE` items ([issue #56](https://example.test/issues/56)).
GAPS

issues=$work/issues.json
cat >"$issues" <<'ISSUES'
[
  {"number":49,"title":"Ultimate goal","updatedAt":"2026-11-17T00:00:00Z","labels":[{"name":"ready-for-agent"}]},
  {"number":56,"title":"Windows seat","updatedAt":"2026-11-19T00:00:00Z","labels":[{"name":"ready-for-agent"}]},
  {"number":80,"title":"Ready work","updatedAt":"2026-11-18T00:00:00Z","labels":[{"name":"ready-for-agent"}]}
]
ISSUES
jq 'map(if .number == 49 then .labels = [{"name":"enhancement"}] else . end)' "$issues" >"$work/issues-no49.json"

handoff=$work/handoff.json
template='{"revision":1,"issue":49,"assertion":null,"tree_sha":"abc123456789","attempt":0,"state":"LANDED","seen":{},"next_action":"recompute"}'
: >"$work/gh.log"
export PATH="$stubs:$PATH" GH_LOG="$work/gh.log" ROBIN_HANDOFF="$handoff" \
    ROBIN_ISSUES_JSON="$issues" ROBIN_DEFECTS="$work/defects.md" \
    ROBIN_GAPS="$work/gaps.md" ROBIN_INTEGRATION_SHA=abc123456789 \
    ROBIN_RELEASE_REPO=owner/repo

fail=0
pass() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1" >&2; fail=1; }

run() { bash "$selector" 2>"$work/err"; }

# Deferred and out-of-scope rows never become the selected assertion.
printf '%s\n' "$template" >"$handoff"
out=$(run) || true
[[ "$out" == 'ISSUE 49|assertion=D14|sha=abc123456789|revision=2' ]] \
    && pass 'deferred-row-is-inadmissible' \
    || bad "deferred-row-is-inadmissible: '$out' / $(cat "$work/err")"

# The HUMAN_GATE issue is skipped even though it carries the admission label.
printf '%s\n' "$template" >"$handoff"
out=$(ROBIN_ISSUES_JSON="$work/issues-no49.json" run) || true
[[ "$out" == *'ISSUE 80|assertion=ISSUE-80'* ]] \
    && pass 'human-gate-row-is-inadmissible' \
    || bad "human-gate-row-is-inadmissible: '$out'"

# A FILED assertion on the same tree is not re-selected.
printf '%s\n' '{"revision":1,"issue":49,"assertion":null,"tree_sha":"abc123456789","attempt":1,"state":"LANDED","seen":{"D14":{"state":"FILED","tree_sha":"abc123456789"}},"next_action":"recompute"}' >"$handoff"
out=$(run) || true
[[ "$out" == 'NO-PROGRESS repeat-D14' ]] \
    && pass 'same-assertion-same-sha-is-no-progress' \
    || bad "same-assertion-same-sha-is-no-progress: '$out'"

# A new integration sha reopens the assertion.
printf '%s\n' '{"revision":1,"issue":49,"assertion":null,"tree_sha":"abc123456789","attempt":1,"state":"LANDED","seen":{"D14":{"state":"FILED","tree_sha":"oldsha0000000"}},"next_action":"recompute"}' >"$handoff"
out=$(run) || true
[[ "$out" == *'assertion=D14'* ]] \
    && pass 'new-integration-sha-reopens-selection' \
    || bad "new-integration-sha-reopens-selection: '$out'"

# gh failure is a named error, never a silent fallback.
printf '%s\n' "$template" >"$handoff"
set +e
out=$(GH_FAIL=1 ROBIN_ISSUES_JSON= run)
rc=$?
set -e
[[ $rc -ne 0 && -z "$out" && "$(cat "$work/err")" == *'authentication failed'* ]] \
    && pass 'gh-failure-fails-closed' \
    || bad "gh-failure-fails-closed: rc=$rc out='$out' err='$(cat "$work/err")'"

# A row with no check cannot be classified as failing.
cat >"$work/defects.md" <<'DEFECTS2'
| id | class | symptom | reproduction | check | status |
| --- | --- | --- | --- | --- | --- |
| D16 | release | unknown check | repro |  | open |
DEFECTS2
printf '%s\n' "$template" >"$handoff"
set +e
out=$(run)
rc=$?
set -e
[[ $rc -ne 0 && "$(cat "$work/err")" == *'unknown-assertion-state'* ]] \
    && pass 'unevaluable-check-fails-closed' \
    || bad "unevaluable-check-fails-closed: rc=$rc err='$(cat "$work/err")'"

(( fail == 0 )) || exit 1
printf 'director-select-issue: OK\n'
