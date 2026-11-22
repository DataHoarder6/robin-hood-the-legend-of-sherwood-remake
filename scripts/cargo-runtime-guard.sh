#!/usr/bin/env bash
# DevShell `cargo` wrapper, installed ahead of the pinned toolchain on PATH.
# The runtime is not a build host: compiling the workspace there competes with
# the agents that live on it and has filled the root disk. The task-3 guard only
# covered check-quality.sh, so a raw `nix develop --command cargo build` slipped
# through. Refuse the build-like subcommands on either OpenCode runtime
# (oci-vm and oci-g-services, ADR 0036); exec the real cargo for everything else
# (developer examples, cargo metadata, any other host).
set -euo pipefail

host=${ROBIN_TEST_HOSTNAME:-$(hostname)}

# Find the real cargo past this wrapper. Both the wrapper's PATH entry and this
# script resolve to the same file, so compare canonical paths and skip it.
self=$(readlink -f -- "$0")

# The wrapper is symlinked into the devShell as `cargo`, so resolve the sibling
# helper from the script's real path, not from the symlink's directory.
# shellcheck source=runtime-hosts.sh
source "$(dirname -- "$self")/runtime-hosts.sh"

real_cargo=
old_ifs=$IFS
IFS=:
for dir in $PATH; do
    [ -n "$dir" ] || dir=.
    candidate="$dir/cargo"
    if [ -x "$candidate" ] && [ "$(readlink -f -- "$candidate")" != "$self" ]; then
        real_cargo=$candidate
        break
    fi
done
IFS=$old_ifs

if [ -z "$real_cargo" ]; then
    printf 'cargo guard: no real cargo found on PATH past %s\n' "$self" >&2
    exit 1
fi

# Cargo puts global flags and a +toolchain selector before the subcommand. Skip
# value-taking flags too, so `--color always build` still resolves to `build`.
subcommand=
skip_value=0
for arg in "$@"; do
    if (( skip_value )); then
        skip_value=0
        continue
    fi
    case "$arg" in
        +*) continue ;;
        --color|--config|-Z|--manifest-path) skip_value=1 ;;
        -*) continue ;;
        *) subcommand=$arg; break ;;
    esac
done

if is_non_build_runtime "$host"; then
    case "$subcommand" in
        build|test|check|clippy)
            printf 'cargo: %s is not a build host; Rust builds are refused here. Run `scripts/remote-check.sh SUITE` on the build host (minipc by default).\n' \
                "$host" >&2
            exit 1
            ;;
    esac
fi

exec "$real_cargo" "$@"
