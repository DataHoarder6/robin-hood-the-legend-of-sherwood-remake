#!/usr/bin/env bash
# Light validation entry point for doc/backlog invariants. The Rust gates are
# owned by `scripts/check-quality.sh`; this is not a substitute for them.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"

fail=0
bash scripts/docs-check.sh || fail=1
bash scripts/backlog-check.sh || fail=1
bash -n scripts/backlog-check.sh scripts/docs-check.sh scripts/validate.sh || fail=1
exit "$fail"
