# A 60-second landing gate

Last verified: 2026-11-27 (fork/main `2aa2c6e8e`). Goal: every landing's gate,
from taking the landing slot to the confirmed push, finishes in 60 seconds or
less on a warm `gate-shared`. Coverage may be cut where it re-proves what the
change cannot have broken. Nothing moves to a later process: whatever the gate
does not run before the push, it does not run.

## Where the time goes today

A real one-function edit in `robin_engine`, gated warm, took 3m38. The lanes ran
concurrently on 12 cores (`journalctl --user | grep -E 'gate-lanes|=== lane|Finished|Summary'`):

| Lane | Work | Time |
| --- | --- | --- |
| sync and startup | git push, checkout, devShell | 13 s |
| lint | clippy, whole workspace, every target | 1m09 |
| engine | engine test build 56 s, 4937 tests 31 s, doc tests 19 s | ~1m50 |
| client | client test build 1m21, 2748 tests 70 s | 2m34 |
| wasm | threaded client check | 52 s |

Three things make it slow:
- **The same code compiles in four configurations.** One engine edit
  recompiles the 17 MB engine for clippy, for the normal test build, for the
  engine's test target and for wasm, and the 11 MB client three times. Cargo
  rebuilds every dependent of a changed crate (no early cutoff).
- **Every suite runs regardless of what changed.** The client's 2748 tests run
  for an engine edit that did not touch the client.
- **A few slow tests set the tail.** One client test takes 55 s; the rest of
  the 2748 take seconds. Per-test times: nextest's `PASS [ n s]` lines in the
  same journal.

## Target budget

| Step | Budget |
| --- | --- |
| slot, sync, devShell, lane checkout | ≤ 8 s |
| concurrent lanes | ≤ 45 s |
| push and remote read-back | ≤ 5 s |

## The gate after this plan

The gate proves the changed packages, not fixed suites. `scripts/push-gate.sh`
maps the diff to packages (`crates/<name>/…` → `<name>`); the lanes then run:

- **lint:** `cargo clippy --workspace --features robin_app/release -- -D warnings`,
  libraries and binaries only. This still type-checks every dependent of a
  changed crate, so an API break anywhere fails here.
- **test:** `cargo nextest run` for the changed packages, their own unit and
  integration tests only. No doc tests.
- **wasm:** `cargo check` for wasm32 of the changed packages that are in the
  browser graph, in the published threaded configuration.
- **format, docs, tooling**, as today: format runs alongside, docs checks the
  changed Markdown, and tooling runs only the fixtures the diff names.

A change to `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`,
`.cargo/config.toml` or `flake.*` gates the whole workspace. It also invalidates
the warm copy, so it cannot meet 60 s; these changes are rare and are the
stated exception.

## What is cut, and why it is safe enough

| Cut | Why |
| --- | --- |
| Clippy on test targets | Test code still compiles in the test lane; only its lints go. |
| Tests of unchanged dependents (the client's tests on an engine edit) | An API break still fails lint. A behaviour break the engine's own tests miss can land. This is the main coverage cut. |
| Doc tests | 31 doc-example blocks across the engine crates cost 19 s per engine landing. Examples worth keeping become unit tests. |
| The client's wasm check on an engine edit | The engine's wasm check still runs. The client's wasm-only code is checked when the client changes. |
| `wasm-release` | Already outside the gate; CI and the release workflow build it. |

## Steps

Each step lands on its own through the gate, and reports its gate time from the
journal.

1. **Changed-package gate.** In `scripts/push-gate.sh`, derive the package set
   from the diff and pass it to the suites (an environment variable, as
   `ROBIN_CHANGED_PATHS` is passed today). `scripts/check-quality.sh` gets
   package-scoped `lint`, `test` and `wasm` suites that read it; the
   per-crate suites (`core`, `engine`, `client-release`, …) leave the gate. They
   stay in `check-quality.sh` for interactive runs and CI. Update
   `scripts/test_push_gate.sh` and `scripts/test_quality_suites.py`.
2. **Slow tests.** Every test over 5 s in the measured gate gets fixed at its
   cause (sizes, budgets, sleeps), keeping what it proves:
   - client: `distributed_mod::cache::native::tests::staging_evicts_the_unpinned_lru_complete_entry` (55 s),
     `replay_archive::tests::incomplete_children_never_publish_and_retries_preserve_orphan_bytes` (26 s),
     `savegame::tests::capture::mirrored_saves_share_one_snapshot_and_one_replay_marker` (22 s),
     `distributed_mod::cache::native::tests::partial_staging_uses_bounded_lru_space_reclamation` (19 s),
     and the 9–12 s save and persistence tests after them;
   - engine: `engine_facade_contract` and `rng_inventory` (about 14 s each:
     both parse the whole engine source; share one parse or narrow it), and
     `compiled_navigation::exported_stairs::arbitrarily_rotated_stairs_support_complete_actor_routes` (12 s).
   Exit: no test over 5 s in a gate's nextest output.
3. **Doc tests off.** `doctest = false` in the `[lib]` of each workspace crate;
   doc examples that assert behaviour move into unit tests.
4. **Startup.** Cache the devShell as a nix profile keyed by its inputs
   (`flake.nix`, `flake.lock`, `nix/`, the two guard scripts), enter it without
   re-evaluating, and fold the sync and the run into one ssh session. Exit:
   slot to first lane under 8 s.
5. **Lane priority.** The test lane runs at normal priority, the lint and wasm
   lanes under `nice`, so the critical lane gets the cores first.
6. **Measure and enforce.** `scripts/gate-lanes.sh` prints each lane's time, and
   `push-gate` prints the total. Three probe gates, run like the measurement
   above (a throwaway commit through `remote-check.sh --suites`, no push): one
   function in `robin_engine`, one in `robin_rs`, and one script change. Each
   must be at or under 60 s. If the engine probe is over, the next cut is the
   engine lane's test build itself (step 7).
7. **If still over: smaller test builds.** The engine's test target compiles
   its 4.7 MB of in-crate tests on every engine landing. Move the largest test
   modules that use only public items into integration-test crates, which
   compile in parallel with the library. A second build machine for the lanes
   would also do it; that needs hardware.

## Not in this plan

- **Splitting `robin_engine` into more crates.** It shortens local rebuilds,
  not the gate: the gate's cost is configurations × dependents, which the
  changed-package scope above removes directly.
