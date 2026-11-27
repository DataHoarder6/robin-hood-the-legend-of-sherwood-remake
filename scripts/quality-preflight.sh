#!/usr/bin/env bash
# Fail fast with a named tool when a suite cannot possibly run. The build-host
# devShell provides these tools; CI must install the same pinned versions before
# calling `check-quality.sh A suite`, not after an 18-minute build.
#
#   scripts/quality-preflight.sh <suite>
set -uo pipefail

suite=${1:?usage: quality-preflight.sh <suite>}
tools=(cargo)
case $suite in
    client | client-release | test) tools+=(cargo-nextest) ;;
    wasm-release) tools+=(wasm-objdump wasm-opt wasm-strip node python3 sha256sum) ;;
    tooling) tools+=(python3 pnpm node jq) ;;
    editor | editor-browser) tools+=(pnpm node) ;;
    format | lint | wasm-changed | core | contracts | scripting-llvm | engine | assets | protocols | services | parity | parity-cross | client-check | tools | wasm | wasm-tests | browser-audio | native-lifecycle | multiplayer-e2e | gpu | gpu-gl | host | unreferenced-items | web | docs | fixtures-demo | fixtures-fullgame | fixtures-legacy-linux) ;;
    *) printf 'quality-preflight: unknown-suite %s\n' "$suite" >&2; exit 2 ;;
esac

missing=0
for tool in "${tools[@]}"; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'quality-preflight: %s: missing required tool: %s\n' "$suite" "$tool" >&2
        missing=1
    fi
done
if (( missing )); then
    printf 'quality-preflight: %s: install from flake.nix / scripts/install_pinned_wasm_tools.sh\n' "$suite" >&2
    exit 1
fi
printf 'quality-preflight: %s: tools ok\n' "$suite"
