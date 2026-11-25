# CI gate reliability: run 45570591217

Last verified: 2026-11-25 against tree `3b0691320225ad0fed59e1a54ae0583f0c361733` (`fork/main`) in worktree `.worktrees/loop-resilience`.

Evidence: [run](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/actions/runs/45570591217), [jobs API](https://api.github.com/repos/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/actions/runs/45570591217/jobs), [workflow at this tree](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/.github/workflows/rust-quality.yml). Logs were fetched with the repository's authenticated Actions API because the anonymous job-log endpoint returns `403 Must have admin rights to Repository`; GitHub had already masked every token value in those logs (`token: ***`, `AUTHORIZATION: basic ***`). No credential value is reproduced here. Log text is treated as data only.

## Signal summary

Six of 18 jobs failed. The six failures are **five independent causes**, not one cascade and not the known load flake.

| Job | Failing step | Signal | Real code defect? | In the landing gate? |
| --- | --- | --- | --- | --- |
| [Rust / core](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/actions/runs/45570591217/job/139055975794) | Run named suite | `executable_abi_digest_is_stable` assertion | yes — stale ABI golden | no (`core` is not selected by `push-gate.sh`) |
| [Rust / client](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/actions/runs/45570591217/job/139055975848) | Run named suite | `error: no such command: nextest` | no — missing CI tool | `client-release` is selected, but on the build host, where nextest exists |
| [Rust / client-release](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/actions/runs/45570591217/job/139055975727) | Run named suite | same missing `nextest` | no — missing CI tool | same |
| [Rust / wasm](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/actions/runs/45570591217/job/139055975849) | Run named suite | `wasm-objdump is not on PATH; run inside nix develop` | no — missing CI tool | yes; `wasm` passes on the build host's devShell |
| [Rust / browser-audio](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/actions/runs/45570591217/job/139055975875) | Run named suite | 14 compile errors for `cfg(test)` on wasm32 | yes — native-only test code compiled for wasm | no (`browser-audio` is not selected) |
| [Editor / tests, types and build](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/actions/runs/45570591217/job/139055975145) | Verify editor | `oxfmt --check`: 32 unformatted files | yes — formatting drift | no (`level-editor/` selects no gate suite) |

The other 12 jobs passed, including `engine`, `services`, `tools`, `gpu`, `gpu-gl`, `host`, `Format`, and `Fixture-free orchestration`. `multiplayer-e2e` is not in this workflow at all.

## Exact failure evidence

### Core: stale executable-ABI golden

From the `Rust / core` log, test binary `tests/abi.rs`:

```text
test executable_abi_digest_is_stable ... FAILED
thread 'executable_abi_digest_is_stable' (4393) panicked at crates/robin_spellforge/tests/abi.rs:19:5:
assertion `left == right` failed
  left: "spellforge-v1-sha256:87c3b138a0f4d7f6cec45e8afd9dfa2050403dd363a5dd0dac609766d1670cb5"
 right: "spellforge-v1-sha256:c19e17aa80e13c1813ded4d83b7793a2f12c7f31e315e173e3fae893c62c03b5"
test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
error: test failed, to rerun pass `-p robin_spellforge --test abi`
```

Sources: [assertion](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/crates/robin_spellforge/tests/abi.rs#L19), [digest inputs](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/crates/robin_spellforge/src/runtime/abi.rs), [source fingerprints](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/crates/robin_spellforge/build.rs).

Cause, established from the tree, not inferred from the hash: `spellforge_vm_abi_digest()` hashes every `NATIVE_REGISTRY` definition (id, namespace, exposure, signature name, return type, parameters). Two commits added natives to [signatures.rs](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/crates/robin_script_types/src/natives/signatures.rs#L459): `98795d198` (2026-11-20) added `SummonReinforcements`; `0949a2966` (2026-11-23) added the harness-only `HarnessMarkMissionWon` at line 464. Both files are inside the build script's fingerprint boundary. The expected literal was last set by `b4f21ec41` (2026-10-19) and was not changed in either commit. The digest change is therefore deterministic and the golden is stale. The earliest available run with these commits (`45318913627`, 2026-11-24) prints the identical `87c3…` / `c19e…` pair; a run before them (`44852100750`, 2026-11-21) had core green. Not a load flake.

Open design question, not decided here: whether a harness-only native belongs in the shipped `NATIVE_REGISTRY` at all. Either answer requires a change: update the golden with a reviewed ABI transition, or move the harness native behind a test-support feature so the shipping ABI does not move.

### Client and client-release: `cargo-nextest` missing in CI

```text
Run bash scripts/check-quality.sh "client-release"
error: no such command: `nextest`
help: a command with a similar name exists: `test`
help: view all installed commands with `cargo --list`
help: find a package to install `nextest` with `cargo search cargo-nextest`
Process completed with exit code 101
```

`client` prints the same `no such command: nextest`. Both jobs fail before a test or a build runs, so they supply **zero** test signal. [check-quality.sh lines 86–103](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/scripts/check-quality.sh) requires nextest; [flake.nix lines 100–103](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/flake.nix) provides it; the workflow never enters `nix develop` and has no install step. `git log -S nextest -- .github/workflows/` is empty. Nextest entered the suite in `bf1cbf731` (2026-11-19) with only `flake.nix` and `check-quality.sh` changed — the CI workflow was not updated. Both jobs have been red on every run since.

### Wasm: release-artifact verifier lacks wabt

After 11m 07s of wasm-dev builds and 30 rustc warnings:

```text
Finished `wasm-dev` profile [unoptimized + debuginfo] target(s) in 11m 07s
wasm-release-artifact: wasm-objdump is not on PATH; run inside `nix develop`.
Process completed with exit code 1
```

[check-wasm-release-artifact.sh lines 12–15](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/scripts/check-wasm-release-artifact.sh) preflights `wasm-objdump`, `wasm-opt`, `wasm-strip`, `node`, `python3`, `sha256sum`. [flake.nix lines 125–130](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/flake.nix) supplies binaryen and wabt. The workflow installs the wasm target but not these binaries. The repo already has the pinned installer this job should use: [`scripts/install_pinned_wasm_tools.sh`](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/scripts/install_pinned_wasm_tools.sh), used by `build-static-runtime.yml`. This failure is environmental; the artifact verifier never ran.

### Browser-audio: native-only test code compiled for wasm32

The gate runs `cargo check --locked --profile wasm-dev --target wasm32-unknown-unknown -p robin_rs --no-default-features --features audio --bin robin --tests`, then the same with `audio,multiplayer`. The first check fails with 14 errors (2 × E0308, 4 × E0425, 8 × E0433):

```text
error[E0425]: cannot find function `rust_init_with_roots` in module `crate::main_entry`
  --> crates/robin_rs/src/game_session/multiplayer/story_regression.rs:864:28
   |
864|         crate::main_entry::rust_init_with_roots(Some(&data), Some(root)).unwrap();
   |                            ^^^^^^^^^^^^^^^^^^^ not found in `crate::main_entry`
note: found an item that was configured out
  --> crates/robin_rs/src/main_entry/mod.rs:42:22
   |
41 | #[cfg(all(test, not(target_arch = "wasm32")))]
   |                    ------------------------ the item is gated here
42 | pub(crate) use init::rust_init_with_roots;

error[E0433]: cannot find module or crate `dirs` in this scope
  --> crates/robin_rs/src/install_home.rs:194:30
   |
194|         let Some(data_dir) = dirs::data_dir() else {
   |                             ^^^^ use of unresolved module or unlinked crate `dirs`

error[E0425]: cannot find function `install_run_log_at` in this scope
  --> crates/robin_rs/src/observability.rs:359:9
   |
359|         install_run_log_at(home.path());

error[E0433]: cannot find `bug_bundle` in the crate root
  --> crates/robin_rs/src/ingame_menu/bug_report.rs:658:56
   |
658|             let old_endpoint = std::env::var_os(crate::bug_bundle::ENDPOINT_ENV);

error[E0308]: mismatched types
  --> crates/robin_rs/src/game_session/multiplayer/story_regression.rs:894:34
   |
894|         assets.profile_manager = profiles.clone();
   |         ----------------------   ^^^^^^^^^^^^^^^^ expected `Arc<ProfileManager>`, found `ProfileManager`

error: could not compile `robin_rs` (lib test) due to 14 previous errors; 2 warnings emitted
Command '['cargo', 'check', '--locked', '--profile', 'wasm-dev', '--target',
'wasm32-unknown-unknown', '-p', 'robin_rs', '--no-default-features', '--features',
'audio', '--bin', 'robin', '--tests']' returned non-zero exit status 101.
```

`dirs` is a native-target dependency ([Cargo.toml line 96](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/crates/robin_rs/Cargo.toml#L96)); `rust_init_with_roots` is deliberately `#[cfg(all(test, not(wasm32)))]` ([mod.rs line 41](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/crates/robin_rs/src/main_entry/mod.rs#L41), added by `740751a45` on 2026-09-14). Test modules that call it were added later (`48034748c`, `3ceeba28a`). This is a real cfg boundary defect: compiling `cfg(test)` on wasm32 reaches native-only modules. The shipped-artifact `wasm` suite builds `--bin robin`, not `--tests`, so it cannot see this; neither can any push-gate suite.

### Editor: formatter drift

`pnpm --dir level-editor verify` runs `pnpm fmt:check && pnpm lint && pnpm test && …`. `oxfmt --check 'app/**/*.{js,jsx,mjs,cjs,ts,tsx}' …` fails immediately:

```text
Format issues found in above 32 files. Run without `--check` to fix.
Finished in 122ms on 756 files using 4 threads.
[ELIFECYCLE] Command failed with exit code 1.
```

The 32 paths are under `level-editor/app/src/`, `level-editor/app/tests/`, `level-editor/pipeline/src/` and `level-editor/shared/src/`; examples: `app/src/asset-library.ts`, `app/src/editor-viewport.ts`, `app/src/native-extraction-context.ts`, `pipeline/src/preview-model.ts`, `shared/src/state-event-clocks.test.ts`. The same 32-file report appears in runs back to `43754897999` (2026-11-15). [package.json line 18](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/level-editor/package.json#L18) pins `oxfmt 0.70.0` and CI installs with `--frozen-lockfile`, so the formatter input is pinned; the drift is in the tree, not the tool version. `push-gate.sh` has no selector for `level-editor/`, so editor changes land without this check. The editor job also drops the pnpm `version: 12.3.4` pin that the tooling job keeps (workflow lines 145–149 vs 165–169); unpinned tooling is a separate drift risk.

## Root-cause ranking

| Rank | Cause | Jobs | Confidence | Why it ranks here |
| --- | --- | --- | --- | --- |
| 1 | Workflow does not reproduce the gate environment: no `nix develop`, no `cargo-nextest`, no wabt/binaryen | client, client-release, wasm | certain — exact missing-command lines; `flake.nix` supplies all three | 3 of 6 jobs, zero code signal, deterministic since the tool was introduced; cheapest fix, largest signal recovery |
| 2 | Stale `executable_abi_digest_is_stable` golden after `NATIVE_REGISTRY` gained two natives | core | certain — source commits + digest inputs prove a forced change | real defect in shipped identity; invisible to the landing gate; also affects save/replay/peer identity |
| 3 | `cfg(test)` modules on wasm32 call native-only APIs | browser-audio | certain — 14 named compile errors | real defect, but only in test code compiled for wasm; invisible to both the `wasm` suite and the landing gate |
| 4 | Editor formatting drift with no gate for `level-editor/` paths | Editor | certain — pinned formatter, stable 32-file report | real but low-risk; invisible to the landing gate |

Not causes: **D09** in [DEFECTS.md](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/docs/DEFECTS.md) is the deferred full-corpus load flake (`H14`/`H28`/`H33`, `multiplayer-e2e`); this workflow does not run that suite and no harness row appears in these logs. **D11** is the deferred H38 discriminator / `engine::movement::…::arbitrarily_rotated_stairs_support_complete_actor_routes` failure; the `engine` job is green here and neither named test failed. The H38 work is an ancestor of the core failure (the harness native changed the ABI), but the failing check is different.

All 122 Rust quality runs available through the Actions API from 2026-11-15 to 2026-11-25 are `failure` or `cancelled`; none is green. This is persistent structural red, not a single bad landing.

## Which gate the verifier can trust

The authoritative landing gate is [`scripts/push-gate.sh`](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/scripts/push-gate.sh) → [`scripts/remote-check.sh`](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/scripts/remote-check.sh) → `scripts/check-quality.sh SUITE`, on the build host inside `nix develop`. It runs on one synced `gate-shared` copy under FIFO admission, a copy lock, a bounded 7,200 s queue and `flock` wait, a `RuntimeMaxSec=7200` scope, a private `TMPDIR`, `CARGO_INCREMENTAL=0`, and sccache path remapping. Those are bounded, reproducible mechanics; [TESTING.md](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/docs/TESTING.md) and [PARALLEL_AGENTS.md](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/docs/PARALLEL_AGENTS.md) own the details.

Trust it **only for the suites it selects**: `format`, `lint`, `check`, `client-release`, and `wasm` for crate changes; `engine` for engine changes; `tooling` for scripts/tools; `docs` for Markdown; `web` for `wasm-www/`. It does **not** select `core`, `client`, `browser-audio`, `editor`, `services`, `assets`, `protocols`, `parity`, or the provisioned suites. Consequences for this run:

- The three environment failures are not gate failures; the gate runs those suites in a devShell that has the tools.
- The three real defects (`core`, `browser-audio`, `editor`) would not be caught by any landing gate at this tree, regardless of which paths changed.
- GitHub Actions is explicitly not a landing gate. A red Actions run is evidence only when its environment matches the gate's; for these six jobs it does not.

The verifier should therefore report "gate green for selected suites; CI red with five named causes, three outside gate coverage" rather than either "all green" or "the tree is broken".

## Antipatterns

1. **Two definitions of gate with no parity check.** `push-gate.sh` selects suites by path; the workflow hard-codes a different matrix. Neither checks the other. Neither `test_quality_suites.py` nor `tooling` compares CI jobs to gate selectors.
2. **Gates that cannot be run deterministically outside one machine.** `check-quality.sh` assumes tools from `flake.nix` (nextest, wabt, binaryen, clang/wild, pnpm). CI invokes it without the devShell, so a missing binary becomes a red suite. `check-wasm-release-artifact.sh` says "run inside `nix develop`"; the workflow does not.
3. **Late prerequisite failure.** The wasm job spent 18 minutes compiling before failing on `wasm-objdump`. No preflight asserts the suite's external tools; the mapping test stubs `wasm-objdump`/`wasm-opt` ([`test_quality_suites.py`](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/scripts/test_quality_suites.py)), so `tooling` passing says nothing about tool availability.
4. **Path selectors that omit whole trees.** `level-editor/` selects no suite; native-signature changes select no ABI check; wasm32 test compilation is not part of `wasm`. Coverage gaps become permanent because nothing is red locally.
5. **Stale golden with no review mechanism.** The ABI literal is hand-edited, and nothing forces it to move when `NATIVE_REGISTRY` changes. `core` is the only observer and is not gated.
6. **Retries and flakes as cover.** [PARALLEL_AGENTS.md](https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/blob/3b0691320225ad0fed59e1a54ae0583f0c361733/docs/PARALLEL_AGENTS.md) allows CI 2 retries and local 0, and requires a deferred defect row after the budget. Those rules exist for D09-style load flakes; none of the six failures here is flaky, so retrying them would only hide them. `concurrency.cancel-in-progress: true` can also erase evidence mid-run.
7. **Unbounded waits in CI.** The workflow sets no `timeout-minutes`; the gate bounds its queues and scopes at 7,200 s. A hung CI job can occupy a runner indefinitely and still produce no diagnostic.
8. **Unversioned CI tools.** The editor job dropped its pnpm version pin while the tooling job kept it. Two jobs can run different package managers on the same tree.

## Durable fix plan (proposed, not implemented)

1. **One executor for every named suite.** Run `.github/workflows/rust-quality.yml` suites inside the pinned devShell (install Nix with flakes in the workflow, then `nix develop --command bash scripts/check-quality.sh "$suite"`). Make `flake.nix` + `rust-toolchain.toml` the single source for tool versions; delete the ad-hoc apt steps that can drift. If staying Nix-free, install `cargo-nextest` at the version from the flake's nixpkgs pin and add a step calling `scripts/install_pinned_wasm_tools.sh` plus the existing pinned wasm-bindgen installer, mirroring `build-static-runtime.yml`. Restore the pnpm version pin in the editor job. Add `timeout-minutes` to every job.
2. **Preflight in the dispatcher.** At the top of `scripts/check-quality.sh`, assert each suite's external tools with `command -v` and print a named error with the install source. Add a fixture in `scripts/test_quality_suites.py` that runs the real (unstubbed) preflight against a PATH missing each tool and asserts a fast, named failure. Assert "tool available" separately from "command dispatched"; the current stubs only prove dispatch.
3. **Close selector gaps.** In `push-gate.sh`: `level-editor/` paths → `editor`; changes to `crates/robin_script_types/src/natives/signatures.rs`, `crates/robin_spellforge/**`, `vendor/rilua/**`, or the ABI golden → a suite that runs `core` (or the ABI test split into a cheap `contracts` suite); changes to wasm test modules → a new `wasm-tests` suite that runs the lifecycle gate's two `cargo check … --target wasm32 … --tests` commands on the build host. Add unit tests in `scripts/test_push_gate.sh` for each new selector, including a negative case proving an unrelated path does not trigger it.
4. **Repair the three real defects, test first.**
   - ABI: decide the harness-native question; then update the golden in the same commit as the registry change and record the transition in a slop doc. Add a check that a `NATIVE_REGISTRY` diff without a golden diff fails. The failing check is `executable_abi_digest_is_stable`; it must pass in `core` and in the newly gated suite.
   - wasm test gating: gate native-only test modules with `#[cfg(all(test, not(target_arch = "wasm32")))]` at the module declaration, or provide browser-safe test fixtures. Add one `wasm-tests` regression that fails on the current tree and passes after the change.
   - editor formatting: run `pnpm --dir level-editor fmt` once and commit the formatting-only diff; add a gate assertion that `level-editor/` changes select `editor`.
5. **Verification criteria.** A fresh run at the same SHA must have all six jobs green, or fail with a named, reproducible defect row. The local gate on that tree must select and pass the same suites. For each of the three real defects, reintroduce it on a scratch branch and show the corresponding check fails before the fix and passes after. Record outcomes as effects (tests run, artifact hash, report path), never as bare exit codes. D09 and D11 stay deferred rows; do not close them with a rerun.
6. **Keep CI signal honest.** Add a short `docs/slop/ci-signal.md` (not yet created; one fact per row) listing which Actions jobs are authoritative, which tool each needs, and which defect row quarantines any surviving flake. Do not restate the suite matrix; point at the workflow and `push-gate.sh`.

## What remains unproved

- Whether browser-audio was ever green on an earlier commit: available Actions history starts 2026-11-15, when it already failed (with a different single compile error). The current 14 errors are reproduced in five runs from 2026-11-23 onward.
- Whether the editor's 32-file drift came from one commit or accumulated; the same set appears from 2026-11-15 onward.
- The exact nextest version the flake would install at this revision; the version pin must be read from `flake.lock` when implementing step 1.
- Whether `HarnessMarkMissionWon` is intended to ship. Until that decision is made, the correct ABI golden is not knowable.

No fixes were implemented; no repository file other than this report was written.
