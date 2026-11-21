#!/usr/bin/env bash
# Hosts where Rust builds are refused. The estate has two OpenCode runtimes
# (ADR 0036): `oci-vm` and the second one, `oci_g_services`/`oci-g-services`.
# Both carry sessions and repos but no build toolchain; compiling the workspace
# there competes with the agents and has filled the runtime's root disk. The
# build host (minipc) and desktops are allowed.
#
# Match the short hostname so an FQDN cannot slip past the guard. Keep this an
# explicit list: a `oci-*` or `oci_*` pattern would also catch future hosts
# before anyone has decided whether they run Rust suites. Both callers source
# this file rather than repeat the list.
is_non_build_runtime() {
    local host=${1:-${ROBIN_TEST_HOSTNAME:-$(hostname)}}
    local short=${host%%.*}
    case "$short" in
        oci-vm | oci-g-services | oci_g_services) return 0 ;;
        *) return 1 ;;
    esac
}
