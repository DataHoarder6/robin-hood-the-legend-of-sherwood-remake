#!/usr/bin/env bash
# Gate one push of the integration branch with the suites its diff selects.
#
#   scripts/push-gate.sh [BASE] [HEAD]
#
# BASE is the integration ref the push starts from (empty for a first push, which
# runs every suite rather than guessing); HEAD is the commit being landed. The
# defaults are `fork/main` and `HEAD`, i.e. the push an agent is about to make.
#
# Both entry points call this, so the suites a landing runs have one source:
#
#   .githooks/pre-push   a plain `git push` of `refs/heads/main`; the hook reads
#                        the shas git would update and gates them
#   scripts/land.sh      the loop's landing command: gate here, then push
#
# Why a second entry point exists at all: git connects to the remote and starts
# its receive-pack conversation *before* it runs the pre-push hook, so a gate that
# takes minutes holds an idle connection open for minutes. Measured 2026-11-19:
# every gated push of a tree whose suites took ~7 minutes died with SIGPIPE (exit
# 141) after the suites passed and the branch never moved, while the same tree
# pushed fine as an ungated review ref. `land.sh` runs the same gate and only
# then opens the connection.
set -euo pipefail

exec </dev/null

repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$repository"

integration=refs/heads/main
base=${1:-$(git rev-parse fork/main)}
head=${2:-HEAD}

usage() {
    awk 'NR > 1 && /^[^#]/{exit} NR > 1{sub(/^# ?/, ""); print}' "${BASH_SOURCE[0]}" >&2
    exit 2
}

case "${1:-}" in
    -h | --help) usage ;;
esac

head=$(git rev-parse --verify "$head^{commit}")
if [ -n "$base" ]; then
    base=$(git rev-parse --verify "$base^{commit}")
fi

# The suites compile the working tree, but the push lands a commit. If the two
# differ, the gate would prove a tree nobody else will ever build.
if ! git diff --quiet "$head" -- .; then
    printf 'push-gate: the worktree differs from the commit being pushed:\n' >&2
    git diff --name-only "$head" -- . | sed 's/^/push-gate:   /' >&2
    printf 'push-gate: commit them first.\n' >&2
    exit 1
fi
# Untracked files are not pushed either: a new module the gate compiles while
# nobody else has it is still a gate that proves the wrong tree.
untracked=$(git ls-files --others --exclude-standard)
if [ -n "$untracked" ]; then
    printf 'push-gate: untracked files are in the tree the suites compile but not in the push:\n' >&2
    printf '%s\n' "$untracked" | sed 's/^/push-gate:   /' >&2
    printf 'push-gate: commit, ignore or delete them first.\n' >&2
    exit 1
fi

if [ -n "$base" ]; then
    paths=$(git diff --name-only "$base" "$head")
else
    # A first push of the integration branch has no base to diff against: run
    # every suite rather than guess that it is clean.
    paths=crates/
fi

# The suites follow the diff, so a Python-only change never pays for a Rust
# build and a Rust change never skips the suite that compiles it.
crates_changed=0
engine_changed=0
tools_changed=0
wasm_changed=0
printf '%s\n' "$paths" | grep -qE '^crates/' && crates_changed=1
printf '%s\n' "$paths" | grep -qE '^(crates/robin_engine|Cargo\.)' && engine_changed=1
printf '%s\n' "$paths" | grep -qE '^(scripts|tools|\.githooks)/' && tools_changed=1
# The browser artifact is built by these files as well as by crates/. `wasm`
# now builds the actual wasm-release module, so its own inputs and the devShell
# that provides wasm-opt/wasm-objdump must select it or the gate can drop out
# silently while a release-only break ships.
printf '%s\n' "$paths" | grep -qE '^crates/' && wasm_changed=1
printf '%s\n' "$paths" | grep -qE '^(wasm-www/|flake\.nix$|rust-toolchain\.toml$|scripts/check-wasm-release-artifact\.sh$)' && wasm_changed=1

suites=()
if [ "$crates_changed" = 1 ] || printf '%s\n' "$paths" | grep -qE '^(Cargo\.|rust-toolchain)'; then
    # `lint` reads the same two feature sets as a crate change, so it follows
    # the same paths. `check` compiles the non-test cfg with -D warnings,
    # catching unused imports that cfg(test) hides. `wasm` is selected
    # separately below: it also depends on the build plan and devShell, not
    # only on crates/.
    suites+=(format lint check client-release)
fi
if [ "$wasm_changed" = 1 ]; then
    suites+=(wasm)
fi
if [ "$engine_changed" = 1 ]; then
    suites+=(engine)
fi
# The editor has its own formatter/lint/test suite; without this selector an
# editor-only change lands with zero checks (32 unformatted files survived).
if printf '%s\n' "$paths" | grep -qE '^level-editor/'; then
    suites+=(editor)
fi
# Native signatures are ABI inputs: the Spellforge golden must move in the
# same landing, or a stale identity ships and no other suite notices.
if printf '%s\n' "$paths" | grep -qE '^(crates/robin_script_types/src/natives/signatures\.rs$|crates/robin_spellforge/|vendor/rilua/)'; then
    suites+=(contracts)
fi
# `cfg(test)` code compiled for wasm32 is not the shipped artifact; the
# lifecycle gate's --tests checks are a separate suite so a native-only test
# module cannot break the browser build unnoticed.
if printf '%s\n' "$paths" | grep -qE '^crates/robin_rs/.*(/tests/|tests\.rs$|story_regression\.rs$)'; then
    suites+=(wasm-tests)
fi
# `tooling` is the allowlisted shell/Python gate for everything under `scripts/`
# and `tools/`; it is green on the build host (issue #40).
if [ "$tools_changed" = 1 ]; then
    suites+=(tooling)
fi
# A docs-only landing would otherwise select nothing and run zero checks.
# The suite checks only the Markdown this landing changes; retained historical
# docs are not a gate on an unrelated change.
if printf '%s\n' "$paths" | grep -qE '(^docs/|\.md$)'; then
    suites+=(docs)
    ROBIN_DOCS_CHECK_PATHS=$(printf '%s\n' "$paths" | grep -E '\.md$' | paste -sd' ' -)
    export ROBIN_DOCS_CHECK_PATHS
fi
# wasm-www JS/TS and its configs run the `web` suite. The release scripts'
# JS/TS is already gated by `tooling` (pnpm --dir scripts/release verify).
if printf '%s\n' "$paths" | grep -qE '^wasm-www/.*\.(js|jsx|mjs|cjs|ts|tsx|json|ya?ml)$'; then
    suites+=(web)
fi

# The scenario corpus keeps its own cheap gate beside `tooling`: this runs the
# real corpus against docs/multiplayer/HARNESS.md, where the suite's unit tests
# only exercise the checker on synthetic contracts.
if printf '%s\n' "$paths" | grep -qE '^(scripts/validation/(scenarios/|scenario_corpus\.py)|docs/multiplayer/HARNESS\.md)'; then
    printf 'push-gate: scenario corpus\n' >&2
    python3 scripts/validation/scenario_corpus.py || {
        printf 'push-gate: the corpus does not match docs/multiplayer/HARNESS.md.\n' >&2
        exit 1
    }
fi
if [ ${#suites[@]} -eq 0 ]; then
    printf 'push-gate: no gated path changed; nothing to run.\n' >&2
    exit 0
fi

# One remote-check call for the whole push: it syncs this checkout once and runs
# every suite under the copy's lock, so a later suite reuses what the earlier one
# built. remote-check pushes this checkout to the build host, and that push would
# enter the pre-push hook again; the copy is a build cache, not an integration.
printf 'push-gate: %s\n' "${suites[*]}" >&2
# Gate copies are keyed by the tree being proved, so two pushes of one tree
# share a build instead of paying for it twice.
export ROBIN_GATE_TREE=$(git rev-parse "${head}^{tree}")
SKIP_PUSH_GATE=1 bash scripts/remote-check.sh --suites "${suites[@]}" || {
    printf 'push-gate: %s failed; the integration branch stays where it is.\n' "${suites[*]}" >&2
    printf 'push-gate: fix it, or push with SKIP_PUSH_GATE=1 and say why.\n' >&2
    exit 1
}
