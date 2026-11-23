#!/usr/bin/env bash
# Keep exactly two known-good AppImages: the owner's robin-pin and the
# previous-good rollback target. The loop may prune and record; it may never
# edit the pin or install anything. Pointers hold one filename on the first
# non-comment line. No network, no build host.
#
#   scripts/appimage-retain.sh retain <dir>
#   scripts/appimage-retain.sh record <file> <dir>
#   scripts/appimage-retain.sh rollback <dir>
#
# Environment: ROBIN_PIN_FILE, ROBIN_PREVIOUS_GOOD_FILE (default <dir>/robin-pin
# and <dir>/previous-good), ROBIN_RETAIN_DRY_RUN=1 (announce, do not delete).
set -euo pipefail

usage() {
    printf 'usage: appimage-retain.sh retain|record|rollback ...\n' >&2
    exit 2
}
die() { printf 'appimage-retain: %s\n' "$*"; exit 1; }

pointer_value() { # FILE -> value, or non-zero when absent/empty
    [ -f "$1" ] || return 1
    local value
    value=$(grep -vE '^[[:space:]]*(#|$)' "$1" | head -1 || true)
    [ -n "$value" ] || return 1
    printf '%s' "$value"
}

command=${1:-}
[ $# -gt 0 ] || usage
case $command in
    retain)
        [ $# -eq 2 ] || usage
        dir=$2
        pin_file=${ROBIN_PIN_FILE:-$dir/robin-pin}
        prev_file=${ROBIN_PREVIOUS_GOOD_FILE:-$dir/previous-good}
        pinned=$(pointer_value "$pin_file") || die 'pin-missing'
        previous=$(pointer_value "$prev_file") || die 'previous-good-missing'
        [ -f "$dir/$pinned" ] || die "pinned-missing: $pinned"
        [ -f "$dir/$previous" ] || die "previous-good-image-missing: $previous"
        local_image=''
        for local_image in "$dir"/*.AppImage; do
            [ -e "$local_image" ] || continue
            name=${local_image##*/}
            if [ "$name" = "$pinned" ] || [ "$name" = "$previous" ]; then continue; fi
            if [ "${ROBIN_RETAIN_DRY_RUN:-}" = 1 ]; then
                printf 'would-delete %s\n' "$name"
            else
                rm -f -- "$local_image"
                printf 'deleted %s\n' "$name"
            fi
        done
        printf 'retained %s %s\n' "$pinned" "$previous"
        ;;
    record)
        [ $# -eq 3 ] || usage
        file=$2
        dir=$3
        prev_file=${ROBIN_PREVIOUS_GOOD_FILE:-$dir/previous-good}
        [ -f "$dir/$file" ] || die "image-missing: $file"
        case $file in */*) die "invalid-image-name: $file" ;; esac
        printf '%s\n' "$file" >"$prev_file"
        printf 'previous-good %s\n' "$file"
        ;;
    rollback)
        [ $# -eq 2 ] || usage
        dir=$2
        prev_file=${ROBIN_PREVIOUS_GOOD_FILE:-$dir/previous-good}
        target=$(pointer_value "$prev_file") || die 'previous-good-missing'
        printf 'rollback-target %s\n' "$target"
        printf 'rollback: robin-pin stays owner-owned; set it by hand\n'
        ;;
    *) usage ;;
esac
