#!/usr/bin/env bash
# Fixture tests for the terminal write-back: stub gh, real ledger, no network.
set -euo pipefail
source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-writeback.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

stubs=$work/stubs
mkdir -p "$stubs"
cat >"$stubs/gh" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$GH_LOG"
case "$1 $2" in
    'issue comment')
        printf '%s\n' "$*" >>"$GH_BODIES"
        [ -n "${GH_COMMENT_FAIL:-}" ] && exit 1
        printf 'comment ok\n'
        ;;
esac
STUB
chmod +x "$stubs/gh"

handoff=$work/handoff.json
template() { # ISSUE ASSERTION REVISION
    printf '{"revision":%s,"issue":%s,"assertion":"%s","tree_sha":"abc123456789","attempt":1,"state":"pending","seen":{},"next_action":null,"selected_item":"D12 / #83 teleport desync"}\n' \
        "$3" "$1" "$2" >"$handoff"
}

export PATH="$stubs:$PATH" GH_LOG=$work/gh.log GH_BODIES=$work/bodies.log \
    ROBIN_HANDOFF=$handoff ROBIN_RELEASE_REPO=owner/repo
: >"$GH_LOG" : >"$GH_BODIES"

fail=0
pass() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1" >&2; fail=1; }

# The explicit issue field wins over any hashtag in a prose field.
template 49 D14 1
bash "$source_root/scripts/director-writeback.sh" FILED 2 b5e955bf 'core red' 'known flake' >/dev/null
grep -q 'comment 49' "$GH_LOG" && ! grep -q 'comment 83' "$GH_LOG" \
    && pass 'explicit-issue-wins-over-hashtag' \
    || bad "explicit-issue-wins-over-hashtag: $(cat "$GH_LOG")"

grep -q -- '--add-label filed' "$GH_LOG" \
    && pass 'filed-writeback-adds-filed-label' \
    || bad "filed-writeback-adds-filed-label: $(cat "$GH_LOG")"

grep -q 'b5e955bf' "$GH_BODIES" && grep -q 'FILED' "$GH_BODIES" \
    && pass 'comment-carries-terminal-evidence' \
    || bad 'comment-carries-terminal-evidence'

jq -e '.state == "FILED" and .seen.D14.state == "FILED"' "$handoff" >/dev/null 2>&1 \
    && pass 'finish-records-terminal-state' \
    || bad "finish-records-terminal-state: $(cat "$handoff")"

# Crash dispatch supplies its own issue variable.
template 49 D14 1
ROBIN_CRASH_ISSUE=93 bash "$source_root/scripts/director-writeback.sh" BLOCKED 1 none none 'quota-exhausted' >/dev/null
grep -q 'comment 93' "$GH_LOG" && grep -q -- '--add-label ready-for-human' "$GH_LOG" \
    && pass 'crash-issue-env-is-honoured' \
    || bad "crash-issue-env-is-honoured: $(cat "$GH_LOG")"

printf '{"revision":1,"issue":null,"assertion":null,"tree_sha":null,"attempt":0,"state":"pending","seen":{},"selected_item":"no issue here"}\n' >"$handoff"
if bash "$source_root/scripts/director-writeback.sh" FILED 1 none none done >/dev/null 2>&1; then
    bad 'writeback-fails-closed-on-missing-issue'
else
    pass 'writeback-fails-closed-on-missing-issue'
fi

# A failed comment must not advance the handoff or change labels.
template 49 D14 1
: >"$GH_LOG"
if GH_COMMENT_FAIL=1 bash "$source_root/scripts/director-writeback.sh" FILED 2 none none 'gh down' >/dev/null 2>&1; then
    bad 'finish-called-after-comment'
else
    jq -e '.state == "pending" and .seen == {}' "$handoff" >/dev/null 2>&1 \
        && ! grep -q -- '--add-label filed' "$GH_LOG" \
        && pass 'finish-called-after-comment' \
        || bad "finish-called-after-comment: $(cat "$handoff") / $(cat "$GH_LOG")"
fi

(( fail == 0 )) || exit 1
printf 'director-writeback: OK\n'
