#!/usr/bin/env bash
# Fixture tests for scripts/push-gate.sh's tree checks: a synthetic repository in
# a temp dir, no network. The suites themselves need the build host, so every
# case here stops before `scripts/remote-check.sh` runs.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-push-gate.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

export HOME=$work/home
export XDG_STATE_HOME=$work/state
export GIT_CONFIG_GLOBAL=$work/gitconfig
export GIT_CONFIG_NOSYSTEM=1
mkdir -p "$HOME"
git config --file "$GIT_CONFIG_GLOBAL" user.name Fixture
git config --file "$GIT_CONFIG_GLOBAL" user.email fixture@example.invalid
git config --file "$GIT_CONFIG_GLOBAL" init.defaultBranch main
git config --file "$GIT_CONFIG_GLOBAL" commit.gpgsign false

pass_count=0
failures=0
pass() { printf 'ok   %s\n' "$1"; pass_count=$((pass_count + 1)); }
fail() {
    printf 'FAIL %s\n' "$1"
    failures=$((failures + 1))
}
expect_status() {
    if [ "$status" -eq "$1" ]; then pass "$2"; else fail "$2 (exit $status, want $1)"; fi
}
expect_contains() {
    case $output in
        *"$2"*) pass "$1" ;;
        *)
            fail "$1 (missing '$2')"
            printf '%s\n' "$output" | sed 's/^/     /' >&2
            ;;
    esac
}

git init --quiet "$work/repo"
cd -- "$work/repo"
mkdir -p scripts
cp -- "$source_root/scripts/push-gate.sh" scripts/push-gate.sh
printf 'base\n' >file.txt
git add file.txt scripts/push-gate.sh
git commit --quiet -m base
head=$(git rev-parse HEAD)

run_gate() {
    status=0
    output=$(bash scripts/push-gate.sh "$head" "$head" 2>&1) || status=$?
}

# The suites compile the working tree; the push lands the commit. A tree they
# disagree on is a gate that proves what nobody else will build.
printf 'uncommitted\n' >>file.txt
run_gate
expect_status 1 'a worktree that differs from the pushed commit is refused'
expect_contains 'the refusal names the difference' 'worktree differs from the commit being pushed'

# An untracked source file is the same lie: compiled here, absent there.
git checkout -- file.txt
printf 'new module\n' >untracked_module.rs
run_gate
expect_status 1 'an untracked file is refused'
expect_contains 'the refusal names the untracked file' 'untracked_module.rs'

# Nothing to refuse: a tree with no gated path runs no suite and passes.
rm -- untracked_module.rs
run_gate
expect_status 0 'a clean tree with no gated path changes passes'
expect_contains 'the pass says no suite was selected' 'no gated path changed'

# A crates/ change selects the suites that compile it: `wasm` is the only one
# that compiles the browser client (the artifact a guest joins with was red for
# a whole change while every native suite stayed green), and `lint` reads the
# same tree under `-D warnings`. The build host is not reachable here, so the
# stub records the selection instead.
mkdir -p crates/example
printf '[package]\nname = "example"\nversion = "0.0.0"\n' >crates/example/Cargo.toml
cat >scripts/remote-check.sh <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >"$GATE_SUITES"
printf '%s\n' "${ROBIN_DOCS_CHECK_PATHS:-}" >"${GATE_DOCS_PATHS:-$GATE_SUITES.paths}"
STUB
chmod +x scripts/remote-check.sh
git add crates/example/Cargo.toml scripts/remote-check.sh
git commit --quiet -m crates
crates_head=$(git rev-parse HEAD)
status=0
output=$(GATE_SUITES="$work/suites" bash scripts/push-gate.sh "$head" "$crates_head" 2>&1) || status=$?
expect_status 0 'a crates change runs the suites its diff selects'
selected=$(cat "$work/suites" 2>/dev/null || true)
if [ "$selected" = '--suites format lint client-release wasm tooling' ]; then
    pass 'the crates change selects lint and wasm, the suites that read every target, and not wasm-release'
else
    fail "the crates change selects lint and wasm without wasm-release (selected '$selected')"
fi

# A change to the browser build plan, the devShell that supplies wasm-opt and
# wasm-objdump, or the release-artifact check itself must select `wasm-release`
# even with no crate change. Otherwise a release-only break ships ungated.
mkdir -p wasm-www/scripts
printf 'export const RUNTIME_FEATURES = [];\n' >wasm-www/scripts/build-runtime.mjs
git add wasm-www/scripts/build-runtime.mjs
git commit --quiet -m wasm-plan
plan_head=$(git rev-parse HEAD)
status=0
output=$(GATE_SUITES="$work/suites" bash scripts/push-gate.sh "$crates_head" "$plan_head" 2>&1) || status=$?
expect_status 0 'a wasm build-plan change gates the wasm-release suite'
selected=$(cat "$work/suites" 2>/dev/null || true)
if [ "$selected" = '--suites web' ]; then
    pass 'the build-plan change selects web; the release artifact is never gated'
else
    fail "the build-plan change selects wasm-release and web (selected '$selected')"
fi

printf '{}' >flake.nix
printf '#!/usr/bin/env bash\n' >scripts/check-wasm-release-artifact.sh
git add flake.nix scripts/check-wasm-release-artifact.sh
git commit --quiet -m wasm-tools
tools_head=$(git rev-parse HEAD)
status=0
output=$(GATE_SUITES="$work/suites" bash scripts/push-gate.sh "$plan_head" "$tools_head" 2>&1) || status=$?
expect_status 0 'a devShell and release-check change gates wasm, wasm-release and tooling'
selected=$(cat "$work/suites" 2>/dev/null || true)
if [ "$selected" = '--suites wasm tooling' ]; then
    pass 'the tool change selects wasm, wasm-release and tooling together'
else
    fail "the tool change selects wasm, wasm-release and tooling (selected '$selected')"
fi

# A docs-only landing must select the docs link check rather than nothing.
mkdir -p docs
printf '# A doc\n' >docs/only.md
git add docs/only.md
git commit --quiet -m docs-only
docs_head=$(git rev-parse HEAD)
status=0
output=$(GATE_SUITES="$work/suites" bash scripts/push-gate.sh "$tools_head" "$docs_head" 2>&1) || status=$?
expect_status 0 'a docs-only change runs a suite'
selected=$(cat "$work/suites" 2>/dev/null || true)
if [ "$selected" = '--suites docs' ]; then
    pass 'the docs-only change selects the deterministic docs gate'
else
    fail "the docs-only change selects docs (selected '$selected')"
fi
checked=$(cat "$work/suites.paths" 2>/dev/null || true)
if [ "$checked" = 'docs/only.md' ]; then
    pass 'the docs gate receives only the Markdown this change touches'
else
    fail "the docs gate receives the changed Markdown (got '$checked')"
fi

# Editor sources previously selected no suite, so formatting drift landed.
mkdir -p level-editor/app/src
printf 'export const x = 1;\n' >level-editor/app/src/asset-library.ts
git add level-editor/app/src/asset-library.ts
git commit --quiet -m editor-path
editor_head=$(git rev-parse HEAD)
status=0
output=$(GATE_SUITES="$work/suites" bash scripts/push-gate.sh "$docs_head" "$editor_head" 2>&1) || status=$?
expect_status 0 'an editor source change gates the editor suite'
selected=$(cat "$work/suites" 2>/dev/null || true)
if [ "$selected" = '--suites editor' ]; then
    pass 'the editor change selects editor'
else
    fail "the editor change selects editor (selected '$selected')"
fi

# Native signatures feed the Spellforge ABI golden; changing them without the
# ABI test leaves a stale golden invisible to every landing gate.
mkdir -p crates/robin_script_types/src/natives
printf '// fixture\n' >crates/robin_script_types/src/natives/signatures.rs
git add crates/robin_script_types/src/natives/signatures.rs
git commit --quiet -m abi-input
abi_head=$(git rev-parse HEAD)
status=0
output=$(GATE_SUITES="$work/suites" bash scripts/push-gate.sh "$editor_head" "$abi_head" 2>&1) || status=$?
expect_status 0 'an ABI input change gates the contracts suite'
selected=$(cat "$work/suites" 2>/dev/null || true)
if [ "$selected" = '--suites format lint client-release wasm contracts' ]; then
    pass 'the ABI change selects contracts'
else
    fail "the ABI change selects contracts (selected '$selected')"
fi

# `cfg(test)` modules compiled for wasm32 can break while the shipped wasm
# artifact builds; the lifecycle gate's --tests checks are a separate suite.
mkdir -p crates/robin_rs/src/game_session/multiplayer
printf '// fixture test module\n' >crates/robin_rs/src/game_session/multiplayer/story_regression.rs
git add crates/robin_rs/src/game_session/multiplayer/story_regression.rs
git commit --quiet -m wasm-test-module
wasmtest_head=$(git rev-parse HEAD)
status=0
output=$(GATE_SUITES="$work/suites" bash scripts/push-gate.sh "$abi_head" "$wasmtest_head" 2>&1) || status=$?
expect_status 0 'a wasm test-module change gates wasm-tests'
selected=$(cat "$work/suites" 2>/dev/null || true)
if [ "$selected" = '--suites format lint client-release wasm wasm-tests' ]; then
    pass 'the wasm test module selects wasm-tests'
else
    fail "the wasm test module selects wasm-tests (selected '$selected')"
fi

# A change with no gated path must still select nothing.
mkdir -p assets
printf 'fixture\n' >assets/readme.txt
git add assets/readme.txt
git commit --quiet -m unrelated
unrelated_head=$(git rev-parse HEAD)
status=0
output=$(GATE_SUITES="$work/suites" bash scripts/push-gate.sh "$wasmtest_head" "$unrelated_head" 2>&1) || status=$?
expect_status 0 'an unrelated path change runs no suite'
expect_contains 'the unrelated change says nothing was selected' 'no gated path changed'

printf '\n%s checks passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
