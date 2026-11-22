#!/usr/bin/env bash
# Fail when a runtime restart can replay a pending runbg call into a second
# gate: an active session with two or more pending `runbg wait`/`run` tool
# calls, or a pending wait for a runbg unit that is no longer active. One
# in-flight gate is normal and passes.
#
# Frozen sessions with any pending tool are backlog-check's failure, so they
# are excluded here. Both checks read the same read-only session DB.
#
# Test seams:
#   SESSION_CHECK_DB           session DB (default ~/.local/share/opencode/opencode.db)
#   SESSION_CHECK_SQLITE       sqlite3 binary (default sqlite3)
#   SESSION_CHECK_RUNBG_LIST   file holding `runbg list` output instead of a
#                              live call; use an empty file to simulate no units
set -uo pipefail

db=${SESSION_CHECK_DB:-$HOME/.local/share/opencode/opencode.db}
sqlite_bin=${SESSION_CHECK_SQLITE:-sqlite3}
list_file=${SESSION_CHECK_RUNBG_LIST:-}

# The build host has no agent session DB. Say so instead of failing a gate that
# cannot possibly inspect the runtime's sessions.
if [[ ! -f $db ]]; then
    printf 'session-gate-check: SKIP: no session DB at %s\n' "$db"
    exit 0
fi
if ! sqlite_bin=$(command -v "$sqlite_bin" 2>/dev/null); then
    printf 'session-gate-check: SKIP: sqlite3 is unavailable\n' >&2
    exit 0
fi

# `runbg list` is `systemctl --user list-units 'runbg-*' --all --no-legend`.
# Only these states are still waiting on something: a wait for a unit that has
# exited (or was never started) is a replay of an already-finished gate.
declare -A active_units=()
have_units=0
if [[ -n $list_file ]]; then
    if [[ ! -f $list_file ]]; then
        printf 'session-gate-check: no runbg list fixture at %s\n' "$list_file" >&2
        exit 1
    fi
    have_units=1
    while read -r unit _load state _sub _desc; do
        case $state in
            active | activating | deactivating | reloading) ;;
            *) continue ;;
        esac
        [[ $unit == runbg-*.service ]] || continue
        name=${unit#runbg-}
        active_units["${name%.service}"]=1
    done <"$list_file"
elif command -v runbg >/dev/null 2>&1; then
    if list=$(runbg list 2>/dev/null); then
        have_units=1
        while read -r unit _load state _sub _desc; do
            case $state in
                active | activating | deactivating | reloading) ;;
                *) continue ;;
            esac
            [[ $unit == runbg-*.service ]] || continue
            name=${unit#runbg-}
            active_units["${name%.service}"]=1
        done <<<"$list"
    fi
fi

# Each content part is one tool call. `runbg run ... && runbg wait ...` is a
# single part and must count once. A tool whose command merely mentions runbg
# in prose (a grep, for instance) is not a gate call.
rows=$("$sqlite_bin" -readonly "$db" -separator '|' "
    SELECT s.id, coalesce(s.title, ''), m.id,
           json_extract(p.value, '\$.id'),
           json_extract(p.value, '\$.state.input.command')
    FROM session_v2 s
    JOIN session_message m ON m.session_id = s.id
    JOIN json_each(m.data, '\$.content') p
    WHERE m.type = 'assistant'
      AND json_extract(p.value, '\$.type') = 'tool'
      AND json_extract(p.value, '\$.state.status') IN ('running', 'pending')
      AND s.time_archived IS NULL
      AND s.time_suspended IS NULL
      AND coalesce(s.title, '') NOT LIKE '[FROZEN]%'
      AND json_extract(p.value, '\$.state.input.command') IS NOT NULL
      AND json_extract(p.value, '\$.state.input.command') GLOB '*runbg*'
    ORDER BY s.id, m.seq;" 2>/dev/null) || {
    printf 'session-gate-check: cannot read session DB %s\n' "$db" >&2
    exit 1
}

# A call starts at the shell boundary or after a separator: prose that merely
# mentions "runbg wait" inside quotes is not a replayable gate command.
gate_re='(^|[[:space:];&|])(runbg[[:space:]]+(wait|run)[[:space:]]+[A-Za-z0-9_.-]+)'

fails=0
calls=0
declare -A counts=() titles=() ids=()
while IFS='|' read -r sid title mid _pid cmd; do
    [[ -n $sid ]] || continue
    [[ $cmd =~ $gate_re ]] || continue
    calls=$((calls + 1))
    counts["$sid"]=$(( ${counts["$sid"]:-0} + 1 ))
    titles["$sid"]=$title
    if [[ -n ${ids["$sid"]:-} ]]; then
        ids["$sid"]="${ids["$sid"]} $mid"
    else
        ids["$sid"]=$mid
    fi

    # A name is a runbg slot: letters, digits, dot, underscore, dash.
    rest=$cmd
    while [[ $rest =~ runbg[[:space:]]+wait[[:space:]]+([A-Za-z0-9_.-]+) ]]; do
        wait_name=${BASH_REMATCH[1]}
        matched=${BASH_REMATCH[0]}
        if (( have_units )) && [[ -z ${active_units["$wait_name"]:-} ]]; then
            printf "session-gate-check: FAIL: session '%s' message %s waits for dead runbg unit '%s'\n" \
                "$title" "$mid" "$wait_name"
            fails=$((fails + 1))
        fi
        rest=${rest#*"$matched"}
    done
done <<<"$rows"

# Anything the runtime replays twice is two gates, even when both waits name
# the same live slot: the first job is still running and the second call waits
# on it again while agents start new names to get around the collision.
for sid in "${!counts[@]}"; do
    if (( counts["$sid"] >= 2 )); then
        printf "session-gate-check: FAIL: session '%s' has %s pending runbg calls (%s)\n" \
            "${titles["$sid"]}" "${counts["$sid"]}" "${ids["$sid"]}"
        fails=$((fails + 1))
    fi
done

printf 'session-gate-check: fails=%s (%s pending runbg call(s))\n' "$fails" "$calls"
[[ $fails -eq 0 ]] || exit 1
exit 0
