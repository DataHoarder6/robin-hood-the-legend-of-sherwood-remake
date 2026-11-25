#!/usr/bin/env bash
# Fixture tests for mechanical next-MINOR preparation: synthetic repo, stub notes.
set -euo pipefail
source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
script=$source_root/scripts/release/prepare-next-minor.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-next-minor.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

export HOME=$work/home GIT_CONFIG_GLOBAL=$work/gitconfig GIT_CONFIG_NOSYSTEM=1
mkdir -p "$HOME"
git config --file "$GIT_CONFIG_GLOBAL" user.name Fixture
git config --file "$GIT_CONFIG_GLOBAL" user.email fixture@example.invalid
git config --file "$GIT_CONFIG_GLOBAL" commit.gpgsign false

repo=$work/repo
mkdir -p "$repo/docs/releases" "$repo/crates/robin_rs" "$repo/stub"
cd "$repo"
printf '[package]\nname = "robin_rs"\nversion = "0.5.0"\n' >crates/robin_rs/Cargo.toml
printf 'version = "0.5.0"\n' >Cargo.lock
printf '{\n  "v0.5.0": {"new_features": [], "commits": ["aaaaaaaaaaaaa"]}\n}\n' >docs/releases/whats-new.json
printf '# Features\n\n- **Rotating map**: turns the map.\n' >docs/NEW_FEATURES.md
printf '# Notes\n' >docs/RELEASE_NOTES.md
cat >stub/notes <<'STUB'
#!/usr/bin/env bash
printf 'generated\n' >docs/RELEASE_NOTES.md
STUB
chmod +x stub/notes
git init --quiet -b main
git add . && git commit --quiet -m base
git tag -a v0.5.0 -m 'v0.5.0'
printf 'change\n' >feature.txt
git add feature.txt && git commit --quiet -m 'feat: landed source'

export ROBIN_RELEASE_NOTES="$repo/stub/notes"
fail=0
pass() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1" >&2; fail=1; }

sha=$(git rev-parse --short=12 HEAD)
version=$(bash "$script" --repo "$repo" 2>"$work/err") || true
[[ "$version" == '0.6.0' ]] \
    && pass 'bump-is-minor-only' \
    || bad "bump-is-minor-only: got '$version' / $(cat "$work/err")"

grep -q 'version = "0.6.0"' crates/robin_rs/Cargo.toml \
    && grep -q 'version = "0.6.0"' Cargo.lock \
    && pass 'version-files-bumped' \
    || bad 'version-files-bumped'

grep -q '"v0.6.0"' docs/releases/whats-new.json \
    && pass 'manifest-entry-added' \
    || bad 'manifest-entry-added'

grep -q "$sha" docs/releases/whats-new.json \
    && pass 'notes-quote-landed-sources' \
    || bad "notes-quote-landed-sources: $sha absent"

git rev-parse -q --verify refs/tags/v0.6.0 >/dev/null \
    && pass 'candidate-tag-created-locally' \
    || bad 'candidate-tag-created-locally'

before=$(git rev-parse HEAD; sha256sum docs/releases/whats-new.json | cut -d' ' -f1)
second=$(bash "$script" --repo "$repo" 2>/dev/null || true)
after=$(git rev-parse HEAD; sha256sum docs/releases/whats-new.json | cut -d' ' -f1)
[[ "$second" == '0.6.0' && "$before" == "$after" ]] \
    && pass 'second-run-is-no-op' \
    || bad "second-run-is-no-op: '$second' / before=$before after=$after"

# No landed source between tags: fail closed instead of inventing a release.
repo2=$work/repo2
git clone --quiet --no-local "$repo" "$repo2"
cd "$repo2"
git checkout --quiet v0.5.0
git tag -d v0.6.0 >/dev/null
set +e
out=$(bash "$script" --repo "$repo2" 2>&1)
rc=$?
set -e
[[ $rc -ne 0 && "$out" == *'no-landed-source'* ]] \
    && pass 'missing-source-fails-closed' \
    || bad "missing-source-fails-closed: rc=$rc out=$out"

(( fail == 0 )) || exit 1
printf 'prepare-next-minor: OK\n'
