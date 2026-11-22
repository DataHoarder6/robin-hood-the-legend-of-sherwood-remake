#!/usr/bin/env bash
# Fixture test for the freeze gate in scripts/backlog-check.sh: a [FROZEN]
# session with a pending/running tool call must FAIL; a files-free revert that
# drops that message (not the worktree) must make the same check PASS.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/../.." && pwd -P)
check=${CHECK_SCRIPT:-$source_root/scripts/backlog-check.sh}
sqlite_bin=${BACKLOG_SQLITE:-sqlite3}
command -v "$sqlite_bin" >/dev/null || {
    printf 'FAIL sqlite3 not available: %s\n' "$sqlite_bin" >&2
    exit 1
}

work=$(mktemp -d "${TMPDIR:-/tmp}/robin-backlog-pending.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

cat >"$work/BACKLOG.md" <<'EOF'
# Backlog fixture

## Freeze probe

- Target release: fixture
- Why deferred: fixture

<!-- backlog: key=freeze-probe live=no -->
EOF

db=$work/sessions.db
"$sqlite_bin" "$db" <<'SQL'
CREATE TABLE session_v2 (id TEXT PRIMARY KEY, title TEXT);
CREATE TABLE session_message (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    type TEXT NOT NULL,
    seq INTEGER NOT NULL,
    data TEXT NOT NULL
);
INSERT INTO session_v2 (id, title) VALUES
    ('ses_frozen', '[FROZEN] Freeze probe — backlog fixture'),
    ('ses_active', 'Active session');
-- A completed tool part in the frozen session must not trip the gate.
INSERT INTO session_message (id, session_id, type, seq, data) VALUES
    ('msg_complete', 'ses_frozen', 'assistant', 1,
     '{"content":[{"type":"tool","name":"shell","executed":false,"state":{"status":"completed","input":{},"content":[],"metadata":{}}}]}'),
    -- A running tool part outside a frozen session must not trip the gate.
    ('msg_active', 'ses_active', 'assistant', 1,
     '{"content":[{"type":"tool","name":"shell","executed":false,"state":{"status":"running","input":{},"content":[],"metadata":{}}}]}');
SQL

failures=0
pass() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1" >&2; failures=$((failures + 1)); }

run_check() { # OUT-FILE
    BACKLOG_DOC="$work/BACKLOG.md" BACKLOG_SESSION_DB="$db" \
        BACKLOG_SQLITE="$sqlite_bin" BACKLOG_REPO="$source_root" \
        bash "$check" >"$1" 2>&1
}

expect_rc() { # WANT LABEL OUT-FILE
    local rc=0
    run_check "$3" || rc=$?
    if [ "$rc" -eq "$1" ]; then
        pass "$2 (exit $rc)"
    else
        fail "$2: exit $rc, want $1"
        sed 's/^/     /' "$3" >&2
    fi
}

# Baseline: a frozen session with only a completed tool call passes.
expect_rc 0 'a completed tool call is not pending' "$work/baseline.out"

# Insert the call that a runtime restart would replay. The freeze must fail.
"$sqlite_bin" "$db" <<'SQL'
INSERT INTO session_message (id, session_id, type, seq, data) VALUES
    ('msg_pending', 'ses_frozen', 'assistant', 2,
     '{"content":[{"type":"tool","name":"shell","executed":false,"state":{"status":"running","input":{"command":"runbg run maprot -- bash scripts/land.sh"},"content":[],"metadata":{}}}]}');
SQL
expect_rc 1 'a frozen session with a pending tool call FAILS' "$work/pending.out"
if grep -qF 'msg_pending' "$work/pending.out"; then
    pass 'the failure names the pending message'
else
    fail 'the failure does not name msg_pending'
    sed 's/^/     /' "$work/pending.out" >&2
fi

# Files-free revert of that one message: stage then commit, files:false.
head_before=$(git -C "$source_root" rev-parse HEAD)
"$sqlite_bin" "$db" "DELETE FROM session_message WHERE id='msg_pending';"
head_after=$(git -C "$source_root" rev-parse HEAD)
[ "$head_before" = "$head_after" ] &&
    pass 'the revert touched no worktree files' ||
    fail 'the revert changed HEAD'

expect_rc 0 'after the files-free revert the session PASSES' "$work/cleared.out"

exit "$failures"
