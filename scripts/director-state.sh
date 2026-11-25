#!/usr/bin/env bash
# Atomic handoff ledger for the unattended director. The tracker remains the
# audit log; this file is only a resume pointer with a no-progress ledger.
#
#   director-state.sh claim  <handoff> <issue> <assertion> <tree-sha>
#   director-state.sh resume <handoff> <tree-sha>
#   director-state.sh finish <handoff> <revision> <state> <reason>
set -euo pipefail

command -v jq >/dev/null || { printf 'director-state: missing-jq\n' >&2; exit 1; }
command -v flock >/dev/null || { printf 'director-state: missing-flock\n' >&2; exit 1; }

action=${1:?usage: director-state.sh claim|resume|finish ...}
shift

case $action in
claim)
    handoff=${1:?claim needs a handoff path}
    issue=${2:?claim needs an issue number}
    assertion=${3:?claim needs an assertion id}
    sha=${4:?claim needs a tree sha}
    exec 9>"$handoff.lock"
    flock -x 9
    revision=$(jq -r '.revision // 0' "$handoff" 2>/dev/null || printf '0')
    revision=$((revision + 1))
    tmp=$handoff.tmp.$$
    jq --argjson rev "$revision" --arg issue "$issue" --arg assertion "$assertion" \
        --arg sha "$sha" --arg at "$(date -u +%FT%TZ)" '
        .revision = $rev
        | .issue = $issue
        | .assertion = $assertion
        | .tree_sha = $sha
        | .attempt = ((.seen[$assertion].attempt // 0) + 1)
        | .state = "pending"
        | .next_action = null
        | .claimed_at = $at
    ' "$handoff" >"$tmp"
    mv -f -- "$tmp" "$handoff"
    printf '%s\n' "$revision"
    ;;
resume)
    handoff=${1:?resume needs a handoff path}
    sha=${2:?resume needs a tree sha}
    [ -f "$handoff" ] || exit 1
    exec 9>"$handoff.lock"
    flock -x 9
    jq -r --arg sha "$sha" '
        select((.state == "pending" or .state == "in_progress")
               and (.assertion != null) and (.tree_sha == $sha))
        | "RESUME|\(.issue)|\(.assertion)|\(.revision)"
    ' "$handoff" 2>/dev/null
    ;;
finish)
    handoff=${1:?finish needs a handoff path}
    expected=${2:?finish needs the claimed revision}
    state=${3:?finish needs a terminal state}
    reason=${4:-none}
    case $state in LANDED | FILED | BLOCKED | NEEDS-DECISION) ;; *)
        printf 'director-state: bad state %s\n' "$state" >&2; exit 2 ;;
    esac
    exec 9>"$handoff.lock"
    flock -x 9
    current=$(jq -r '.revision // 0' "$handoff" 2>/dev/null || printf '0')
    [ "$current" = "$expected" ] || {
        printf 'director-state: stale revision %s (current %s)\n' "$expected" "$current" >&2
        exit 2
    }
    tmp=$handoff.tmp.$$
    jq --argjson rev "$((current + 1))" --arg state "$state" --arg reason "$reason" \
        --arg assertion "$(jq -r '.assertion // empty' "$handoff")" \
        --arg sha "$(jq -r '.tree_sha // empty' "$handoff")" \
        --arg attempt "$(jq -r '.attempt // 0' "$handoff")" \
        --arg at "$(date -u +%FT%TZ)" '
        .revision = $rev
        | .state = $state
        | .reason = $reason
        | .finished_at = $at
        | .next_action = "recompute"
        | if $assertion != "" then
            .seen[$assertion] = {
                state: $state, reason: $reason, tree_sha: $sha,
                attempt: ($attempt | tonumber), at: $at
            }
          else . end
    ' "$handoff" >"$tmp"
    mv -f -- "$tmp" "$handoff"
    ;;
*)
    printf 'director-state: unknown action %s\n' "$action" >&2
    exit 2
    ;;
esac
