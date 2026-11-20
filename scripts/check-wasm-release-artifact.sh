#!/usr/bin/env bash
# Build the exact wasm-release module the site publishes and prove its
# contract here. The `wasm` suite's cargo commands compile wasm-dev; the site
# additionally runs wasm-bindgen and wasm-opt, and the guest's decode pool
# instantiates the threaded module's shared memory. A release-only break was
# therefore invisible to every gate until this check.
set -euo pipefail

repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
cd -- "$repository"

for tool in node python3 wasm-objdump wasm-opt wasm-strip sha256sum; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'wasm-release-artifact: %s is not on PATH; run inside `nix develop`.\n' "$tool" >&2
        exit 1
    fi
done

bindgen=${ROBINHOOD_WASM_BINDGEN:-}
if [[ -z $bindgen ]]; then
    # Outside the checkout: a destination under target/ is picked up as part
    # of this workspace and `cargo install --path` refuses it. A cache under
    # HOME also survives the build host's rsync, which excludes it.
    bindgen=${XDG_CACHE_HOME:-$HOME/.cache}/robin-wasm-tools/wasm-bindgen-0.2.129/bin/wasm-bindgen
    if [[ ! -x $bindgen ]]; then
        printf 'wasm-release-artifact: installing pinned wasm-bindgen 0.2.129\n' >&2
        # The installer runs outside this checkout, so it does not inherit
        # .cargo/config.toml; without an explicit driver rustc reaches for a
        # machine-local lld wrapper. The repo's own linker script is the one
        # source for how this workspace links.
        export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER:-$repository/scripts/clang-wild-linker.sh}
        bash scripts/install_pinned_wasm_bindgen.sh \
            "${XDG_CACHE_HOME:-$HOME/.cache}/robin-wasm-tools/wasm-bindgen-0.2.129" >&2
    fi
fi
bindgen_version=$("$bindgen" --version)
if [[ $bindgen_version != 'wasm-bindgen 0.2.129' ]]; then
    printf 'wasm-release-artifact: need wasm-bindgen 0.2.129, got %s\n' "$bindgen_version" >&2
    exit 1
fi

# The shell compares the invitation against this number, so it comes from the
# published contract, never a literal in this script.
schema=$(python3 -c 'import json; print(json.load(open("wasm-www/runtime-contract.json"))["ticketSchema"])')

out=$(mktemp -d "${TMPDIR:-/tmp}/robin-wasm-release.XXXXXX")
trap 'rm -rf -- "$out"' EXIT

# The repo's plan, with the shipped profile: threaded wasm-release,
# audio,multiplayer, the pinned bindgen naming, and the production
# wasm-opt/wasm-strip pass. `--threads` is what the published module uses.
node wasm-www/scripts/build-runtime.mjs --profile wasm-release --threads \
    --out-dir "$out" --bindgen "$bindgen"

wasm=$out/robin_bg.wasm
if [[ ! -f $wasm ]]; then
    printf 'wasm-release-artifact: build plan produced no %s\n' "$wasm" >&2
    exit 1
fi

# build-runtime.mjs checks the cargo module before bindgen. The published
# bytes are post wasm-opt, and those are what the page instantiates: require
# exactly one shared memory with the full wasm32 maximum the decode workers
# declare, rather than trusting the pre-optimization check.
mapfile -t memories < <(wasm-objdump -x -j Import "$wasm" | grep -E 'memory\[[0-9]+\]')
if [[ ${#memories[@]} -ne 1 ]]; then
    printf 'wasm-release-artifact: expected one memory import, got %s\n' \
        "$(printf '%s | ' "${memories[@]}")" >&2
    exit 1
fi
if [[ ${memories[0]} != *' shared '* ]]; then
    printf 'wasm-release-artifact: published module does not import shared memory: %s\n' \
        "${memories[0]}" >&2
    exit 1
fi
max_re='max=65536([^0-9]|$)'
if [[ ${memories[0]} =~ $max_re ]]; then
    printf 'wasm-release-artifact: %s\n' "${memories[0]}" >&2
else
    printf 'wasm-release-artifact: published module has no 65536-page maximum: %s\n' \
        "${memories[0]}" >&2
    exit 1
fi

# The compatibility export is the first thing the stable shell calls. wasm-opt
# does not keep bindgen's export name in the wasm Export section: it is exposed
# through the generated JS glue. Instantiating that glue is the only check that
# proves the published bytes answer the call, so the verifier below is the
# assertion; do not grep a section that a correct release need not carry.

# Schema, from the artifact's own glue through the existing verifier.
node wasm-www/scripts/verify-runtime-multiplayer.mjs \
    --pkg "$out" --expect-ticket-schema "$schema"

printf 'wasm-release-artifact: %s bytes sha256=%s\n' \
    "$(wc -c <"$wasm")" "$(sha256sum "$wasm" | awk '{print $1}')" >&2
