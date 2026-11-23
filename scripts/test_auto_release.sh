#!/usr/bin/env bash
# Fixture tests for scripts/auto-release.sh: local git repos, a stub `gh`, and a
# jq-readable summary. No build host, no remote gate, no GitHub, no install.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-auto-release.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

export GIT_AUTHOR_NAME=fixture GIT_AUTHOR_EMAIL=fixture@example.invalid
export GIT_COMMITTER_NAME=fixture GIT_COMMITTER_EMAIL=fixture@example.invalid

stubs=$work/stubs
mkdir -p "$stubs"
cat >"$stubs/gh" <<'GH'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$FAKE_GH_LOG"
GH
chmod +x "$stubs/gh"
export FAKE_GH_LOG=$work/gh.log

pass_count=0
failures=0
pass() { printf 'ok   %s\n' "$1"; pass_count=$((pass_count + 1)); }
fail() {
    printf 'FAIL %s\n' "$1"
    failures=$((failures + 1))
}
expect_status() {
    if [ "$status" -eq "$1" ]; then pass "$2"; else fail "$2 (exit $status, want $1): $output"; fi
}
expect_reason() {
    case $output in
        *"$2"*) pass "$1" ;;
        *) fail "$1 (missing '$2'): $output" ;;
    esac
}
expect_absent() {
    case $output in
        *"$2"*) fail "$1 (unexpected '$2'): $output" ;;
        *) pass "$1" ;;
    esac
}

REPO=''
summary=$work/summary.json
fixture() { # NAME VERSION [EXISTING_TAG]
    local name=$1 version=$2 existing=${3:-}
    REPO=$work/$name
    mkdir -p "$REPO/crates/robin_rs" "$REPO/scripts/release"
    git init --quiet -b main "$REPO"
    printf '[package]\nname = "robin_rs"\nversion = "%s"\n' "$version" \
        >"$REPO/crates/robin_rs/Cargo.toml"
    git -C "$REPO" add -- crates/robin_rs/Cargo.toml
    git -C "$REPO" commit -qm 'chore: base' -- crates/robin_rs/Cargo.toml
    git -C "$REPO" tag v0.4.0
    if [ -n "$existing" ]; then git -C "$REPO" tag "$existing"; fi
    git init --quiet --bare "$REPO.git"
    git -C "$REPO" remote add fork "$REPO.git"
    git -C "$REPO" push --quiet "$REPO.git" HEAD:refs/heads/main
    git -C "$REPO" update-ref refs/remotes/fork/main HEAD
    cp -- "$source_root/scripts/release/verify_release_tag.sh" "$REPO/scripts/release/"
    printf '{"verdict":"observed","verdicts":{"H23":"observed"}}\n' >"$summary"
    : >"$FAKE_GH_LOG"
}

run() {
    status=0
    output=$(cd "$REPO" && PATH="$stubs:$PATH" \
        bash "$source_root/scripts/auto-release.sh" "$@" 2>&1) || status=$?
}
remote_tag() { git -C "$REPO.git" rev-parse -q --verify "refs/tags/$1" 2>/dev/null || true; }

# 1. The happy path tags, pushes and dispatches, in that order.
fixture happy 0.5.0
candidate=$(git -C "$REPO" rev-parse HEAD)
run "$candidate" "$work/robin" "$summary"
expect_status 0 'a MINOR release succeeds'
expect_reason 'the tag is named' 'v0.5.0'
expect_reason 'the dispatch is reported' 'dispatched'
if [ -n "$(remote_tag v0.5.0)" ]; then pass 'the tag reached fork'; else fail 'the tag did not reach fork'; fi
if grep -q 'workflow run native-release.yml --ref v0.5.0' "$FAKE_GH_LOG"; then
    pass 'the native-release workflow was dispatched at the tag'
else
    fail "the dispatch was wrong: $(cat "$FAKE_GH_LOG")"
fi
order=$(printf '%s\n' "$output" | grep -nE 'tagged|pushed|dispatched' | tr '\n' ' ' || true)
case $order in *tagged*pushed*dispatched*) pass 'tag, push, dispatch run in order' ;; *) fail "the order is wrong: $order" ;; esac

# 1b. The tag names the audited candidate, not a later HEAD.
fixture headnotdef 0.5.0
audited=$(git -C "$REPO" rev-parse HEAD)
printf 'later\n' >"$REPO/later.txt"
git -C "$REPO" add -- later.txt
git -C "$REPO" commit -qm 'docs: later un audited work' -- later.txt
run "$audited" "$work/robin" "$summary"
expect_status 0 'a release of an earlier candidate succeeds'
tagged_commit=$(git -C "$REPO" rev-parse 'v0.5.0^{commit}' 2>/dev/null || true)
if [ "$tagged_commit" = "$audited" ]; then
    pass 'the tag points at the audited candidate'
else
    fail "the tag points at $tagged_commit, not $audited"
fi
if [ "$tagged_commit" != "$(git -C "$REPO" rev-parse HEAD)" ]; then
    pass 'the tag does not point at HEAD'
else
    fail 'the tag silently moved to HEAD'
fi

# 2. An existing tag is never moved.
fixture exists 0.5.0 v0.5.0
run "$(git -C "$REPO" rev-parse HEAD)" "$work/robin" "$summary"
expect_status 1 'an existing tag is refused'
expect_reason 'the reason is named' 'tag-exists'
expect_absent 'no dispatch happens' 'dispatched'
if [ -z "$(cat "$FAKE_GH_LOG")" ]; then pass 'no workflow was dispatched'; else fail 'a workflow was dispatched'; fi

# 3. Only a MINOR bump is allowed: patch and major are both refused.
fixture patch 0.4.1
run "$(git -C "$REPO" rev-parse HEAD)" "$work/robin" "$summary"
expect_status 1 'a PATCH bump is refused'
expect_reason 'the reason is named' 'non-minor-bump'
fixture major 1.0.0
run "$(git -C "$REPO" rev-parse HEAD)" "$work/robin" "$summary"
expect_status 1 'a MAJOR bump is refused'
expect_reason 'the reason is named' 'non-minor-bump'

# 4. A breaking commit since the last tag needs a human decision.
fixture breaking 0.5.0
printf 'boom\n' >"$REPO/breaking.txt"
git -C "$REPO" add -- breaking.txt
git -C "$REPO" commit -qm 'feat!: change the invite format' -- breaking.txt
run "$(git -C "$REPO" rev-parse HEAD)" "$work/robin" "$summary"
expect_status 1 'a breaking change is refused'
expect_reason 'the reason is named' 'major-not-allowed'

# 5. The release gate evidence is mandatory and authoritative.
fixture nogate 0.5.0
run "$(git -C "$REPO" rev-parse HEAD)" "$work/robin" "$work/absent.json"
expect_status 1 'a missing summary is refused'
expect_reason 'the reason is named' 'summary-missing'
printf '{"verdict":"failed","verdicts":{"H23":"observed"}}\n' >"$summary"
run "$(git -C "$REPO" rev-parse HEAD)" "$work/robin" "$summary"
expect_status 1 'a failed suite verdict is refused'
expect_reason 'the reason is named' 'release-gate-not-observed'
printf '{"verdict":"observed","verdicts":{"H23":"failed"}}\n' >"$summary"
run "$(git -C "$REPO" rev-parse HEAD)" "$work/robin" "$summary"
expect_status 1 'H23 not observed is refused'
expect_reason 'the reason is named' 'h23-not-observed'
if [ -n "$(remote_tag v0.5.0)" ]; then fail 'a refused release still pushed a tag'; else pass 'a refused release pushes no tag'; fi

# 6. The verification preflight is part of the contract.
fixture unverified 0.5.0
rm -- "$REPO/scripts/release/verify_release_tag.sh"
run "$(git -C "$REPO" rev-parse HEAD)" "$work/robin" "$summary"
expect_status 1 'a missing verification script is refused'
expect_reason 'the reason is named' 'verify-script-missing'

# 7. The release stays inside this repo: no pin, no install, no seat action.
script=$source_root/scripts/auto-release.sh
if grep -qE 'apps-sync|flake\.nix|nixos-config|workflow_dispatch|curl |wget |just apps' "$script"; then
    fail 'auto-release reaches outside the repo'
else
    pass 'auto-release never touches the pin or an install'
fi
if grep -q 'git tag -a' "$script" && grep -q 'git push fork' "$script" &&
   grep -q 'gh workflow run native-release.yml' "$script"; then
    pass 'the three hardened release commands are called'
else
    fail 'a release command is missing'
fi

# 8. The doc amendments and the new ADR exist and cross-link.
for pair in \
    "docs/UNATTENDED.md|0011-autonomous-release.md" \
    ".opencode/agents/director.md|0011-autonomous-release.md" \
    "docs/RELEASES.md|auto-release.sh"; do
    path=${pair%%|*}
    term=${pair#*|}
    if grep -q "$term" "$source_root/$path"; then
        pass "$path points at $term"
    else
        fail "$path does not point at $term"
    fi
done
if [ -f "$source_root/docs/adr/0011-autonomous-release.md" ]; then
    pass 'ADR 0011 exists'
else
    fail 'ADR 0011 is missing'
fi

printf '\n%s checks passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
