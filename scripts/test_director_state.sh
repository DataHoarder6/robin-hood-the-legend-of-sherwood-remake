#!/usr/bin/env bash
# Fixture tests for the director handoff ledger: real jq, temp files, no network.
set -euo pipefail
source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
state=$source_root/scripts/director-state.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-director-state.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

fail=0
pass() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1" >&2; fail=1; }

new_handoff() { printf '%s' "$work/handoff.json"; }

# A pending selection resumes only for the same integration tree.
h=$(new_handoff)
printf '{"revision":0,"issue":null,"assertion":null,"tree_sha":null,"attempt":0,"state":"none","seen":{},"next_action":null}\n' >"$h"
rev=$(bash "$state" claim "$h" 49 D09 abcdef123456 2>/dev/null) || true
out=$(bash "$state" resume "$h" abcdef123456 2>/dev/null) || true
[[ "$out" == 'RESUME|49|D09|'"$rev" ]] \
    && pass 'pending-handoff-resumes' \
    || bad "pending-handoff-resumes: got '$out' (revision '$rev')"

# A terminal handoff is not a resume pointer for a stale assertion.
bash "$state" finish "$h" "$rev" LANDED 'fixed' >/dev/null 2>&1 || true
out=$(bash "$state" resume "$h" abcdef123456 2>/dev/null) || true
[[ -z "$out" ]] && pass 'terminal-handoff-does-not-resume' \
    || bad "terminal-handoff-does-not-resume: got '$out'"

# Every claim increments revision and leaves valid JSON.
current=$(jq -r '.revision' "$h")
rev2=$(bash "$state" claim "$h" 80 D10 abcdef123456 2>/dev/null) || true
jq -e . "$h" >/dev/null 2>&1 \
    && [[ "$rev2" == "$((current + 1))" ]] \
    && pass 'selection-advances-revision' \
    || bad "selection-advances-revision: current=$current rev2=$rev2"

# A finish against an older revision is refused; the ledger stays unchanged.
if bash "$state" finish "$h" "$rev" FILED 'stale' >/dev/null 2>&1; then
    bad 'finish-refuses-stale-revision'
else
    jq -e --argjson rev "$rev2" '.revision == $rev and .seen.D10 == null' "$h" >/dev/null 2>&1 \
        && pass 'finish-refuses-stale-revision' \
        || bad "finish-refuses-stale-revision: $(cat "$h")"
fi

# A valid finish records the terminal state in the seen ledger.
bash "$state" finish "$h" "$rev2" FILED 'known-flake' >/dev/null 2>&1 || true
recorded=$(jq -r '.seen.D10.state // empty' "$h" 2>/dev/null || true)
[[ "$recorded" == FILED ]] \
    && pass 'finish-records-seen-ledger' \
    || bad "finish-records-seen-ledger: got '$recorded'"

# Concurrent writers cannot corrupt the file.
h2=$work/concurrent.json
printf '{"revision":0,"issue":null,"assertion":null,"tree_sha":null,"attempt":0,"state":"none","seen":{},"next_action":null}\n' >"$h2"
for i in 1 2 3 4 5; do
    bash "$state" claim "$h2" "$i" "D0$i" deadbeef0000 >/dev/null 2>&1 &
done
wait
jq -e '.revision == 5' "$h2" >/dev/null 2>&1 \
    && pass 'concurrent-writers-leave-valid-json' \
    || bad "concurrent-writers-leave-valid-json: $(cat "$h2")"

(( fail == 0 )) || exit 1
printf 'director-state: OK\n'
