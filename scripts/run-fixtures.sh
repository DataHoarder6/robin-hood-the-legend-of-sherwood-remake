#!/usr/bin/env bash
# Run fixture commands concurrently, only the ones a change can affect.
#
#   scripts/run-fixtures.sh 'COMMAND' ...
#
# A command's fixture is the first argument that names a file or directory in
# the tree (`bash scripts/test_land.sh` -> scripts/test_land.sh;
# `python3 -m unittest discover -s DIR -p PAT` -> DIR/PAT; `pnpm --dir D` -> D).
# With ROBIN_CHANGED_PATHS (a landing's diff, set by push-gate) a command runs
# only when a changed path is its fixture, lies under it, or is named in it —
# fixtures copy the scripts they test by path, so naming is the dependency. With
# no ROBIN_CHANGED_PATHS every command runs. Each command's output is printed
# whole when it ends; the exit status is non-zero if any command failed.
set -uo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

fixture_of() { # COMMAND -> path
    local -a words
    read -r -a words <<<"$1"
    local i
    for ((i = 0; i < ${#words[@]}; i++)); do
        case ${words[i]} in
            -s) [ -n "${words[i + 3]:-}" ] && [ "${words[i + 2]}" = -p ] &&
                    { printf '%s/%s' "${words[i + 1]}" "${words[i + 3]}"; return; } ;;
            --dir) printf '%s' "${words[i + 1]}"; return ;;
        esac
    done
    for ((i = 1; i < ${#words[@]}; i++)); do
        [[ ${words[i]} == -* ]] && continue
        [ -e "${words[i]}" ] && { printf '%s' "${words[i]}"; return; }
        [[ ${words[i]} == */* ]] && { printf '%s' "${words[i]}"; return; }
    done
}

affected() { # FIXTURE -> 0 when a changed path can affect it
    local fixture=$1 path
    [ -z "${ROBIN_CHANGED_PATHS:-}" ] && return 0
    for path in $ROBIN_CHANGED_PATHS; do
        [ "$path" = "$fixture" ] && return 0
        [[ $path == "$fixture"/* ]] && return 0
        [ -f "$fixture" ] && grep -qF -- "$path" "$fixture" && return 0
        [ -f "$fixture" ] && grep -qwF -- "${path##*/}" "$fixture" && return 0
    done
    return 1
}

logs=$(mktemp -d "${TMPDIR:-/tmp}/robin-fixtures.XXXXXX")
trap 'rm -rf -- "$logs"' EXIT
selected=()
for command in "$@"; do
    fixture=$(fixture_of "$command")
    if affected "$fixture"; then
        selected+=("$command")
    fi
done
printf 'run-fixtures: %s of %s fixtures affected\n' "${#selected[@]}" "$#" >&2
(( ${#selected[@]} > 0 )) || exit 0

pids=()
for index in "${!selected[@]}"; do
    (
        start=$(date +%s)
        bash -c "${selected[index]}"
        status=$?
        printf '%s' "$(( $(date +%s) - start ))" >"$logs/$index.seconds"
        exit "$status"
    ) >"$logs/$index.log" 2>&1 &
    pids+=("$!")
done
failed=()
for index in "${!selected[@]}"; do
    if wait "${pids[index]}"; then
        result=passed
    else
        result=FAILED
        failed+=("${selected[index]}")
    fi
    printf '=== %s in %ss: %s\n' "$result" "$(cat "$logs/$index.seconds")" "${selected[index]}"
    cat -- "$logs/$index.log"
done
if (( ${#failed[@]} > 0 )); then
    printf 'run-fixtures: failed: %s\n' "${failed[@]}" >&2
    exit 1
fi
