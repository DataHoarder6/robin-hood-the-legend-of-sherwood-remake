#!/usr/bin/env bash
# Assert that a stable release tag, the crate version and the built binary all
# name the same version, and that the tagged commit is in the integration
# branch's history. GitHub tags are mutable (no branch protection or ruleset),
# so this is the check that actually runs before a release is published.
#
#   verify_release_tag.sh <version> [built-robin-binary]
#
# Run from the checkout root, with HEAD at the release tag and the integration
# ref available as `origin/fork/main` (the release workflow fetches it).
set -euo pipefail

version=${1:?usage: verify_release_tag.sh <version> [built-robin-binary]}
binary=${2:-}
tag="v${version}"

fail() {
    printf 'verify-release-tag: %s\n' "$*" >&2
    exit 1
}

# Exact match, not "a tag reachable from HEAD": a tag left on an older commit
# must not silently publish a binary built from a later commit.
described=$(git describe --tags --exact-match HEAD 2>/dev/null) ||
    fail "HEAD is not exactly at a tag (expected ${tag})"
[[ "${described}" == "${tag}" ]] ||
    fail "HEAD carries tag ${described}, expected ${tag}"

# A tag moved to a commit that was never integrated is the failure mode this
# guards: ancestry in the integration ref is the only immutability we can
# enforce without server-side protection.
git rev-parse --verify --quiet origin/fork/main >/dev/null ||
    fail "integration ref origin/fork/main is missing; fetch it before publishing"
git merge-base --is-ancestor "${tag}^0" origin/fork/main ||
    fail "tag ${tag} is not an ancestor of origin/fork/main; refusing a moved or unmerged tag"

# Read the package version, not the unpublished workspace `.version = "0.0.0"`.
crate_version=$(
    awk '
        /^\[package\]/ { in_package = 1; next }
        /^\[/ { in_package = 0 }
        in_package && /^version = "/ {
            gsub(/"/, "", $3)
            print $3
            exit
        }
    ' crates/robin_rs/Cargo.toml
)
[[ -n "${crate_version}" ]] || fail "no package version in crates/robin_rs/Cargo.toml"
[[ "${crate_version}" == "${version}" ]] ||
    fail "crates/robin_rs/Cargo.toml says ${crate_version}, expected ${version}"

# The shipped identity is what the player sees. clap renders `robin <version>+<hash>`;
# the +<hash> suffix is what distinguishes a real build from a stale source tree.
if [[ -n "${binary}" ]]; then
    [[ -x "${binary}" ]] || fail "built binary ${binary} is missing or not executable"
    rendered=$("${binary}" --version) || fail "${binary} --version failed"
    got=${rendered#robin }
    [[ "${got}" == "${version}+"* ]] ||
        fail "${binary} --version reports '${rendered}', expected 'robin ${version}+<commit>'"
fi

printf 'verified release tag %s: crate and binary report %s\n' "${tag}" "${version}"
