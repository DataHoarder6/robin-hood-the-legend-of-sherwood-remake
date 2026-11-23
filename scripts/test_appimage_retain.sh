#!/usr/bin/env bash
# Fixture tests for scripts/appimage-retain.sh and the release-worthy predicate
# in scripts/batch-controller.sh. Local dirs and local git repos only; no gate,
# no build host, no network, no install.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-appimage-retain.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

export GIT_AUTHOR_NAME=fixture GIT_AUTHOR_EMAIL=fixture@example.invalid
export GIT_COMMITTER_NAME=fixture GIT_COMMITTER_EMAIL=fixture@example.invalid

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

DIR=''
PIN=''
PREV=''
images() { find "$DIR" -maxdepth 1 -name '*.AppImage' -printf '%f\n' | sort | tr '\n' ' '; }
fixture_images() { # NAME ...
    local name=$1
    shift
    DIR=$work/$name
    PIN=$work/$name.pin
    PREV=$work/$name.previous-good
    mkdir -p "$DIR"
    export ROBIN_PIN_FILE=$PIN ROBIN_PREVIOUS_GOOD_FILE=$PREV
    local f
    for f in "$@"; do printf '%s\n' "$f" >"$DIR/$f"; done
}

run_retain() {
    status=0
    output=$(bash "$source_root/scripts/appimage-retain.sh" "$@" 2>&1) || status=$?
}

# 1. Exactly two known-good AppImages survive: the pin and previous-good.
fixture_images basic \
    robin-0.4.0.AppImage robin-0.3.0.AppImage robin-0.2.0.AppImage robin-0.1.0.AppImage
printf 'robin-0.4.0.AppImage\n' >"$PIN"
printf 'robin-0.3.0.AppImage\n' >"$PREV"
run_retain retain "$DIR"
expect_status 0 'retention succeeds'
if [ "$(images)" = 'robin-0.3.0.AppImage robin-0.4.0.AppImage ' ]; then
    pass 'exactly the two known-good images remain'
else
    fail "the remaining images are: $(images)"
fi
expect_reason 'the survivors are reported' 'robin-0.4.0.AppImage'

# 2. The pinned image is never deleted, even when it is the oldest.
fixture_images pinned \
    robin-0.4.0.AppImage robin-0.3.0.AppImage robin-0.2.0.AppImage robin-0.1.0.AppImage
printf 'robin-0.1.0.AppImage\n' >"$PIN"
printf 'robin-0.4.0.AppImage\n' >"$PREV"
run_retain retain "$DIR"
expect_status 0 'retention with an old pin succeeds'
if [ -f "$DIR/robin-0.1.0.AppImage" ]; then pass 'the pinned image survives'; else fail 'the pinned image was deleted'; fi
if [ -f "$DIR/robin-0.4.0.AppImage" ]; then pass 'the previous-good image survives'; else fail 'the previous-good image was deleted'; fi

# 3. Missing pointers and missing images fail closed before any deletion.
fixture_images nopin robin-0.4.0.AppImage robin-0.3.0.AppImage robin-0.2.0.AppImage
printf 'robin-0.3.0.AppImage\n' >"$PREV"
run_retain retain "$DIR"
expect_status 1 'a missing pin is refused'
expect_reason 'the reason is named' 'pin-missing'
if [ "$(images)" = 'robin-0.2.0.AppImage robin-0.3.0.AppImage robin-0.4.0.AppImage ' ]; then
    pass 'no image was deleted on a missing pin'
else
    fail "images were deleted on a missing pin: $(images)"
fi

fixture_images pinnedgone robin-0.4.0.AppImage robin-0.2.0.AppImage
printf 'robin-0.9.9.AppImage\n' >"$PIN"
printf 'robin-0.4.0.AppImage\n' >"$PREV"
run_retain retain "$DIR"
expect_status 1 'a missing pinned image is refused'
expect_reason 'the reason is named' 'pinned-missing'

fixture_images prevgone robin-0.4.0.AppImage robin-0.2.0.AppImage
printf 'robin-0.4.0.AppImage\n' >"$PIN"
run_retain retain "$DIR"
expect_status 1 'a missing previous-good pointer is refused'
expect_reason 'the reason is named' 'previous-good-missing'

# 4. Dry run announces deletions without making them.
fixture_images dry robin-0.4.0.AppImage robin-0.3.0.AppImage robin-0.2.0.AppImage
printf 'robin-0.4.0.AppImage\n' >"$PIN"
printf 'robin-0.3.0.AppImage\n' >"$PREV"
ROBIN_RETAIN_DRY_RUN=1 run_retain retain "$DIR"
expect_status 0 'a dry run succeeds'
expect_reason 'the dry run names its action' 'would-delete'
if [ -f "$DIR/robin-0.2.0.AppImage" ]; then pass 'the dry run deleted nothing'; else fail 'the dry run deleted an image'; fi

# 5. record moves previous-good only; rollback names the target and never the pin.
fixture_images record robin-0.4.0.AppImage robin-0.3.0.AppImage robin-0.2.0.AppImage
printf 'robin-0.4.0.AppImage\n' >"$PIN"
printf 'robin-0.3.0.AppImage\n' >"$PREV"
run_retain record robin-0.2.0.AppImage "$DIR"
expect_status 0 'record succeeds'
if [ "$(cat "$PREV")" = 'robin-0.2.0.AppImage' ]; then pass 'previous-good was updated'; else fail 'previous-good was not updated'; fi
if [ "$(cat "$PIN")" = 'robin-0.4.0.AppImage' ]; then pass 'the pin was not touched'; else fail 'the pin was changed'; fi
run_retain rollback "$DIR"
expect_status 0 'rollback prints its target'
expect_reason 'the target is named' 'robin-0.2.0.AppImage'
if [ "$(cat "$PIN")" = 'robin-0.4.0.AppImage' ]; then pass 'rollback never edits the pin'; else fail 'rollback edited the pin'; fi

# 6. The owner-owned pointers exist in the repo.
for pointer in docs/releases/robin-pin docs/releases/previous-good; do
    if [ -f "$source_root/$pointer" ]; then pass "$pointer exists"; else fail "$pointer is missing"; fi
done

# 7. Release-worthy is derived from Conventional Commits in the batch.
BREPO=''
batch_fixture() { # NAME SUBJECT...
    local name=$1
    shift
    BREPO=$work/batch-$name
    export ROBIN_BATCH_REPO=$BREPO
    export ROBIN_BATCH_STATE=$work/batch-$name.state
    git init --quiet -b main "$BREPO"
    printf 'base\n' >"$BREPO/base.txt"
    git -C "$BREPO" add -- base.txt
    git -C "$BREPO" commit -qm 'chore: base' -- base.txt
    git -C "$BREPO" update-ref refs/remotes/fork/main HEAD
    local i=0 subject file
    for subject in "$@"; do
        i=$((i + 1))
        file=m$i.txt
        git -C "$BREPO" checkout -q -b "a$i" main
        printf '%s\n' "$subject" >"$BREPO/$file"
        git -C "$BREPO" add -- "$file"
        git -C "$BREPO" commit -qm "$subject" -- "$file"
        git -C "$BREPO" checkout -q main
        bash "$source_root/scripts/batch-controller.sh" ready "a$i" \
            "$(git -C "$BREPO" rev-parse "a$i")" >/dev/null
    done
    local out
    out=$(ROBIN_BATCH_MIN=1 ROBIN_BATCH_MAX=5 bash \
        "$source_root/scripts/batch-controller.sh" epoch)
    BATCH_EPOCH=$(printf '%s\n' "$out" | awk '{print $2}')
}
run_worthy() {
    status=0
    output=$(bash "$source_root/scripts/batch-controller.sh" release-worthy "$BATCH_EPOCH" 2>&1) || status=$?
}
batch_fixture feat 'fix: correct the bow'
run_worthy
expect_status 0 'a fix batch is release-worthy'
expect_reason 'the bump is MINOR' 'minor'
batch_fixture docs 'docs: update a guide' 'test: add a fixture'
run_worthy
expect_status 0 'a docs/test batch is not release-worthy'
expect_reason 'the bump is none' 'none'
batch_fixture breaking 'feat!: change the invite format'
run_worthy
expect_status 1 'a breaking batch is refused'
expect_reason 'the reason is named' 'major-not-allowed'

printf '\n%s checks passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
