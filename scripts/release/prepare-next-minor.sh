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
cargo=crates/robin_rs/Cargo.toml
[ -f "$manifest" ] || fail 'no-whats-new-manifest'
[ -f "$cargo" ] || fail 'no-crate-manifest'

# A retry after a successful preparation is a no-op: HEAD carries the tag,
# the manifest entry and the version files all name the same release.
head_tag=$(git tag --points-at HEAD | grep '^v[0-9]' | sort -V | tail -1 || true)
if [ -n "$head_tag" ] &&
   grep -q "\"$head_tag\"" "$manifest" &&
   grep -q "^version = \"${head_tag#v}\"\$" "$cargo"; then
    printf '%s\n' "${head_tag#v}"
    exit 0
fi
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    fail 'candidate-tag-exists-with-unprepared-tree'
fi
current=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$cargo" | head -1)
lock_version=$(awk '
    /^\[\[package\]\]/ { name = "" }
    /^name = / { gsub(/"/, "", $3); name = $3 }
    /^version = / && name == "robin_rs" { gsub(/"/, "", $3); print $3; exit }
' Cargo.lock)
case $current in
    "$version") staged=0 ;;
    "$next") staged=1 ;;
    *) fail 'version-precondition' ;;
esac
[ "$lock_version" = "$current" ] || fail 'lockfile-precondition'

mapfile -t commits < <(git log --format='%H' "$previous"..HEAD | cut -c1-12)
(( ${#commits[@]} > 0 )) || fail 'no-landed-source'

python3 -B - "$version" "$next" "$staged" "$cargo" Cargo.lock "$manifest" "${commits[@]}" <<'PY'
import json
import pathlib
import re
import sys

previous, next_version, staged, cargo_path, lock_path, manifest_path, *commits = sys.argv[1:]
if staged == "0":
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

if [ "$staged" = 0 ]; then
    git add -- "$cargo" Cargo.lock "$manifest"
    git commit --quiet -m "release: bump to $next"
else
    git add -- "$manifest"
fi
git tag -a "$tag" -m "$tag"

notes=${ROBIN_RELEASE_NOTES:-$repo/scripts/release-notes.sh}
bash "$notes" whats-new --repo "$repo" || fail 'notes-generation-failed'
printf '%s\n' "$next"
