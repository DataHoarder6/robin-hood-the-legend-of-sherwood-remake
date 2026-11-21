#!/usr/bin/env bash
# Fixture test for the devShell cargo wrapper. It never compiles: a stub `cargo`
# stands in for the pinned toolchain and records pass-through invocations, and
# ROBIN_TEST_HOSTNAME injects the host so the fixture runs anywhere.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/../.." && pwd -P)
guard=$source_root/scripts/cargo-runtime-guard.sh

work=$(mktemp -d "${TMPDIR:-/tmp}/robin-cargo-runtime-guard.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

# `wrapper/cargo` is a symlink to the guard (its devShell position); `real/cargo`
# is the stub the guard must exec for every command it allows through.
mkdir -p -- "$work/wrapper" "$work/real"
ln -s -- "$guard" "$work/wrapper/cargo"
cat >"$work/real/cargo" <<'STUB'
#!/usr/bin/env bash
printf 'REAL_CARGO %s\n' "$*"
STUB
chmod +x -- "$work/real/cargo"

test_path="$work/wrapper:$work/real:$PATH"

failures=0
pass() { printf 'ok   %s\n' "$1"; }
fail() {
    printf 'FAIL %s\n' "$1"
    failures=$((failures + 1))
}

run() { # HOST ARGS...
    local host=$1
    shift
    ROBIN_TEST_HOSTNAME=$host PATH="$test_path" cargo "$@" 2>&1
}

expect_refused() { # HOST ARGS...
    local host=$1
    shift
    local out rc
    set +e
    out=$(run "$host" "$@")
    rc=$?
    set -e
    if (( rc != 0 )) &&
       [[ "$out" == *'scripts/remote-check.sh SUITE'* ]] &&
       [[ "$out" != *'REAL_CARGO'* ]]; then
        pass "refused on $host: cargo $*"
    else
        fail "expected refusal on $host for 'cargo $*': rc=$rc out=$out"
    fi
}

expect_pass() { # HOST ARGS...
    local host=$1
    shift
    local out rc
    set +e
    out=$(run "$host" "$@")
    rc=$?
    set -e
    if (( rc == 0 )) &&
       [[ "$out" == REAL_CARGO* ]] &&
       [[ "$out" != *'remote-check.sh'* ]]; then
        pass "passed through on $host: cargo $*"
    else
        fail "expected pass-through on $host for 'cargo $*': rc=$rc out=$out"
    fi
}

# --- build-like subcommands are refused on the runtime ----------------------
for subcommand in build test check clippy; do
    expect_refused oci-vm "$subcommand"
done

# --- a toolchain/flag prefix must not hide a build --------------------------
expect_refused oci-vm +nightly build
expect_refused oci-vm --color always build

# --- the second runtime and FQDNs are refused too (ADR 0036) ----------------
expect_refused oci-g-services build
expect_refused oci_g_services clippy
expect_refused oci-vm.example.com test
expect_refused oci-g-services.example.com check

# --- developer commands and a normal host still reach cargo -----------------
expect_pass oci-vm run --example ferris_overlay
expect_pass oci-vm metadata
expect_pass minipc build
expect_pass minipc test

printf '\n%s checks failed\n' "$failures"
(( failures == 0 ))
