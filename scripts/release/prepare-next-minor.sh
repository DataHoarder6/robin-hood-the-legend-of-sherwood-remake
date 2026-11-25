#!/usr/bin/env bash
# Prepare the next MINOR release from landed sources: bump the crate versions,
# add the tag's whats-new entry, commit, tag locally and regenerate the notes.
# REFUSES a patch bump, a missing source, or a version that disagrees with the
# latest tag. It never moves or deletes a tag.
#
#   scripts/release/prepare-next-minor.sh [--repo DIR]
#
# stdout: the new version (e.g. 0.6.0)
set -euo pipefail

repo=$(pwd)
while (( $# )); do
    case $1 in
        --repo) repo=${2:?--repo needs a directory}; shift 2 ;;
        *) printf 'prepare-next-minor: unknown option %s\n' "$1" >&2; exit 2 ;;
    esac
done
cd -- "$repo"

fail() { printf 'prepare-next-minor: %s\n' "$1" >&2; exit 1; }

previous=$(git tag --list 'v[0-9]*.[0-9]*.[0-9]*' --sort=-v:refname | head -1)
[ -n "$previous" ] || fail 'no-previous-tag'
version=${previous#v}
IFS=. read -r major minor patch <<<"$version"
[[ "$major" =~ ^[0-9]+$ && "$minor" =~ ^[0-9]+$ && "$patch" =~ ^[0-9]+$ ]] || fail 'invalid-previous-tag'
[ "$patch" = 0 ] || fail 'previous-tag-is-not-a-minor'
next="$major.$((minor + 1)).0"
tag="v$next"

manifest=docs/releases/whats-new.json

# A retry after a successful preparation is a no-op: the bump commit, its tag
# and its manifest entry all name the same version.
if [ "$(git log -1 --format=%s)" = "release: bump to $version" ] &&
   grep -q "\"v$version\"" "$manifest" &&
   grep -q "^version = \"$version\"\$" crates/robin_rs/Cargo.toml 2>/dev/null; then
    printf '%s\n' "$version"
    exit 0
fi
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    fail 'candidate-tag-exists-with-unprepared-tree'
fi
cargo=crates/robin_rs/Cargo.toml
[ -f "$manifest" ] || fail 'no-whats-new-manifest'
[ -f "$cargo" ] || fail 'no-crate-manifest'
grep -q "^version = \"$version\"\$" "$cargo" || fail 'version-precondition'
grep -q "^version = \"$version\"\$" Cargo.lock || fail 'lockfile-precondition'

mapfile -t commits < <(git log --format='%H' "$previous"..HEAD | cut -c1-12)
(( ${#commits[@]} > 0 )) || fail 'no-landed-source'

python3 -B - "$version" "$next" "$cargo" Cargo.lock "$manifest" "${commits[@]}" <<'PY'
import json
import pathlib
import re
import sys

previous, next_version, cargo_path, lock_path, manifest_path, *commits = sys.argv[1:]
for path in (pathlib.Path(cargo_path), pathlib.Path(lock_path)):
    body = path.read_text(encoding="utf-8")
    updated, count = re.subn(
        rf'^version = "{re.escape(previous)}"$',
        f'version = "{next_version}"',
        body,
        count=1,
        flags=re.MULTILINE,
    )
    if count != 1:
        raise SystemExit(f"prepare-next-minor: could not bump {path}")
    path.write_text(updated, encoding="utf-8")
manifest = json.loads(pathlib.Path(manifest_path).read_text(encoding="utf-8"))
if f"v{next_version}" in manifest:
    raise SystemExit("prepare-next-minor: manifest already names the next tag")
manifest[f"v{next_version}"] = {"new_features": [], "commits": commits}
pathlib.Path(manifest_path).write_text(
    json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
PY

git add -- "$cargo" Cargo.lock "$manifest"
git commit --quiet -m "release: bump to $next"
git tag -a "$tag" -m "$tag"

notes=${ROBIN_RELEASE_NOTES:-$repo/scripts/release-notes.sh}
bash "$notes" whats-new --repo "$repo" || fail 'notes-generation-failed'
printf '%s\n' "$next"
