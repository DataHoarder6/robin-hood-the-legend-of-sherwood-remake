# Why a warm gate took 30 minutes

Investigated 2026-11-27 from the runtime journal and the source, without
compiling. The numbers below are measurements from that day; re-measure with the
commands in [`../TESTING.md`](../TESTING.md) ("Measuring a build before changing
it") before trusting them for a new decision.

## The measurement

`journalctl --user` on the runtime keeps every gate's Cargo output. Pairing each
`Finished` line with the `Compiling`/`Checking` lines before it gives the units
each step rebuilt. Four `crates/` landings on 2026-11-27 (12:17, 12:55, 13:42,
14:26 on `gate-shared`) were identical to within seconds:

| Step | Time | Units |
| --- | --- | --- |
| `lint`, workspace clippy | 3m47 | 300 (dependencies) |
| `lint`, `release` clippy | 2m07 | 17 (workspace crates) |
| `check`, `-D warnings` | 3m18 | 460 |
| `client-release` nextest | 1m13 | 2 |
| `client-release` `robin` build | 4m15 | 361 |
| `client-release` example check | 2m13 | 453 |
| `wasm`, three wasm-dev compiles | 2m31 | `robin_rs` + `robin_replay_format` each |
| `wasm`, wasm-release | 6m51 | 2 |

About 26 of the gate's 29 minutes were compiling.

## Causes and what changed

Ordered by measured cost.

1. **One suite's rustflags rebuilt every other suite's dependencies.** `check`
   added `-D warnings` through `target."cfg(all())".rustflags`. Without
   `--target` that also reaches proc-macros and build scripts, and Cargo keeps
   rustflags out of artifact file names (`compilation_files.rs`), so the suite
   overwrote the shared host artifacts and the check-mode `rmeta`. Every later
   step, and the next gate's first, rebuilt 300–460 units, `robin_engine`
   included. sccache's 99 % Rust hit rate on the build host that day was this
   loop restoring what it had just overwritten; its 542 non-cacheable calls were
   the check-mode work recompiling for real. Fix: `check` is gone (clippy
   `--all-targets -D warnings` checks every library with and without
   `cfg(test)`), and no suite may pass its own rustflags — the rule is in
   `scripts/check-quality.sh` and `scripts/test_quality_suites.py` asserts it.
2. **The audio example check repeated clippy.** The `release` clippy run over
   `--all-targets` already checks every example. Removed.
3. **wasm-release ran on every landing.** Fat LTO with one codegen unit
   re-optimizes the whole program: 6m51 warm. It is now the `wasm-release` suite,
   selected by `scripts/push-gate.sh` only when the release plan changes, and
   run by `.github/workflows/rust-quality.yml` on every push to `main`. The
   threaded wasm-dev build stays in `wasm`: it is now the only per-landing link
   of the threaded configuration.
4. **The commit hash rebuilt `robin_rs` in every configuration.**
   `robin_replay_format` (above `robin_engine`) baked `ROBIN_GIT_HASH` in through
   a build script watching `.git/HEAD`, and the gate moves `refs/heads/main` on
   every sync, so every gate rebuilt it and then the 11 MB `robin_rs` — "2 units:
   robin_replay_format robin_rs" on every wasm-dev step, even for a re-sync of
   the same branch. Fix: `robin_build_info` declares the identity as an extern
   symbol the libraries read; final artifacts define it with
   `robin_build_identity::define!()`, library test harnesses with
   `robin_build_info::define_test_identity!()`. The binaries, examples and the
   tests that run the binary moved from `robin_rs` to `robin_app`, so a commit
   rebuilds and relinks only those roots.
5. **Incremental was off in the gate.** It was off only because sccache refuses
   it (ADR 0007), and sccache adds nothing on the one stable copy. The gate copy
   now runs with an empty `RUSTC_WRAPPER` and the profiles' incremental default
   ([ADR 0013](../adr/0013-gate-incremental-without-sccache.md)).
6. **The two largest crates type-check on one core.** The `release` clippy step
   that rebuilt only the 17 workspace crates still took 2m07. `-Z threads=8` in
   `.cargo/config.toml` turns on the nightly parallel front end for every
   target. It is unmeasured here: compare one gate before keeping it.
7. **`robin_engine` (565 files, 18.4 MB, 3.5 MB of it in-crate tests) and
   `robin_rs` (10.9 MB) are single compilation units.** The one layer that
   never reaches back into the engine moved out: `robin_engine_geometry`
   (positions, sectors, gates, sight obstacles, fast-find grid, patches,
   material zones, element kinds, orders, the parity diagnostics gate; about
   0.9 MB), re-exported at the old paths. Everything else sits on the cycle
   described below.
8. **`test` and `dev` differed only in `debug`.** That gave every dependency and
   workspace library two artifacts, so nextest and the `robin` build in one gate
   compiled `robin_engine` twice. `profile.test` now inherits `dev` unchanged
   apart from the test-only `robin_test_support` backend.
9. **Interactive copies started cold.** A copy per worktree name meant every
   new worktree compiled ~1800 crates (60m14 and 15m53 builds on 2026-11-21).
   Checks now take the first free of `ROBIN_BUILD_SLOTS` warm pool copies; an
   admitted caller leaves the FIFO queue, so a held copy never blocks a free
   one. `--sync` keeps the worktree's own copy for hand-run commands.
   `nix develop` evaluation was suspected too and measured at 6.1 s cold and
   3.4 s warm per call on `gate-shared`, about 10 s per gate: not worth a
   cached devShell profile and its staleness risk.

## Splitting the two large crates

A module-coupling scan (top-level modules, `crate::<module>` references)
shows why item 7 needs a compile-driven refactor rather than a mechanical move:

- `robin_engine`: the `engine` module is 11.7 MB of the crate, references 84
  sibling modules and is referenced by 34. `element` is used by 39 modules,
  `position_interface` by 26, `fast_find_grid` by 21, and each of those reaches
  back into `engine` or `element`. Every large module sits on that cycle.
- `robin_rs`: `host` is used by 43 modules and `renderer` by 33;
  `game_session` (2.1 MB) references 81. The modules with no in-crate
  dependency (`sound`, `campaign_progress`, `mouse_way`) total about 200 KB.

Seams that look free are not: `legacy_save` (775 KB, two inbound uses) and the
`test-helpers` mutation surface both write `EngineInner` state directly, which
`engine_facade_contract` keeps out of other crates. The geometry layer was the
one cluster whose outbound references reduced to movable value types
(`sector_production::Type`, `sound_cache::Material`, `engine::diagnostics`), so
it is the part that moved. The next step that would pay is breaking `element`
free of `engine` behind data-only types, which changes visibility across
hundreds of call sites and has to be compiled step by step.
