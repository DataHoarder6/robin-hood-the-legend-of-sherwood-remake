#!/usr/bin/env bash
# The autonomous work queue: print the next ready item, generate proposals when
# the queue is empty, promote only low-risk items, and record reviewer verdicts.
# Records are pipe-delimited lines in a fenced block of the backlog doc:
#
#   ID|CLASS|KIND|SCALE|TITLE|PATHS
#
# CLASS is proposed|ready|needs-work|rejected. The loop may promote docs, tests
# and small refactors only; gameplay, schema, protocol, release, deletion and
# version items stay proposed for the owner. Local files, no gate or network.
set -euo pipefail

file=${ROBIN_BACKLOG_FILE:-docs/AUTONOMOUS_BACKLOG.md}
defects=${ROBIN_DEFECTS_FILE:-docs/DEFECTS.md}

usage() {
    printf 'usage: autobacklog.sh next-ready | next-ready-after <id> | generate-proposed [title] | promote <item> | verdict <item> needs-work|rejected\n' >&2
    exit 2
}
die() { printf 'autobacklog: %s\n' "$*"; exit 1; }

[ -f "$file" ] || die "missing-backlog-file: $file"

records() { grep -E '^P[0-9]+\|' "$file" || true; }
find_item() { records | grep -F -m1 "$1|" || true; }

# Replace the class field of one record, failing closed if the id is absent.
rewrite() { # ID CLASS
    local id=$1 class=$2 tmp
    tmp=$(mktemp "${TMPDIR:-/tmp}/robin-autobacklog.XXXXXX")
    awk -F'|' -v id="$id" -v class="$class" '
        BEGIN { OFS = "|" }
        $1 ~ /^P[0-9]+$/ && $1 == id { $2 = class; found = 1 }
        { print }
        END { if (!found) exit 3 }
    ' "$file" >"$tmp" || { rm -f -- "$tmp"; die "unknown-item: $id"; }
    cat -- "$tmp" >"$file"
    rm -f -- "$tmp"
}

promotable() { # KIND SCALE -> 0 when the loop may promote
    case $1 in
        docs | tests) return 0 ;;
        refactor) [ "$2" = small ] ;;
        *) return 1 ;;
    esac
}

case "${1:-}" in
    next-ready)
        [ $# -eq 1 ] || usage
        line=$(records | awk -F'|' '$2 == "ready" { print; exit }')
        [ -n "$line" ] || die 'no-ready-item'
        printf '%s\n' "$line"
        ;;
    next-ready-after)
        # Resume cursor: the first ready record AFTER the named id in file
        # order. The id need not still be ready; it only has to exist.
        [ $# -eq 2 ] || usage
        line=$(records | awk -F'|' -v after="$2" '
            $1 == after { seen = 1; next }
            seen && $2 == "ready" { print; exit }
        ')
        [ -n "$line" ] || die 'no-ready-item'
        printf '%s\n' "$line"
        ;;
    promote)
        [ $# -eq 2 ] || usage
        id=$2
        line=$(find_item "$id")
        [ -n "$line" ] || die "unknown-item: $id"
        IFS='|' read -r _ class kind scale _ <<<"$line"
        [ "$class" = proposed ] || die "not-proposed: $id is $class"
        promotable "$kind" "$scale" || die "not-promotable: $kind/$scale"
        rewrite "$id" ready
        find_item "$id"
        ;;
    verdict)
        [ $# -eq 3 ] || usage
        id=$2 verdict=$3
        case $verdict in
            needs-work | rejected) ;;
            *) die "unknown-verdict: $verdict" ;;
        esac
        [ -n "$(find_item "$id")" ] || die "unknown-item: $id"
        rewrite "$id" "$verdict"
        find_item "$id"
        ;;
    generate-proposed)
        [ $# -le 2 ] || usage
        title=${2:-}
        if [ -z "$title" ]; then
            [ -f "$defects" ] || die "no-proposal-source: $defects is missing"
            id_from_defect=$(awk -F'|' '/deferred/ {
                for (i = 1; i <= NF; i++) {
                    gsub(/^[ \t]+|[ \t]+$/, "", $i)
                    if ($i ~ /^D[0-9]+$/) { print $i; exit }
                }
            }' "$defects")
            [ -n "$id_from_defect" ] || die 'no-proposal-source: no deferred defect row'
            title="Deferred $id_from_defect from DEFECTS.md"
        fi
        # Pipes would break the record format; a title with one is invalid.
        case $title in *'|'*) die 'invalid-title: no pipe allowed' ;; esac
        max=0
        while IFS='|' read -r rid _; do
            n=${rid#P}
            n=$((10#$n))
            [ "$n" -gt "$max" ] && max=$n
        done < <(records)
        next=$(printf 'P%03d' "$((max + 1))")
        printf '%s|proposed|docs|small|%s|docs/AUTONOMOUS_BACKLOG.md\n' \
            "$next" "$title" >>"$file"
        printf '%s|proposed|docs|small|%s|docs/AUTONOMOUS_BACKLOG.md\n' \
            "$next" "$title"
        ;;
    *) usage ;;
esac
