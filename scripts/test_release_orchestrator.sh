#!/usr/bin/env bash
# Fixture tests for the idempotent release tail: stubs for git/gh/curl/promote.
set -euo pipefail
source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
orchestrator=$source_root/scripts/release-orchestrator.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-release-orch.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

stubs=$work/stubs
mkdir -p "$stubs" "$work/repo/docs/releases"
cat >"$stubs/git" <<'STUB'
#!/usr/bin/env bash
printf 'git %s\n' "$*" >>"$CALL_LOG"
case "$1 $2" in
    'show-ref --verify') [ -n "${LOCAL_TAG:-}" ] ;;
    'ls-remote fork') [ -n "${REMOTE_TAG:-}" ] && printf 'sha\trefs/tags/%s\n' "${TAG:?}" ;;
    'push fork') exit 0 ;;
    *) printf 'stub git: unexpected %s\n' "$*" >&2; exit 64 ;;
esac
STUB
cat >"$stubs/gh" <<'STUB'
#!/usr/bin/env bash
printf 'gh %s\n' "$*" >>"$CALL_LOG"
exit "${GH_FAIL:-0}"
STUB
cat >"$stubs/curl" <<'STUB'
#!/usr/bin/env bash
printf 'curl %s\n' "$*" >>"$CALL_LOG"
printf '%s\n' "${RELEASE_JSON:-{\"}assets\":[]}"
STUB
cat >"$stubs/promote" <<'STUB'
#!/usr/bin/env bash
printf 'promote %s\n' "$*" >>"$CALL_LOG"
exit "${PROMOTE_FAIL:-0}"
STUB
cat >"$stubs/parity" <<'STUB'
#!/usr/bin/env bash
printf 'parity %s\n' "$*" >>"$CALL_LOG"
exit "${PARITY_FAIL:-0}"
STUB
chmod +x "$stubs/git" "$stubs/gh" "$stubs/curl" "$stubs/promote" "$stubs/parity"

printf '# What\n\n## v0.6.0\n\n- landed source\n' >"$work/repo/docs/RELEASE_NOTES.md"
printf '{"v0.6.0":{"new_features":[],"commits":["abc123456789"]}}\n' >"$work/repo/docs/releases/whats-new.json"

export PATH="$stubs:$PATH" CALL_LOG="$work/calls.log" TAG=v0.6.0 \
    ROBIN_REPO_DIR="$work/repo" ROBIN_GIT=git ROBIN_GH=gh ROBIN_CURL=curl \
    ROBIN_PROMOTE="$stubs/promote" ROBIN_PARITY="$stubs/parity" \
    ROBIN_RELEASE_STATE="$work/state.json" ROBIN_RELEASE_POLL=0 \
    RELEASE_JSON='{"tag_name":"v0.6.0","assets":[{"name":"robinhood-remake-linux.AppImage","digest":"sha256:aaa"},{"name":"robinhood-remake-windows-Setup.exe","digest":"sha256:bbb"}]}'

fail=0
pass() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1" >&2; fail=1; }
reset() {
    : >"$CALL_LOG"
    rm -f -- "$ROBIN_RELEASE_STATE"
    export LOCAL_TAG=1 REMOTE_TAG=1 PROMOTE_FAIL=0 PARITY_FAIL=0 GH_FAIL=0
}

reset
out=$(bash "$orchestrator" "$TAG" 2>&1) || true
[[ -f "$ROBIN_RELEASE_STATE" ]] && jq -e '.tag == "v0.6.0" and .outcome == "RELEASED"' "$ROBIN_RELEASE_STATE" >/dev/null 2>&1 \
    && ! grep -q '^git push' "$CALL_LOG" \
    && grep -q '^gh workflow run' "$CALL_LOG" \
    && grep -q '^parity ' "$CALL_LOG" \
    && pass 'tag-exists-is-idempotent' \
    || bad "tag-exists-is-idempotent: $out / $(cat "$CALL_LOG")"

reset
RELEASE_JSON='{"tag_name":"v0.6.0","assets":[]}' \
    bash "$orchestrator" "$TAG" >/dev/null 2>&1 || true
[ ! -f "$ROBIN_RELEASE_STATE" ] \
    && pass 'no-assets-does-not-record-RELEASED' \
    || bad 'no-assets-does-not-record-RELEASED'

reset
PARITY_FAIL=1 bash "$orchestrator" "$TAG" >/dev/null 2>&1 || true
[ ! -f "$ROBIN_RELEASE_STATE" ] \
    && pass 'parity-failure-blocks-RELEASED' \
    || bad 'parity-failure-blocks-RELEASED'

reset
PROMOTE_FAIL=1 bash "$orchestrator" "$TAG" >/dev/null 2>&1 || true
[ ! -f "$ROBIN_RELEASE_STATE" ] \
    && pass 'promote-failure-blocks-RELEASED' \
    || bad 'promote-failure-blocks-RELEASED'

reset
LOCAL_TAG=1 REMOTE_TAG= bash "$orchestrator" "$TAG" >/dev/null 2>&1 || true
grep -q '^git push fork v0.6.0$' "$CALL_LOG" \
    && ! grep -qE 'tag -d|push --force|push -f' "$CALL_LOG" \
    && pass 'retry-pushes-new-tag-once-without-moving' \
    || bad "retry-pushes-new-tag-once-without-moving: $(cat "$CALL_LOG")"

grep -q 'release-orchestrator.sh' "$source_root/scripts/autonomy-driver.sh" \
    && pass 'driver-calls-orchestrator-before-RELEASED' \
    || bad 'driver-calls-orchestrator-before-RELEASED'

[[ "$fail" -eq 0 ]] || exit 1
printf 'release-orchestrator: OK\n'
