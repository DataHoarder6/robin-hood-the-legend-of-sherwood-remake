#!/usr/bin/env bash
# Light doc check: the new/changed doc surface resolves its relative links and
# the backlog is indexed from docs/README.md. The full-tree link audit lives in
# `scripts/check_markdown_links.py` (the `docs` suite) — this does not duplicate
# it.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"

fail=0
python3 scripts/check_markdown_links.py \
  docs/BACKLOG.md docs/README.md docs/SESSION_FINISH.md || fail=1
grep -qF 'BACKLOG.md' docs/README.md || {
  echo 'docs-check: docs/README.md does not index BACKLOG.md' >&2
  fail=1
}
exit "$fail"
