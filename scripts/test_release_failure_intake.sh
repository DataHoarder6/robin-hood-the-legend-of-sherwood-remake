#!/usr/bin/env bash
# Fixture tests for release-failure intake: stub gh, no network.
set -uo pipefail
source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-intake.XXXXXX")
trap 'rm -rf -- "$work"' EXIT
stubs=$work/stubs
mkdir -p "$stubs"
cat >"$stubs/gh" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$GH_LOG"
case "$1 $2" in
    'issue list')
        if [ -n "${EXISTING:-}" ]; then printf '%s\n' "$EXISTING"; else printf '\n'; fi
        ;;
    'issue create') printf 'https://github.com/example/issues/99\n' ;;
esac
STUB
chmod +x "$stubs/gh"
export PATH="$stubs:$PATH" GH_LOG=$work/gh.log
: >"$GH_LOG"

fail=0
pass() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1" >&2; fail=1; }

out=$(bash "$source_root/scripts/release-failure-intake.sh" release-gate-refused abc123456789 0.5.0)
[[ $out == *'issues/99'* ]] && pass 'a refusal files one issue' || bad "create failed: $out"
grep -q 'ready-for-agent' "$GH_LOG" && grep -q 'bug' "$GH_LOG" \
    && pass 'the issue is admitted and labelled as a bug' || bad 'labels missing'
grep -q 'release-gate-refused' "$GH_LOG" && grep -q 'abc123456789' "$GH_LOG" \
    && pass 'the issue carries the named reason and candidate' || bad 'evidence missing'
: >"$GH_LOG"
out=$(EXISTING=77 bash "$source_root/scripts/release-failure-intake.sh" release-gate-refused abc123456789 0.5.0)
[[ $out == *'already-filed #77'* ]] && pass 'a repeat refusal does not duplicate' || bad "duplicate not caught: $out"

(( fail == 0 )) || exit 1
printf 'release-failure-intake: OK\n'
