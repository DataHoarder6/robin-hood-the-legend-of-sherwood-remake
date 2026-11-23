#!/usr/bin/env bash
# The one source for the batch gate's green stamp: its path, its writer and its
# validity rule. Both the batch controller (writer) and land.sh (reader) call
# this helper, so a caller of land.sh can name a tree but cannot name a file.
#
#   gate-stamp.sh path <tree>
#   gate-stamp.sh write <tree> <base> green|red
#   gate-stamp.sh valid <tree> <base>       # green for exactly this tree and base
#
# The state root is XDG_STATE_HOME/HOME, never a per-call environment knob.
set -euo pipefail

usage() {
    printf 'usage: gate-stamp.sh path <tree> | write <tree> <base> green|red | valid <tree> <base>\n' >&2
    exit 2
}

stamp_dir=${XDG_STATE_HOME:-$HOME/.local/state}/robin-batch/gates

case "${1:-}" in
    path)
        [ $# -eq 2 ] || usage
        printf '%s/%s\n' "$stamp_dir" "$2"
        ;;
    write)
        [ $# -eq 4 ] || usage
        tree=$2 base=$3 verdict=$4
        case $verdict in green | red) ;; *) usage ;; esac
        mkdir -p -- "$stamp_dir"
        printf '%s\nbase %s\n' "$verdict" "$base" >"$stamp_dir/$tree"
        ;;
    valid)
        [ $# -eq 3 ] || usage
        tree=$2 base=$3 file=$stamp_dir/$2
        [ -f "$file" ] || exit 1
        [ "$(sed -n '1p' "$file")" = green ] || exit 1
        [ "$(sed -n '2p' "$file")" = "base $base" ] || exit 1
        ;;
    *) usage ;;
esac
