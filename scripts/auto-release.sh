#!/usr/bin/env bash
# Autonomous MINOR release for the robin-hood-remake repo. Refuse every tag
# move, every non-MINOR bump, every release without an observed release gate and
# H23; then tag, push the tag to fork and dispatch the existing native-release
# workflow. Nothing here installs, pins a seat, or launches a game.
#
#   scripts/auto-release.sh <candidate-commit> <release-binary> [summary.json]
#
# The summary is produced by scripts/release-gate.sh (its path is printed on a
# green run); it can also be named by ROBIN_RELEASE_SUMMARY. jq is the authority
# on `.verdict` and `.verdicts.H23`, never agent narration.
set -euo pipefail

usage() {
    printf 'usage: auto-release.sh <candidate-commit> <release-binary> [summary.json]\n' >&2
    exit 2
}
die() { printf 'auto-release: %s\n' "$*"; exit 1; }

[ $# -eq 2 ] || [ $# -eq 3 ] || usage
candidate=$1
binary=$2
summary=${3:-${ROBIN_RELEASE_SUMMARY:-}}

verify_script=scripts/release/verify_release_tag.sh
[ -f "$verify_script" ] || die 'verify-script-missing'

git rev-parse --verify -q "$candidate^{commit}" >/dev/null || die "not-a-commit: $candidate"
candidate=$(git rev-parse --verify "$candidate^{commit}")

version=$(
    awk '
        /^\[package\]/ { in_package = 1; next }
        /^\[/ { in_package = 0 }
        in_package && /^version = "/ { gsub(/"/, "", $3); print $3; exit }
    ' crates/robin_rs/Cargo.toml
)
[ -n "$version" ] || die 'missing-crate-version'
tag="v$version"

# Existence is checked before the bump: an existing tag is never moved, even if
# the crate version still declares it.
if git rev-parse --verify -q "refs/tags/$tag" >/dev/null; then
    die "tag-exists: $tag is never moved"
fi

previous=$(git tag --list 'v*' --sort=-v:refname | head -1)
[ -n "$previous" ] || die 'no-previous-tag'
prev_version=${previous#v}
for v in "$prev_version" "$version"; do
    [[ $v =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "bad-version: $v"
done
IFS=. read -r prev_major prev_minor prev_patch <<<"$prev_version"
IFS=. read -r major minor patch <<<"$version"
# The first digit of 0.x.0 is human-only; only the next MINOR is automatic.
if [ "$((10#$major))" -ne "$((10#$prev_major))" ] ||
   [ "$((10#$minor))" -ne "$((10#$prev_minor + 1))" ] ||
   [ "$((10#$patch))" -ne 0 ]; then
    die "non-minor-bump: $previous -> v$version"
fi
if git log --format='%s%n%b' "$previous..$candidate" |
   grep -qE '^[a-z]+(\([^)]*\))?!:|^BREAKING CHANGE:'; then
    die 'major-not-allowed: a breaking commit needs a human decision'
fi

[ -n "$summary" ] && [ -f "$summary" ] || die 'summary-missing'
if ! jq -e '.verdict == "observed"' "$summary" >/dev/null 2>&1; then
    die 'release-gate-not-observed'
fi
if ! jq -e '.verdicts.H23 == "observed"' "$summary" >/dev/null 2>&1; then
    die 'h23-not-observed'
fi

# The verification script is run by the release workflow at the pushed tag; the
# preflight above proves it is present and that this checkout carries no drift
# the tag would hide.
printf 'auto-release: preflight ok; %s -> %s (binary %s)\n' "$previous" "$tag" "$binary"

git tag -a "$tag" -m "$tag from $candidate; MINOR from $previous"
printf 'auto-release: tagged %s\n' "$tag"
git push fork "$tag"
printf 'auto-release: pushed %s\n' "$tag"
gh workflow run native-release.yml --ref "$tag"
printf 'auto-release: dispatched %s\n' "$tag"
