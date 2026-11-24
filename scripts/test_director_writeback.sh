#!/usr/bin/env bash
# Fixture tests for the terminal write-back: stub gh, no network.
set -euo pipefail
source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-writeback.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

stubs=$work/stubs
mkdir -p "$stubs"
cat >"$stubs/gh" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$GH_LOG"
case "$1 $2" in
    'issue comment') printf 'comment ok\n' ;;
esac
STUB
chmod +x "$stubs/gh"

handoff=$work/handoff.json
printf '{"selected_item":"D12 / #83 teleport desync","state":"pending"}\n' >"$handoff"
export PATH="$stubs:$PATH" GH_LOG=$work/gh.log ROBIN_HANDOFF=$handoff
: >"$GH_LOG"

fail=0
pass() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1" >&2; fail=1; }

bash "$source_root/scripts/director-writeback.sh" LANDED 2 b5e955bf 'engine pass' done >/dev/null
grep -q 'comment 83' "$GH_LOG" && grep -q 'remove-label ready-for-agent' "$GH_LOG" \
    && pass 'LANDED comments the issue and clears admission labels' \
    || bad 'LANDED write-back missed comment or label change'
grep -q 'b5e955bf' "$GH_LOG" && grep -q 'LANDED' "$GH_LOG" \
    && pass 'LANDED comment carries the commit and state' \
    || bad 'LANDED comment lacks evidence'

bash "$source_root/scripts/director-writeback.sh" BLOCKED 2 none none quota-exhausted >/dev/null
grep -q 'add-label ready-for-human' "$GH_LOG" \
    && pass 'BLOCKED escalates to a human label' \
    || bad 'BLOCKED did not escalate'

printf '{"selected_item":"no issue here","state":"pending"}\n' >"$handoff"
if bash "$source_root/scripts/director-writeback.sh" FILED 1 none none done >/dev/null 2>&1; then
    bad 'missing issue number did not fail closed'
else
    pass 'missing issue number fails closed'
fi

(( fail == 0 )) || exit 1
printf 'director-writeback: OK\n'
