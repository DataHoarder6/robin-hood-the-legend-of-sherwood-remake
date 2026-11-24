#!/usr/bin/env bash
# The autonomous work queue: print the next ready item, generate proposals when
# the queue is empty, promote only low-risk items, and record reviewer verdicts.
# Records are pipe-delimited lines in a fenced block of the backlog doc:
#
#   ID|CLASS|KIND|SCALE|TITLE|PATHS
#
# CLASS is proposed|ready|needs-work|rejected. The loop may promote docs, tests
# and small refactors only, and never a protected PATHS entry. Gameplay, schema,
# protocol, release, deletion and version items stay proposed for the owner.
# Proposals come from curated IDEA sources, never a deferred defect row.
# Local files, no gate or network.
set -euo pipefail

file=${ROBIN_BACKLOG_FILE:-docs/AUTONOMOUS_BACKLOG.md}

usage() {
    printf 'usage: autobacklog.sh next-ready | next-ready-after <id> | generate-proposed [title] | promote <item> | verdict <item> needs-work|rejected\n' >&2
    exit 2
}
die() { printf 'autobacklog: %s\n' "$*"; exit 1; }

[ -f "$file" ] || die "missing-backlog-file: $file"

records() { grep -E '^P[0-9]+\|' "$file" || true; }
find_item() { records | grep -F -m1 "$1|" || true; }
# Curated low-risk work: one `IDEA|title|paths` line per docs/test gap. Deferred
# DEFECTS.md rows are never a source: a known flake must never be re-fixed.
sources() { grep '^IDEA|' "$file" || true; }

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

# A path outside the loop's authority is never promotable, whatever the kind
# or scale. Mirrors the risk-routing deny list in docs/UNATTENDED.md.
paths_safe() { # PATHS
    local -a tokens=()
    local p
    [ -n "$1" ] || return 1
    IFS=', ' read -r -a tokens <<<"$1"
    for p in "${tokens[@]}"; do
        p=${p#./}
        [ -n "$p" ] || continue
        case $p in
            scripts/land.sh | scripts/remote-check.sh | scripts/push-gate.sh | \
            scripts/gate-stamp.sh | scripts/release*.sh | \
            .github | .github/* | *.nix | \
            .env | .env* | *.env | */.env* | *secret* | */secrets/* )
                return 1 ;;
        esac
    done
    return 0
}

promotable() { # KIND SCALE PATHS -> 0 when the loop may promote
    paths_safe "$3" || return 1
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
        IFS='|' read -r _ class kind scale _ paths <<<"$line"
        [ "$class" = proposed ] || die "not-proposed: $id is $class"
        promotable "$kind" "$scale" "$paths" ||
            die "not-promotable: $kind/$scale PATHS '$paths' (protected path or high-risk kind/scale)"
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
        paths=docs/AUTONOMOUS_BACKLOG.md
        if [ -z "$title" ]; then
            # Only a curated docs/test gap may become loop work. A deferred
            # DEFECTS.md row (notably a known flake) is never a source.
            existing_titles=$(records | awk -F'|' '{ print $5 }')
            candidate=$(sources | while IFS='|' read -r _ candidate_title candidate_paths; do
                printf '%s\n' "$existing_titles" | grep -F -x -q "$candidate_title" && continue
                printf '%s|%s\n' "$candidate_title" "$candidate_paths"
                break
            done)
            if [ -z "$candidate" ]; then
                die "no-proposal-source: add a low-risk docs/test gap as an IDEA|title|paths line in $file"
            fi
            title=${candidate%%|*}
            paths=${candidate#*|}
        fi
        # Pipes would break the record format; a title or path with one is invalid.
        case $title in *'|'*) die 'invalid-title: no pipe allowed' ;; esac
        case $paths in *'|'*) die 'invalid-paths: no pipe allowed' ;; esac
        [ -n "$paths" ] || die 'invalid-paths: empty'
        max=0
        while IFS='|' read -r rid _; do
            n=${rid#P}
            n=$((10#$n))
            [ "$n" -gt "$max" ] && max=$n
        done < <(records)
        next=$(printf 'P%03d' "$((max + 1))")
        record="$next|proposed|docs|small|$title|$paths"
        printf '%s\n' "$record" >>"$file"
        printf '%s\n' "$record"
        ;;
    *) usage ;;
esac
