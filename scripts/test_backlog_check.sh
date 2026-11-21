#!/usr/bin/env bash
# Fixture test for scripts/backlog-check.sh. No live doc, DB or worktree is
# touched: the seam variables point at a temp doc and a temp SQLite DB.
set -uo pipefail

repo=$(git rev-parse --show-toplevel)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/backlog-check-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

cat > "$tmp/ok.md" <<'EOF'
# Deferred work backlog

## Rotating map

- Target release: 0.5.0

<!-- backlog: key=fixture-rotation live=yes -->

## Fighting mechanics

- Target release: later patch

<!-- backlog: key=fighting-mechanics live=no -->
EOF

cat > "$tmp/orphan.md" <<'EOF'
# Deferred work backlog

## Rotating map

- Target release: 0.5.0

<!-- backlog: key=fixture-rotation live=yes -->
EOF

make_db() {
  local db=$1
  sqlite3 "$db" 'CREATE TABLE session_v2 (title TEXT);'
}

fails=0
check() {
  local label=$1 expect=$2 needle=$3 doc=$4 db=$5
  local out rc
  out=$(BACKLOG_REPO="$repo" BACKLOG_DOC="$doc" BACKLOG_SESSION_DB="$db" \
    bash "$repo/scripts/backlog-check.sh" 2>&1); rc=$?
  printf '%s ... ' "$label"
  if [[ $rc -ne $expect ]]; then
    printf 'FAIL (exit %s, wanted %s)\n' "$rc" "$expect"
    printf '%s\n' "$out"
    fails=$((fails + 1))
    return
  fi
  if ! printf '%s' "$out" | grep -qF -- "$needle"; then
    printf 'FAIL (missing: %s)\n' "$needle"
    printf '%s\n' "$out"
    fails=$((fails + 1))
    return
  fi
  echo OK
}

# A matching frozen session and a missing worktree: pass with a warning.
make_db "$tmp/match.db"
sqlite3 "$tmp/match.db" "INSERT INTO session_v2 VALUES ('[FROZEN] Rotating map — backlog 0.5.0');"
check 'matching session, absent worktree warns' 0 \
  "no worktree for 'fixture-rotation'" "$tmp/ok.md" "$tmp/match.db"

# An orphan frozen session is a FAIL; a planning-only entry needs no worktree.
make_db "$tmp/orphan.db"
sqlite3 "$tmp/orphan.db" "INSERT INTO session_v2 VALUES ('[FROZEN] Rotating map — backlog 0.5.0'), ('[FROZEN] Unknown feature — backlog 9.9.9');"
check 'orphan frozen session fails' 1 \
  "no BACKLOG entry: '[FROZEN] Unknown feature" "$tmp/ok.md" "$tmp/orphan.db"

# An entry whose worktree is gone is a warning, never a failure.
make_db "$tmp/single.db"
sqlite3 "$tmp/single.db" "INSERT INTO session_v2 VALUES ('[FROZEN] Rotating map — backlog 0.5.0');"
check 'missing worktree does not fail on its own' 0 \
  'backlog-check: fails=0 warns=2' "$tmp/orphan.md" "$tmp/single.db"

if [[ $fails -ne 0 ]]; then
  echo "test_backlog_check: $fails failure(s)"
  exit 1
fi
echo 'test_backlog_check: OK'
