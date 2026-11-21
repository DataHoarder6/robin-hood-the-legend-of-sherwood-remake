#!/usr/bin/env bash
# The runtime is not a build host: quality/gate entry points must refuse to run
# there and name the build-host driver. No cargo, no ssh — host detection is
# injected through ROBIN_TEST_HOSTNAME so the fixture runs anywhere.
set -euo pipefail

repository=$(cd -- "$(dirname -- "$0")/../.." && pwd -P)
quality="$repository/scripts/check-quality.sh"

pass_count=0
failures=0
pass() { printf 'ok   %s\n' "$1"; pass_count=$((pass_count + 1)); }
fail() {
    printf 'FAIL %s\n' "$1"
    failures=$((failures + 1))
}

run() {
    local host=$1
    shift
    ROBIN_TEST_HOSTNAME=$host bash "$quality" "$@" 2>&1
}

# --- oci-vm is refused, with the build-host entry point in the message --------
set +e
out=$(run oci-vm --help)
rc=$?
set -e
if (( rc != 0 )) && [[ "$out" == *'scripts/remote-check.sh SUITE'* ]]; then
    pass "a quality entry point on oci-vm refuses and names remote-check.sh"
else
    fail "oci-vm: rc=$rc, output: $out"
fi

# --- the build host proceeds to suite dispatch --------------------------------
set +e
out=$(run minipc --help)
rc=$?
set -e
if [[ "$out" == *'unknown quality suite: --help'* ]] && [[ "$out" != *'remote-check.sh'* ]]; then
    pass "a quality entry point on the build host reaches suite dispatch"
else
    fail "minipc: rc=$rc, output: $out"
fi

# --- the no-argument suite list still works on the runtime --------------------
# remote-check.sh calls check-quality.sh with no arguments to validate suite
# names before it ever sshs; the listing must not be guarded.
set +e
out=$(run oci-vm)
rc=$?
set -e
if (( rc != 0 )) && [[ "$out" == *'suites: '* ]] && [[ "$out" != *'remote-check.sh'* ]]; then
    pass "the local suite listing is available on oci-vm for remote-check.sh"
else
    fail "suite listing on oci-vm: rc=$rc, output: $out"
fi

printf '\n%s passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
