#!/usr/bin/env bash
# Fixture test for scripts/session-gate-check.sh. Only temp SQLite DBs and
# `runbg list` fixtures are touched, never the live session DB or systemd.
set -uo pipefail

repo=$(git rev-parse --show-toplevel)
check=${SESSION_CHECK_SCRIPT:-$repo/scripts/session-gate-check.sh}
sqlite_bin=${SESSION_CHECK_SQLITE:-sqlite3}
command -v "$sqlite_bin" >/dev/null || {
    printf 'test_session_gate_check: sqlite3 not available: %s\n' "$sqlite_bin" >&2
    exit 1
}

work=$(mktemp -d "${TMPDIR:-/tmp}/session-gate-check.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

cat >"$work/active.list" <<'EOF'
UNIT LOAD ACTIVE SUB DESCRIPTION
runbg-gate-a.service loaded active running Gate A
runbg-gate-b.service loaded active running Gate B
EOF
cat >"$work/empty.list" <<'EOF'
UNIT LOAD ACTIVE SUB DESCRIPTION
EOF

new_db() { # DB
    "$sqlite_bin" "$1" <<'SQL'
CREATE TABLE session_v2 (id TEXT PRIMARY KEY, title TEXT,
                         time_archived INTEGER, time_suspended INTEGER);
CREATE TABLE session_message (
    id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL,
    seq INTEGER NOT NULL, data TEXT NOT NULL
);
INSERT INTO session_v2 VALUES
    ('ses_active', 'Active session', NULL, NULL),
    ('ses_frozen', '[FROZEN] Rotating map — backlog fixture', NULL, NULL),
    ('ses_archived', 'Archived session', 1, NULL);
SQL
}

tool() { # DB ID SESSION SEQ STATUS COMMAND
    "$sqlite_bin" "$1" "INSERT INTO session_message VALUES (
        '$2', '$3', 'assistant', $4,
        '{\"content\":[{\"type\":\"tool\",\"id\":\"$2\",\"name\":\"shell\",\"state\":{\"status\":\"$5\",\"input\":{\"command\":\"$6\"}}}]}');"
}

fails=0
expect() { # WANT LABEL NEEDLE DB LIST
    local out rc=0
    out=$(SESSION_CHECK_DB="$4" SESSION_CHECK_SQLITE="$sqlite_bin" \
        SESSION_CHECK_RUNBG_LIST="$5" bash "$check" 2>&1) || rc=$?
    if [ "$rc" -ne "$1" ]; then
        printf 'FAIL %s: exit %s, want %s\n' "$2" "$rc" "$1" >&2
        printf '%s\n' "$out" | sed 's/^/     /' >&2
        fails=$((fails + 1))
    elif [ -n "$3" ] && ! printf '%s' "$out" | grep -qF -- "$3"; then
        printf 'FAIL %s: missing %s\n' "$2" "$3" >&2
        printf '%s\n' "$out" | sed 's/^/     /' >&2
        fails=$((fails + 1))
    else
        printf 'ok   %s\n' "$2"
    fi
}

# One in-flight wait is normal: it must not fail the gate.
new_db "$work/one.db"
tool "$work/one.db" msg_wait_a ses_active 1 running 'runbg wait gate-a'
expect 0 'one in-flight wait passes' 'fails=0' "$work/one.db" "$work/active.list"

# run + wait in one tool call is still one in-flight gate.
new_db "$work/combined.db"
tool "$work/combined.db" msg_combined ses_active 1 running 'runbg run gate-b -- bash scripts/land.sh && runbg wait gate-b'
expect 0 'one combined run+wait call passes' 'fails=0' "$work/combined.db" "$work/active.list"

# Two pending runbg calls in one active session are replayed as two gates.
new_db "$work/two.db"
tool "$work/two.db" msg_wait_a ses_active 1 running 'runbg wait gate-a'
tool "$work/two.db" msg_wait_b ses_active 2 pending 'runbg wait gate-b'
expect 1 'two pending waits FAIL' 'msg_wait_b' "$work/two.db" "$work/active.list"

# The replay of a wait whose unit no longer exists is a duplicate gate.
new_db "$work/dead.db"
tool "$work/dead.db" msg_wait_dead ses_active 1 running 'runbg wait gate-a'
expect 1 'wait for a dead unit FAILS' 'gate-a' "$work/dead.db" "$work/empty.list"

# Frozen/archived sessions and completed/non-runbg tools are not this check's
# business: backlog-check owns frozen sessions, and one in-flight gate is fine.
new_db "$work/noise.db"
tool "$work/noise.db" msg_done ses_active 1 completed 'runbg wait gate-a'
tool "$work/noise.db" msg_other ses_active 2 running 'echo hello'
tool "$work/noise.db" msg_frozen ses_frozen 3 running 'runbg wait gate-a'
tool "$work/noise.db" msg_old ses_archived 4 running 'runbg wait gate-a'
expect 0 'completed, non-runbg, frozen and archived tools are ignored' \
    'fails=0' "$work/noise.db" "$work/active.list"

if [ "$fails" -ne 0 ]; then
    printf 'test_session_gate_check: %s failure(s)\n' "$fails" >&2
    exit 1
fi
echo 'test_session_gate_check: OK'
