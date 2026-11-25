#!/usr/bin/env bash
# Fixture tests for suite tool preflight: no compilation, stub tools on PATH.
set -euo pipefail
source_root=$(cd -- "$(dirname -- "$0")/../.." && pwd -P)
preflight=$source_root/scripts/quality-preflight.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-quality-preflight.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

fail=0
pass() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1" >&2; fail=1; }

bins=$work/bin
mkdir -p "$bins"
ln -s "$(command -v bash)" "$bins/bash"
for tool in cargo wasm-objdump wasm-opt wasm-strip node python3 sha256sum pnpm jq; do
    printf '#!/bin/sh\nexit 0\n' >"$bins/$tool"
    chmod +x "$bins/$tool"
done

set +e
out=$(PATH="$bins" bash "$preflight" client-release 2>&1)
rc=$?
set -e
[[ $rc -ne 0 && "$out" == *'cargo-nextest'* ]] \
    && pass 'missing-tool-preflight' \
    || bad "missing-tool-preflight: rc=$rc out=$out"

printf '#!/bin/sh\nexit 0\n' >"$bins/cargo-nextest"
chmod +x "$bins/cargo-nextest"
set +e
out=$(PATH="$bins" bash "$preflight" client-release 2>&1)
rc=$?
set -e
[[ $rc -eq 0 && "$out" == *'tools ok'* ]] \
    && pass 'present-tools-are-not-rejected' \
    || bad "present-tools-are-not-rejected: rc=$rc out=$out"

set +e
out=$(PATH="$bins" bash "$preflight" wasm 2>&1)
rc=$?
set -e
[[ $rc -eq 0 ]] && pass 'wasm-tool-set-is-complete' \
    || bad "wasm-tool-set-is-complete: rc=$rc out=$out"

# A missing wasm tool fails before a long build.
rm -f -- "$bins/wasm-objdump"
set +e
out=$(PATH="$bins" bash "$preflight" wasm 2>&1)
rc=$?
set -e
[[ $rc -ne 0 && "$out" == *'wasm-objdump'* ]] \
    && pass 'wasm-preflight-names-missing-tool' \
    || bad "wasm-preflight-names-missing-tool: rc=$rc out=$out"

set +e
out=$(PATH="$bins" bash "$preflight" no-such-suite 2>&1)
rc=$?
set -e
[[ $rc -ne 0 && "$out" == *'unknown-suite'* ]] \
    && pass 'unknown-suite-fails-closed' \
    || bad "unknown-suite-fails-closed: rc=$rc out=$out"

(( fail == 0 )) || exit 1
printf 'quality-preflight: OK\n'
