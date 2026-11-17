#!/usr/bin/env bash
# Read-only state checks shared by scripts/doctor.sh and scripts/reconcile-main.sh.
# Source this file; it starts nothing, writes nothing and needs no arguments
# beyond the repository root each function is given.

# preflight_root [DIRECTORY]: absolute top level of the work tree containing it.
preflight_root() {
    git -C "${1:-.}" rev-parse --show-toplevel 2>/dev/null
}

# preflight_integration_ref ROOT: the ref a checkout's main follows.
# `fork/main` is this project's integration branch; `origin/main` is the
# upstream project. A branch's own upstream is the last resort, so a plain
# clone with one remote still gets an answer. Prints nothing when there is none.
preflight_integration_ref() {
    local root=$1 candidate
    for candidate in fork/main origin/main; do
        if git -C "$root" rev-parse --verify --quiet "$candidate^{commit}" >/dev/null; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done
    candidate=$(git -C "$root" rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null) || candidate=
    [ -n "$candidate" ] && printf '%s\n' "$candidate"
    return 0
}

# preflight_dirty ROOT: succeeds when tracked or untracked work is uncommitted.
preflight_dirty() {
    [ -n "$(git -C "$1" status --porcelain --untracked-files=normal)" ]
}

# preflight_build_processes ROOT: "PID COMMAND" for every compiler or build
# driver whose working directory is inside ROOT. A build there is compiled
# against a tree another agent may switch, so it must be visible.
preflight_build_processes() {
    local root=$1 pid cwd comm
    for pid in /proc/[0-9]*; do
        pid=${pid#/proc/}
        [ "$pid" != "$$" ] || continue
        cwd=$(readlink -f "/proc/$pid/cwd" 2>/dev/null) || continue
        case $cwd in
            "$root" | "$root"/*) ;;
            *) continue ;;
        esac
        comm=$(cat "/proc/$pid/comm" 2>/dev/null) || continue
        case $comm in
            cargo | rustc | rustfmt | clippy-driver | nix | nix-build | cc | gcc | g++ | clang | clang++ | ld | ld.lld | mold | sccache)
                printf '%s %s\n' "$pid" "$comm"
                ;;
        esac
    done
}

# preflight_cherry ROOT REF BRANCH: "UNIQUE DUPLICATE" commit counts.
# `git cherry` compares patch ids, so a commit already upstream under another
# hash counts as duplicate: re-landing it would duplicate the change.
preflight_cherry() {
    local root=$1 ref=$2 branch=${3:-HEAD} unique=0 duplicate=0 line
    while IFS= read -r line; do
        case $line in
            '+'*) unique=$((unique + 1)) ;;
            '-'*) duplicate=$((duplicate + 1)) ;;
        esac
    done < <(git -C "$root" cherry "$ref" "$branch" 2>/dev/null)
    printf '%s %s\n' "$unique" "$duplicate"
}
