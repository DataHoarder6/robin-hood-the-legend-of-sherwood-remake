#!/usr/bin/env bash
# Both tag-driven publishers must derive the artifact short from the commit,
# not from the annotated tag object. The fixture uses a real annotated tag so
# the two ids are demonstrably different, and each script's own derivation is
# evaluated against it (the assertion can fail when the script changes).
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid
git -C "$work" init -q -b main
printf 'fixture\n' >"$work/file"
git -C "$work" add file
git -C "$work" commit -qm fixture
git -C "$work" tag -a v0.0.0-test -m v0.0.0-test

tag_object=$(git -C "$work" rev-parse --short=12 v0.0.0-test)
commit_short=$(git -C "$work" rev-parse --short=12 'v0.0.0-test^{commit}')
[ "$tag_object" != "$commit_short" ] || {
    echo 'publisher-short: fixture tag object and commit coincide' >&2
    exit 1
}

for script in scripts/deploy_web_jautis.sh scripts/release-parity-check.sh; do
    line=$(grep -E '^short=\$\(git .*rev-parse --short=12 ' "$root/$script") || {
        echo "publisher-short: $script has no short derivation" >&2
        exit 1
    }
    case $line in
        *'^{commit}'*) ;;
        *)
            echo "publisher-short: $script derives the short from the tag object: $line" >&2
            exit 1
            ;;
    esac
    repo=$work tag=v0.0.0-test
    command=${line#short=\$\(}
    command=${command%)}
    derived=$(cd "$work" && eval "$command")
    [ "$derived" = "$commit_short" ] || {
        echo "publisher-short: $script derived $derived, expected $commit_short" >&2
        exit 1
    }
    printf 'ok: %s derives %s (tag object %s)\n' "$script" "$derived" "$tag_object"
done

echo 'publisher-short: all checks passed'
