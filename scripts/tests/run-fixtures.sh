#!/usr/bin/env bash
# Fixture tests for scripts/run-fixtures.sh: change-based selection, concurrency
# and failure reporting, against synthetic fixtures.
set -euo pipefail
source_root=$(cd -- "$(dirname -- "$0")/../.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-run-fixtures.XXXXXX")
trap 'rm -rf -- "$work"' EXIT
mkdir -p "$work/scripts/sub"
cp -- "$source_root/scripts/run-fixtures.sh" "$work/scripts/"
printf 'cp -- scripts/land.sh x\necho land-fixture\n' >"$work/scripts/test_land.sh"
printf 'echo other-fixture\n' >"$work/scripts/test_other.sh"
printf 'exit 1\n' >"$work/scripts/test_broken.sh"
printf 'import unittest\nclass T(unittest.TestCase):\n    def test_ok(self):\n        pass\n' >"$work/scripts/sub/case_test.py"
failures=0
pass() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; failures=$((failures + 1)); }
run() { status=0; output=$(cd "$work" && env "$@" bash scripts/run-fixtures.sh \
    'bash scripts/test_land.sh' 'bash scripts/test_other.sh' \
    'python3 -m unittest discover -s scripts/sub -p case_test.py' 2>&1) || status=$?; }

run ROBIN_CHANGED_PATHS=scripts/land.sh
[[ $output == *'1 of 3 fixtures affected'* && $output == *land-fixture* && $output != *other-fixture* ]] &&
    pass 'a changed script runs only the fixtures that name it' || fail "$output"
run ROBIN_CHANGED_PATHS=scripts/sub/case_test.py
[[ $output == *'1 of 3 fixtures affected'* ]] && pass 'a discover fixture is its -s/-p file' || fail "$output"
run X=1
[[ $output == *'3 of 3 fixtures affected'* && $status -eq 0 ]] &&
    pass 'without a diff every fixture runs' || fail "$output"
status=0
output=$(cd "$work" && bash scripts/run-fixtures.sh 'bash scripts/test_broken.sh' 'bash scripts/test_other.sh' 2>&1) || status=$?
[ "$status" -eq 1 ] && [[ $output == *'failed: bash scripts/test_broken.sh'* ]] &&
    pass 'a failing fixture fails the run and is named' || fail "exit $status: $output"

printf '\n%s checks failed\n' "$failures"
(( failures == 0 ))
