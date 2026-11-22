#!/usr/bin/env bash
# Fixture tests for scripts/release/verify_release_tag.sh: tiny local git repos,
# no network, no build. The v0.3.0 tree's crate version was 0.2.0, so the tag
# assertion must reject it and accept the bumped tree.
set -euo pipefail

source_root=$(cd "$(dirname "$0")/.." && pwd -P)
verify=$source_root/scripts/release/verify_release_tag.sh
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid

# One tagged commit with the given crate version and an integration ref at it.
fixture() {
    local dir=$1 crate_version=$2
    mkdir -p "$dir/crates/robin_rs"
    git -C "$dir" init -q -b main
    printf '[package]\nname = "robin_rs"\nversion = "%s"\n' "$crate_version" \
        >"$dir/crates/robin_rs/Cargo.toml"
    git -C "$dir" add .
    git -C "$dir" commit -qm init
    git -C "$dir" tag v0.3.0
    git -C "$dir" update-ref refs/remotes/origin/fork/main HEAD
}

run_verify() {
    local dir=$1
    shift
    ( cd "$dir" && bash "$verify" "$@" )
}

expect_pass() {
    local label=$1 pattern=$2
    shift 2
    local out rc=0
    out=$("$@" 2>&1) || rc=$?
    if ((rc != 0)); then
        printf 'FAIL %s: expected success, got %d: %s\n' "$label" "$rc" "$out" >&2
        exit 1
    fi
    [[ "$out" == *"$pattern"* ]] || {
        printf 'FAIL %s: expected %q in output: %s\n' "$label" "$pattern" "$out" >&2
        exit 1
    }
    printf 'ok: %s\n' "$label"
}

expect_fail() {
    local label=$1 pattern=$2
    shift 2
    local out rc=0
    out=$("$@" 2>&1) || rc=$?
    if ((rc == 0)); then
        printf 'FAIL %s: expected failure, got success: %s\n' "$label" "$out" >&2
        exit 1
    fi
    [[ "$out" == *"$pattern"* ]] || {
        printf 'FAIL %s: expected %q in output: %s\n' "$label" "$pattern" "$out" >&2
        exit 1
    }
    printf 'ok: %s\n' "$label"
}

binary=$work/robin
printf '#!/usr/bin/env sh\necho "robin 0.3.0+abcdefabcdef"\n' >"$binary"
old_binary=$work/robin-old
printf '#!/usr/bin/env sh\necho "robin 0.2.0+abcdefabcdef"\n' >"$old_binary"
chmod +x "$binary" "$old_binary"

# The v0.3.0 tag's tree still declared 0.2.0: the exact bug this catches.
fixture "$work/before" 0.2.0
expect_fail 'drifted crate version before the bump' 'Cargo.toml says 0.2.0' \
    run_verify "$work/before" 0.3.0 "$binary"

fixture "$work/after" 0.3.0
expect_pass 'bumped tree and matching binary' 'verified release tag v0.3.0' \
    run_verify "$work/after" 0.3.0 "$binary"
expect_fail 'binary built from a different version' "expected 'robin 0.3.0+<commit>'" \
    run_verify "$work/after" 0.3.0 "$old_binary"

# A commit after the tag must not publish as that tag.
printf 'more' >>"$work/after/crates/robin_rs/Cargo.toml"
git -C "$work/after" add .
git -C "$work/after" commit -qm later
expect_fail 'HEAD past the tag' 'not exactly at a tag' \
    run_verify "$work/after" 0.3.0 "$binary"

fixture "$work/unintegrated" 0.3.0
git -C "$work/unintegrated" checkout -q --orphan other
git -C "$work/unintegrated" rm -q -rf .
printf 'unrelated' >"$work/unintegrated/other.txt"
git -C "$work/unintegrated" add .
git -C "$work/unintegrated" commit -qm unrelated
git -C "$work/unintegrated" update-ref refs/remotes/origin/fork/main HEAD
git -C "$work/unintegrated" checkout -q main
expect_fail 'tag not an ancestor of the integration ref' 'not an ancestor' \
    run_verify "$work/unintegrated" 0.3.0 "$binary"

fixture "$work/no-ref" 0.3.0
git -C "$work/no-ref" update-ref -d refs/remotes/origin/fork/main
expect_fail 'missing integration ref' 'origin/fork/main is missing' \
    run_verify "$work/no-ref" 0.3.0 "$binary"

printf 'verify_release_tag: all fixture checks passed\n'
