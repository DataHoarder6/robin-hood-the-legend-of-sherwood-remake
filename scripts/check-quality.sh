#!/usr/bin/env bash
# Named local/CI suites. Keep package selections explicit: bare Cargo commands
# intentionally select only the workspace's two small default members.
set -euo pipefail
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$repository"

# shellcheck source=runtime-hosts.sh
source "$repository/scripts/runtime-hosts.sh"

# The runtime is not a build host: compiling the workspace there competes with
# the agents that live on it. Every suite goes through the build host via
# scripts/remote-check.sh SUITE. The no-argument call below is the suite listing
# remote-check.sh reads locally to validate names, so it is not guarded.
assert_build_host() {
    local host=${ROBIN_TEST_HOSTNAME:-$(hostname)}
    if is_non_build_runtime "$host"; then
        printf 'check-quality: %s is not a build host; run `scripts/remote-check.sh SUITE` instead (build host: minipc by default)\n' \
            "$host" >&2
        exit 1
    fi
}

if (( $# != 1 )); then
    printf 'usage: bash scripts/check-quality.sh SUITE\n' >&2
    printf 'suites: format lint check core contracts scripting-llvm engine assets protocols services parity parity-cross client client-check client-release tools wasm wasm-tests browser-audio native-lifecycle multiplayer-e2e tooling unreferenced-items web editor editor-browser gpu gpu-gl host fixtures-demo fixtures-fullgame fixtures-legacy-linux docs\n' >&2
    exit 2
fi

assert_build_host

# Name a missing tool before any long build. CI installs the same pinned tools
# from flake.nix before calling a suite; a missing binary is a prerequisite
# failure, never a red test result.
bash "$repository/scripts/quality-preflight.sh" "$1"

case "$1" in
    format)
        cargo fmt --all -- --check
        ;;
    lint)
        # Two feature sets, because one run would lint almost nothing: the
        # default members are two small crates, and `robin_rs`'s default
        # features compile a fraction of the client. The shipping client is the
        # `release` set. No `-A` here: a lint that is wrong for this code is
        # allowed by name where it fires, with a reason.
        cargo clippy --locked --workspace --all-targets -- -D warnings
        cargo clippy --locked -p robin_rs --all-targets \
            --no-default-features --features release -- -D warnings
        ;;
    check)
        # Compile the non-test cfg of the whole workspace with warnings denied.
        # Cargo test and `clippy --all-targets` compile with cfg(test), which
        # hides unused imports and dead code that only the shipped build sees.
        # `--lib --bins` selects those targets and no test harnesses.
        # `cargo check` has no trailing-`--` rustc-argument form, so add -D
        # warnings through a cfg(all()) rustflag: Cargo joins every matching
        # target rustflag source (including .cargo/config.toml's target-cpu
        # baseline) rather than replacing them as a bare RUSTFLAGS would.
        cargo check --locked --workspace --lib --bins \
            --config 'target."cfg(all())".rustflags=["-D","warnings"]'
        ;;
    core)
        cargo test --locked -p robin_util -p robin_display_text -p robin_state_hash_derive -p robin_script_types -p robin_spellforge -p robin_lua -p robin_test_support
        ;;
    contracts)
        # The executable Spellforge identity: every NATIVE_REGISTRY definition
        # is part of the ABI, so a signature change without a reviewed golden
        # update must fail the landing.
        cargo test --locked -p robin_spellforge --test abi
        ;;
    wasm-tests)
        # The lifecycle gate's two `cfg(test)`-on-wasm32 checks, run separately
        # from the shipped-artifact `wasm` suite.
        cargo check --locked --profile wasm-dev --target wasm32-unknown-unknown -p robin_rs --no-default-features --features audio --bin robin --tests
        cargo check --locked --profile wasm-dev --target wasm32-unknown-unknown -p robin_rs --no-default-features --features audio,multiplayer --bin robin --tests
        ;;
    scripting-llvm)
        # LLVM verifies unwind/destructor semantics, not just panic detection.
        # Cranelift cannot continue through the poison-recovery catch_unwind.
        # Select only fixture-free cases, not every ignored corpus test.
        cargo test --locked -p robin_spellforge --lib poisoned_runtime_rebuilds_from_committed_history \
            --config 'profile.test.package.robin_spellforge.codegen-backend="llvm"' -- --ignored
        cargo test --locked -p robin_lua --test natives_smoke panic_unwind_detaches_native_session \
            --config 'profile.test.package.robin_lua.codegen-backend="llvm"'
        ;;
    engine) cargo test --locked -p robin_engine_types -p robin_legacy_save -p robin_level_data -p robin_engine ;;
    assets)
        cargo test --locked -p robin_asset_codecs -p robin_assets -p robin_data_io
        cargo test --locked -p robin_assets --no-default-features
        python3 scripts/check_asset_boundary.py
        ;;
    protocols)
        cargo test --locked -p robin_run_types -p robin_run_protocol -p robin_replay_format -p robin_identity_signer
        ;;
    services)
        # Prove the production API lacks raw fixture access before opting in.
        cargo check --locked -p robin_highscores --lib --bins --no-default-features
        cargo test --locked -p robin_highscores --doc --no-default-features
        cargo test --locked -p robin_highscores -p robin_replay_verifier --features robin_highscores/test-support
        ;;
    parity) cargo test --locked -p robin_parity ;;
    client)
        # nextest runs each test in its own process on one scheduler, which is
        # where the wall time goes on a workspace with this many test binaries.
        # It runs no doctests, so they stay on `cargo test`.
        cargo nextest run --locked -p robin_rs
        cargo test --locked -p robin_rs --doc
        cargo build --locked -p robin_rs --bin robin
        ;;
    client-check)
        # Compile-only local iteration on the client: no nextest, no release
        # build. The push gate stays authoritative; this is not substituted
        # for `client-release` when landing.
        cargo check --locked -p robin_rs --lib --bins
        ;;
    client-release)
        cargo nextest run --locked -p robin_rs --lib --no-default-features --features release
        cargo build --locked -p robin_rs --bin robin --no-default-features --features release
        cargo check --locked -p robin_rs --example audio_decode_bench --no-default-features --features release
        ;;
    tools)
        cargo test --locked -p robin_modding_tools
        cargo test --locked -p robin_rs --features tools --bin convert_datadir --bin dump_level
        cargo check --locked -p robin_rs --features tools --bins --examples
        ;;
    wasm)
        cargo check --locked --profile wasm-dev --target wasm32-unknown-unknown -p robin_identity_signer --bin leaderboard_identity_bridge --features identity-signer-bridge
        cargo check --locked --profile wasm-dev --target wasm32-unknown-unknown -p robin_replay_admission_wasm
        cargo check --locked --profile wasm-dev --target wasm32-unknown-unknown -p robin_rs --bin robin --no-default-features
        # The artifact a browser guest joins with: `wasm-www/scripts/build-runtime.mjs`
        # ships `audio,multiplayer` (a runtime without multiplayer exports ticket
        # schema 0, so the stable shell refuses every invitation) and its threaded
        # plan adds `wasm-threads`, whose std has to be rebuilt with atomics.
        # Build, not check: only a link exercises the threaded link arguments.
        cargo build --locked --profile wasm-dev --target wasm32-unknown-unknown -p robin_rs --bin robin --no-default-features --features audio,multiplayer
        cargo build --locked -Zbuild-std=std,panic_abort --profile wasm-dev --target wasm32-unknown-unknown --config scripts/wasm-threads.cargo-config.toml -p robin_rs --bin robin --no-default-features --features audio,multiplayer,wasm-threads
        # wasm-dev links prove compilation; this builds the shipped wasm-release
        # artifact with the repo's plan and checks the optimized bytes' shared
        # memory, compatibility export and schema. A release-only break cannot
        # hide behind a green compile gate.
        bash scripts/check-wasm-release-artifact.sh
        ;;
    browser-audio|native-lifecycle)
        # This delegated gate is the entire suite. Do not retain a suspended
        # shell reader while a long compilation/runtime command is executing.
        exec python3 scripts/validation/lifecycle_gate.py "$1"
        ;;
    multiplayer-e2e)
        # Section 5 of docs/multiplayer/HARNESS.md: provisioned host only, real
        # processes and real signals. Exit 3 (a missing prerequisite) is a suite
        # failure here too — a broken machine must never read as a green run.
        : "${ROBIN_E2E_BINARY:?Set ROBIN_E2E_BINARY to a robin built with --features release}"
        : "${ROBINHOOD_DATA_DIR:?Set ROBINHOOD_DATA_DIR to the game root containing Data/}"
        evidence=${ROBIN_E2E_EVIDENCE:-$(mktemp -d "${TMPDIR:-/tmp}/robin-multiplayer-e2e.XXXXXX")}
        printf 'multiplayer-e2e evidence: %s\n' "$evidence" >&2
        # Two scenarios at a time: a fault fires at frame N, so the width was
        # measured on the build host before it was raised from one. 14 process
        # rows, same binary: 1357 s at --jobs 1, 744 s at --jobs 2 (1.82x), all
        # 14 verdicts "observed" in both runs. Scenarios own their display, data
        # dir and ports. Two leaves room for their game clients on 12 cores;
        # higher widths are unmeasured and may turn core starvation into a
        # timing artefact reported as a defect.
        exec python3 scripts/validation/multiplayer_e2e.py \
            --binary "$ROBIN_E2E_BINARY" --data "$ROBINHOOD_DATA_DIR" \
            --snapshot "$(git rev-parse HEAD)" --evidence "$evidence" \
            --jobs "${ROBIN_E2E_JOBS:-2}"
        ;;
    parity-cross)
        # Linux x Windows replay-hash parity: every recording in the tracked
        # corpus plus any PARITY_REPLAY, one data root, the native binary and the
        # Windows release binary under wine. Provisioned only (licensed data and
        # wine); exit 3 (a missing prerequisite) is a suite failure here, never a
        # green run. Wine is not Windows; see docs/TESTING.md for the limit this
        # proves and the one it does not.
        : "${PARITY_LINUX_BIN:?Set PARITY_LINUX_BIN to a native robin built from the target commit}"
        : "${PARITY_WINDOWS_EXE:?Set PARITY_WINDOWS_EXE to the robin.exe from that commit}"
        : "${PARITY_DATA_DIR:?Set PARITY_DATA_DIR to the game root containing Data/}"
        corpus=scripts/validation/parity_corpus
        mapfile -t replays < <(find "$corpus" -type f \
            \( -name '*.rhrec.jsonl' -o -name '*.rhrec' \) | sort)
        [[ -z "${PARITY_REPLAY:-}" ]] || replays+=("$PARITY_REPLAY")
        if (( ${#replays[@]} == 0 )); then
            printf 'parity-cross: no recorded corpus in %s\n' "$corpus" >&2
            exit 3
        fi
        # Each binary runs from its install directory, which must carry
        # assets/core-datadir. Defaults to the binary's own directory; set these
        # when the binary lives in target/release of a checkout.
        args=(--linux "$PARITY_LINUX_BIN" --windows "$PARITY_WINDOWS_EXE"
              --data-dir "$PARITY_DATA_DIR")
        [[ -n "${PARITY_LINUX_DIR:-}" ]] && args+=(--linux-dir "$PARITY_LINUX_DIR")
        [[ -n "${PARITY_WINDOWS_DIR:-}" ]] && args+=(--windows-dir "$PARITY_WINDOWS_DIR")
        status=0
        for replay in "${replays[@]}"; do
            printf 'parity-cross: %s\n' "$replay"
            rc=0
            python3 scripts/validation/parity_check.py "${args[@]}" --replay "$replay" || rc=$?
            if (( rc != 0 && status == 0 )); then
                status=$rc
            fi
        done
        exit "$status"
        ;;
    gpu)
        # This test also invokes renderer::verify_offscreen_gpu_contract.
        WGPU_BACKEND=vulkan cargo test --locked -p robin_rs --lib gpu_upscale::tests::headless_downlevel_device_executes_every_multipass_profile -- --ignored --exact
        ;;
    gpu-gl)
        WGPU_BACKEND=gl xvfb-run -a cargo test --locked -p robin_rs --lib gpu_upscale::tests::headless_downlevel_device_executes_every_multipass_profile -- --ignored --exact
        ;;
    host)
        cargo test --locked -p robin_rs --lib --no-default-features --features hardware-info hardware::tests::native_memory_query_reports_real_total -- --ignored --exact
        ;;
    tooling)
        # Curated fixture-free tests: never discover arbitrary scripts that may
        # capture real game sessions or operate a deployed service.
        python3 scripts/test_quality_suites.py
        python3 scripts/test_workspace_hygiene.py
        pnpm --dir scripts/release verify
        # The browser runtime plan the stable site publishes. A plan without
        # multiplayer exports ticket schema 0 and refuses every invitation.
        node --test wasm-www/scripts/build-runtime.test.mjs
        # Child processes must be OS-supervised, and a browser join must fail
        # at the host when its artifact was never staged.
        node --test scripts/validation/supervised.test.mjs
        node --experimental-strip-types --test wasm-www/src/content_cache.test.ts
        node --test scripts/validation/browser-boot.test.mjs
        # Issue #76: the reload check's pure contract (including the hard
        # "no live mission after reload" assertion) runs without a browser.
        node --test scripts/validation/campaign-persistence.test.mjs
        node --experimental-strip-types --test wasm-www/src/join_ticket.test.ts
        # A published manifest must be same-origin to the page that serves it,
        # not to a hard-coded upstream hostname.
        node --experimental-strip-types --test wasm-www/src/demo_datadir.test.ts
        python3 -m unittest discover -s scripts/validation -p supervised_test.py

        test -f scripts/test_portable_linker.py
        python3 -m unittest discover -s scripts -p test_portable_linker.py
        python3 -m unittest discover -s scripts -p test_triage_bug_bundle.py
        python3 -m unittest discover -s scripts -p test_defect_ledger.py
        test -f scripts/validation/save_load_live_test.py
        python3 -m unittest discover -s scripts/validation -p save_load_live_test.py
        python3 -m unittest discover -s scripts/validation -p capture_live_test.py
        python3 -m unittest discover -s scripts/validation -p lifecycle_gate_test.py
        python3 -m unittest discover -s scripts/validation -p namespace_x11_test.py
        python3 -m unittest discover -s scripts/validation -p runtime_evidence_test.py
        python3 -m unittest discover -s scripts/validation -p parity_check_test.py
        python3 -m unittest discover -s scripts/validation -p scenario_corpus_test.py
        python3 -m unittest discover -s scripts/validation -p multiplayer_e2e_test.py
        python3 -m unittest discover -s scripts/validation -p display_allocation_test.py
        test -f tools/crash-receiver/test_receiver.py
        python3 -m unittest discover -s tools/crash-receiver -p test_receiver.py
        bash scripts/test_parity_orchestration.sh
        bash scripts/test_publisher_short.sh
        bash scripts/test_release.sh
        bash scripts/test_release_notes.sh
        bash scripts/test_verify_release_tag.sh
        bash scripts/test_doctor.sh
        bash scripts/test_reconcile_main.sh
        bash scripts/test_prune_worktrees.sh
        bash scripts/test_land.sh
        bash scripts/test_session_gate_check.sh
        bash scripts/test_push_gate.sh
        bash scripts/test_release_gate.sh
        bash scripts/test_build_host.sh
        bash scripts/test_remote_check.sh
        # The unattended loop's state machine and one-run driver: fixture-only,
        # no network, no gate. They catch wrong-target write-backs and repeated
        # assertions before those behaviours strand a run.
        bash scripts/test_director_state.sh
        bash scripts/test_director_select_issue.sh
        bash scripts/test_director_writeback.sh
        bash scripts/tests/director-loop.sh
        bash scripts/tests/quality-preflight.sh
        bash scripts/test_prepare_next_minor.sh
        ;;
    unreferenced-items)
        # Advisory review queue, never a gate: rust-analyzer cannot see every
        # macro/feature/callback reference, so this always exits 0 and only
        # reports a count (or why the scan was unavailable). The full JSON
        # report is kept only when UNREFERENCED_ITEMS_REPORT names a path.
        report=${UNREFERENCED_ITEMS_REPORT:-}
        if [[ -z "$report" ]]; then
            report=$(mktemp)
            trap 'rm -f -- "$report"' EXIT
        fi
        scan_status=0
        python3 scripts/find_unreferenced_rust_items.py --json --output "$report" crates \
            >/dev/null || scan_status=$?
        if count=$(python3 -c 'import json, sys; print(len(json.load(open(sys.argv[1]))))' \
            "$report" 2>/dev/null)
        then
            printf 'advisory: %s unreferenced Rust functions/methods (review queue, not a failure)\n' \
                "$count"
        else
            printf 'advisory: unreferenced-item scan unavailable (exit %s); is rust-analyzer installed?\n' \
                "$scan_status"
        fi
        ;;
    docs)
        # Relative links in the Markdown a landing touches. The gate supplies
        # the changed paths; without them this audits every tracked Markdown
        # file, including retained historical docs with known rot.
        doc_paths=()
        if [ -n "${ROBIN_DOCS_CHECK_PATHS:-}" ]; then
            read -r -a doc_paths <<< "$ROBIN_DOCS_CHECK_PATHS"
        fi
        python3 scripts/check_markdown_links.py "${doc_paths[@]}"
        ;;
    web) pnpm --dir wasm-www verify:web ;;
    editor) pnpm --dir level-editor verify ;;
    editor-browser)
        : "${CHROME:?Set CHROME to the Chrome/Chromium executable}"
        "$CHROME" --version
        pnpm --dir level-editor build:browser
        if curl --silent --max-time 1 http://127.0.0.1:5181/tests/lifecycle.html >/dev/null; then
            printf 'port 5181 already serves HTTP; stop that preview before running the browser gate\n' >&2
            exit 1
        fi
        preview_log=$(mktemp "${TMPDIR:-/tmp}/robin-editor-preview.XXXXXX")
        preview_pid=
        cleanup_preview() {
            if [[ -n "$preview_pid" ]]; then
                kill -- "-$preview_pid" 2>/dev/null || true
                for _attempt in {1..10}; do
                    kill -0 "$preview_pid" 2>/dev/null || break
                    sleep 0.1
                done
                kill -KILL -- "-$preview_pid" 2>/dev/null || true
                wait "$preview_pid" 2>/dev/null || true
            fi
            cat -- "$preview_log"
            rm -f -- "$preview_log"
        }
        trap cleanup_preview EXIT
        trap 'exit 130' INT
        trap 'exit 143' TERM
        setsid pnpm --dir level-editor serve:browser >"$preview_log" 2>&1 &
        preview_pid=$!
        preview_ready=0
        for _attempt in {1..30}; do
            if curl --fail --silent --max-time 1 http://127.0.0.1:5181/tests/lifecycle.html >/dev/null; then
                preview_ready=1
                break
            fi
            if ! kill -0 "$preview_pid" 2>/dev/null; then
                printf 'editor preview exited before becoming ready\n' >&2
                exit 1
            fi
            sleep 1
        done
        if [[ "$preview_ready" != 1 ]]; then
            printf 'editor preview did not become ready\n' >&2
            exit 1
        fi
        timeout --signal=TERM --kill-after=10s 120s pnpm --dir level-editor test:browser
        ;;
    fixtures-demo)
        : "${ROBINHOOD_DATA_DIR:?Set ROBINHOOD_DATA_DIR to the Leicester demo root containing Data/}"
        python3 scripts/run_fixture_test.py --locked -p robin_level_data --lib profiles::tests::load_demo_profile_cpf -- --ignored --exact
        python3 scripts/run_fixture_test.py --locked -p robin_level_data --lib profiles::tests::original_cpf_and_exported_document_share_profile_validation -- --ignored --exact
        python3 scripts/run_fixture_test.py --locked -p robin_level_data --lib profiles::tests::demo_profile_serde_round_trip -- --ignored --exact
        python3 scripts/run_fixture_test.py --locked -p robin_rs --lib font::tests::test_parse_real_tfn -- --ignored --exact
        python3 scripts/run_fixture_test.py --locked -p robin_assets --lib frame_holder::tests::test_initialize_sprite_bank_from_game_data -- --ignored --exact
        python3 scripts/run_fixture_test.py --locked -p robin_assets --lib frame_holder::tests::test_sprite_bank_packed_data_present -- --ignored --exact
        python3 scripts/run_fixture_test.py --locked -p robin_assets --lib frame_holder::tests::test_validate_all_sprite_bank_streams -- --ignored --exact
        python3 scripts/run_fixture_test.py --locked -p robin_assets --features engine-adapters --lib picture::tests::original_demo_dialogue_portraits -- --ignored --exact
        python3 scripts/run_fixture_test.py --locked -p robin_assets demo_script -- --ignored
        python3 scripts/run_fixture_test.py --locked -p robin_rs --features tools --bin convert_datadir tests::authentic_demo_start_sxt_is_a_sixteen_picture -- --ignored --exact
        python3 scripts/run_fixture_test.py --locked -p robin_rs --features tools --bin convert_datadir tests::authentic_demo_root_has_exact_typed_edition -- --ignored --exact
        ;;
    fixtures-fullgame)
        : "${ROBINHOOD_DATA_DIR:?Set ROBINHOOD_DATA_DIR to the full-game root containing Data/}"
        python3 scripts/run_fixture_test.py --locked -p robin_level_data --lib profiles::tests::load_fullgame_profile_cpf -- --ignored --exact
        python3 scripts/run_fixture_test.py --locked -p robin_level_data --lib profiles::tests::original_cpf_and_exported_document_share_profile_validation -- --ignored --exact
        python3 scripts/run_fixture_test.py --locked -p robin_assets fullgame_scripts -- --ignored
        python3 scripts/run_fixture_test.py --locked -p robin_rs --features tools --bin convert_datadir tests::authentic_fullgame_root_has_exact_typed_edition -- --ignored --exact
        ;;
    fixtures-legacy-linux)
        : "${ROBINHOOD_DATA_DIR:?Set ROBINHOOD_DATA_DIR to the Linux i386 v48 root containing the required profile.cpf and profile saves}"
        python3 scripts/run_fixture_test.py --locked -p robin_legacy_save --lib engine::tests::golden_lincoln_restart_engine_boundary -- --ignored --exact
        python3 scripts/run_fixture_test.py --locked -p robin_legacy_save --lib engine::tests::parses_current_linux_continue_engine_boundary -- --ignored --exact
        python3 scripts/run_fixture_test.py --locked -p robin_legacy_save --lib engine::tests::rejects_sound_source_count_before_allocation -- --ignored --exact
        python3 scripts/run_fixture_test.py --locked -p robin_engine --lib legacy_save::campaign::tests::golden_lincoln_restart_campaign_boundaries -- --ignored --exact
        python3 scripts/run_fixture_test.py --locked -p robin_engine --lib legacy_save::campaign::tests::parses_current_linux_continue_campaign_boundaries -- --ignored --exact
        ;;
    *) printf 'unknown quality suite: %s\n' "$1" >&2; exit 2 ;;
esac
