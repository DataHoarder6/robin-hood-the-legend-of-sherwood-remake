#!/usr/bin/env bash
# Re-read the pre-run contract's bounds before an autonomous tick, and refuse any
# edit to the contract itself. No gate, no network: this is a local parser whose
# named reasons are the interface (scripts/test_run_bounds.sh).
#
#   scripts/run-bounds.sh check <contract-file>
#   scripts/run-bounds.sh guard-edit <path>
#
# `check` prints one named reason per failed bound and exits 1, or prints `ok`
# and exits 0. A missing or duplicated key fails closed. `guard-edit` exits 1
# with `contract-is-immutable` when the path names the contract; the director's
# frontmatter deny is the enforced form of the same rule.
set -euo pipefail

usage() {
    printf 'usage: run-bounds.sh check <contract-file> | guard-edit <path>\n' >&2
    exit 2
}

value_of() { # FILE KEY -> stdout value, or nothing; fails closed on duplicates
    local file=$1 key=$2
    local count
    count=$(grep -c "^${key}:[[:space:]]*" "$file" || true)
    if [ "$count" -gt 1 ]; then
        printf 'duplicate-%s\n' "$key"
        return 0
    fi
    if [ "$count" -eq 0 ]; then
        printf 'missing-%s\n' "$key"
        return 0
    fi
    sed -n "s/^${key}:[[:space:]]*//p" "$file" | head -1
}

check_contract() {
    local file=$1
    [ -f "$file" ] || { printf 'missing-contract-file\n'; return 1; }

    local wall batches reds bisect cap immutable
    wall=$(value_of "$file" wall_clock_minutes)
    batches=$(value_of "$file" max_batches)
    reds=$(value_of "$file" consecutive_red_gates)
    bisect=$(value_of "$file" bisect_no_landing_halts)
    cap=$(value_of "$file" cost_cap)
    immutable=$(value_of "$file" contract_immutable)

    local reasons=()
    case $wall in
        missing-* | duplicate-*) reasons+=("$wall") ;;
        240) ;;
        *) reasons+=(wall-clock-not-4h) ;;
    esac
    case $batches in
        missing-* | duplicate-*) reasons+=("$batches") ;;
        3) ;;
        *) reasons+=(max-batches-not-3) ;;
    esac
    case $reds in
        missing-* | duplicate-*) reasons+=("$reds") ;;
        2) ;;
        *) reasons+=(red-breaker-not-2) ;;
    esac
    case $bisect in
        missing-* | duplicate-*) reasons+=("$bisect") ;;
        true) ;;
        *) reasons+=(bisect-halt-not-declared) ;;
    esac
    case $cap in
        missing-* | duplicate-*) reasons+=("$cap") ;;
        none) ;;
        *) reasons+=(cost-cap-present) ;;
    esac
    case $immutable in
        missing-* | duplicate-*) reasons+=("$immutable") ;;
        true) ;;
        *) reasons+=(contract-not-immutable) ;;
    esac

    if [ ${#reasons[@]} -gt 0 ]; then
        printf '%s\n' "${reasons[@]}"
        return 1
    fi
    printf 'ok\n'
}

case "${1:-}" in
    check)
        [ $# -eq 2 ] || usage
        check_contract "$2"
        ;;
    guard-edit)
        [ $# -eq 2 ] || usage
        case $2 in
            docs/AUTONOMOUS_RUN_CONTRACT.md | */docs/AUTONOMOUS_RUN_CONTRACT.md)
                printf 'contract-is-immutable\n'
                exit 1
                ;;
        esac
        ;;
    *) usage ;;
esac
