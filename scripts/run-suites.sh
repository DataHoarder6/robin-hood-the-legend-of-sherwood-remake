#!/usr/bin/env bash
# Run the quality suites of one gate on one synced copy.
#
#   scripts/run-suites.sh SUITE...
#
# Suites that never touch Cargo's build directory (shell, Python and pnpm
# fixtures, the Markdown link check) run concurrently with the Cargo suites
# instead of waiting behind them: on 2026-11-27 `tooling` alone held a landing
# for 4m41 after every compile had finished. The Cargo suites run in order and
# stop at the first failure, since they share one target directory. Each
# concurrent suite's output is printed whole when it ends, so logs never
# interleave. The exit status is non-zero if any suite failed.
set -uo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

(( $# > 0 )) || { printf 'usage: scripts/run-suites.sh SUITE...\n' >&2; exit 2; }

independent=" tooling docs web editor "
logs=$(mktemp -d "${TMPDIR:-/tmp}/robin-suites.XXXXXX")
trap 'rm -rf -- "$logs"' EXIT

concurrent=()
concurrent_pids=()
cargo_suites=()
for suite in "$@"; do
    if [[ $independent == *" $suite "* ]]; then
        bash scripts/check-quality.sh "$suite" >"$logs/$suite.log" 2>&1 &
        concurrent+=("$suite")
        concurrent_pids+=("$!")
    else
        cargo_suites+=("$suite")
    fi
done

failed=()
for suite in "${cargo_suites[@]}"; do
    if ! bash scripts/check-quality.sh "$suite"; then
        failed+=("$suite")
        break
    fi
done

for index in "${!concurrent[@]}"; do
    suite=${concurrent[$index]}
    if wait "${concurrent_pids[$index]}"; then
        result=passed
    else
        result=FAILED
        failed+=("$suite")
    fi
    printf '=== %s %s (ran alongside the cargo suites)\n' "$suite" "$result"
    cat -- "$logs/$suite.log"
done

if (( ${#failed[@]} > 0 )); then
    printf 'run-suites: failed: %s\n' "${failed[*]}" >&2
    exit 1
fi
