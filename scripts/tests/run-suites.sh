#!/usr/bin/env bash
# Fixture tests for scripts/run-suites.sh against a stub check-quality.sh: no
# cargo, no build host. Proves the concurrent suites overlap the cargo suites,
# a failure in either kind fails the run, and cargo suites stop at the first
# failure.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/../.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-run-suites.XXXXXX")
trap 'rm -rf -- "$work"' EXIT
mkdir -p "$work/scripts"
cp -- "$source_root/scripts/run-suites.sh" "$work/scripts/run-suites.sh"
cat >"$work/scripts/check-quality.sh" <<'STUB'
#!/usr/bin/env bash
# tooling waits until a cargo suite has started, so it can only pass when the
# two kinds really run at the same time.
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
printf '%s\n' "$1" >>order
case $1 in
    tooling)
        for _ in $(seq 100); do [ -e lint.started ] && break; sleep 0.05; done
        [ -e lint.started ] || { echo 'tooling never overlapped lint'; exit 1; }
        echo 'tooling output' ;;
    lint) : >lint.started; sleep 0.3 ;;
    docs) [ "${FAIL_DOCS:-}" ] && exit 1 ;;
    engine) [ "${FAIL_ENGINE:-}" ] && exit 1 ;;
esac
exit 0
STUB

pass_count=0
failures=0
pass() { printf 'ok   %s\n' "$1"; pass_count=$((pass_count + 1)); }
fail() { printf 'FAIL %s\n' "$1"; failures=$((failures + 1)); }
run() { # ENV... -- SUITES...
    rm -f -- "$work/order" "$work/lint.started"
    status=0
    output=$(cd "$work" && env "$@" bash scripts/run-suites.sh $SUITES 2>&1) || status=$?
}

SUITES='tooling format lint engine client-release'
run X=1
[ "$status" -eq 0 ] && pass 'all green exits 0' || fail "all green exited $status: $output"
[[ $output == *'=== tooling passed'*'tooling output'* ]] &&
    pass 'a concurrent suite prints its whole log once it ends' ||
    fail "concurrent output missing: $output"

run FAIL_ENGINE=1
[ "$status" -eq 1 ] && pass 'a cargo suite failure fails the run' || fail "exit $status"
if grep -qx client-release "$work/order"; then
    fail 'cargo suites continued past the first failure'
else
    pass 'cargo suites stop at the first failure'
fi
[[ $output == *'=== tooling passed'* ]] &&
    pass 'concurrent suites still finish and report after a cargo failure' ||
    fail "tooling not reported: $output"

SUITES='docs format'
run FAIL_DOCS=1
[ "$status" -eq 1 ] && [[ $output == *'run-suites: failed: docs'* ]] &&
    pass 'a concurrent suite failure fails the run and is named' ||
    fail "exit $status: $output"

printf '\n%s checks passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
