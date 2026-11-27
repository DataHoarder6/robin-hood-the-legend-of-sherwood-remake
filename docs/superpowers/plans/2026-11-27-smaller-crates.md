# Smaller crates: splitting `robin_engine` and cutting `robin_rs` loose from it

Last verified: 2026-11-27 (fork/main `1c50fb2f8`). Every number below is
reprinted by the command next to it; re-run it before acting on a number.

## Why

A one-function edit in `robin_engine` rebuilds four units before the `robin`
binary links, measured warm and uncontended on the build host
(`cargo build -p robin_app --bin robin --timings` after the edit):

| Unit | Time | Changed source |
| --- | --- | --- |
| `robin_engine` | 18.5 s | one function |
| `robin_rs` | 14.9 s | none |
| `robin_replay_format` | 2.8 s | none |
| `robin_app` (link) | 1.7 s | none |

Two causes, each with its own fix:

1. **Incremental cost scales with the crate, not the edit.** rustc re-validates
   the whole crate's incremental state and rewrites its metadata. Fix: make the
   crate an edit lands in small (phases 3 and 4).
2. **Cargo has no early cutoff.** Any change to `robin_engine` changes its crate
   hash, so every dependent recompiles, and `robin_rs` (11 MB, untouched) loses
   most of its incremental reuse. Fix: dependents that do not depend on the
   engine at all (phases 1 and 2).

The gate pays both causes once per lane (lint, engine, client, wasm), on 12
shared cores: a warm one-function engine landing gated in 3m38.

## Where the coupling is

All commands run from the repository root and read source only.

- `python3 tools/crate-graph/impl_share.py .` — 12.4 MB of non-test engine code,
  35 % of it inside `impl EngineInner` / `impl Engine` blocks. That 35 % is the
  orchestration; the other 65 % is type definitions, their impls and free
  functions, which can move into lower crates when they do not reach back.
- `python3 tools/crate-graph/engine_graph.py .` — method-call graph between the
  files of `engine/`. 55 of its 80 units (6.1 MB) form one cycle of
  `EngineInner` method calls (`ai`, `melee`, `movement`, `level_loading`, `tick`,
  `script`, …). Inherent impls must stay in the crate that defines
  `EngineInner`, so the cycle stays in one crate; only what it calls *down to*
  can leave.
- `python3 tools/crate-graph/coupling.py crates/robin_engine` — top-level
  modules: `engine` references 69 siblings and is referenced by 33.
- `git grep -h -oE 'robin_engine::[a-z_]+' -- crates/robin_rs/src ':!*tests*' | sort | uniq -c | sort -rn`
  — what the client takes from the engine: the facade (`robin_engine::engine`)
  first, then value types (`player_command`, `replay`, `multiplayer`,
  `profiles`, `coordinates`, `campaign`, …).
- `cluster.py`, `inbound.py`, `lower.py`, `upper.py` (same directory) answer
  "what does this set of modules need from the rest", "what does the rest need
  from it", and "which sets are cycle-free"; each prints its usage.

Already below the engine: `robin_engine_types`, `robin_level_data`,
`robin_engine_geometry`, `robin_engine_script`.

## Invariants for every phase

- **Behaviour is byte-identical.** Moving a type keeps its derives, field order,
  attributes and discriminants, so state hashes (`robin_state_hash_derive`),
  bitcode/serde save, replay and wire formats do not change. The `parity`
  suite and the replay corpus are the proof, not reading.
- **No runtime regression.** No new dynamic dispatch, boxing or allocation on
  simulation or render paths. A function that moves out of the engine and is
  called per entity or per frame gets `#[inline]` so the `parity` and
  `dev-opt1` profiles (no cross-crate LTO) keep inlining it. Phase 2's one
  dynamic boundary is measured, below.
- **The facade stays closed.** `EngineInner` state is never public to host
  crates; `crates/robin_engine/tests/engine_facade_contract.rs` keeps enforcing
  it. Code that writes `EngineInner` state stays in the crate that owns it.
- **Old paths keep working through re-exports** until every user is moved in
  the same landing; then the re-export goes (no shims left behind).
- **Compile after every step.** Each move is a compile-driven loop on a pool
  copy (`scripts/remote-check.sh lint`), then the full gate when the step
  lands. One extracted crate per landing.

## Phase 0 — a rebuild probe in the repo

Make the measurement above one command so every later phase reports its effect.

- Add `scripts/measure-rebuild.sh`: syncs to a pool copy, applies a one-function
  probe to a named file, runs `cargo build -p robin_app --bin robin --timings`,
  prints the rebuilt units with their times from
  `target/cargo-timings/cargo-timing-*.html` (`UNIT_DATA`), reverts the probe.
  Probes: `crates/robin_engine/src/engine/tick.rs`,
  `crates/robin_engine_geometry/src/gate.rs`, `crates/robin_rs/src/version.rs`.
- Exit: the table in "Why" comes out of the script.

## Phase 1 — a value-type floor under the engine

Move the plain data the engine, the client and the services exchange into
crates below `robin_engine`: replay data (`replay`), `player_command`,
multiplayer wire messages (`multiplayer`), `game_operation`, `coop`, and the
data halves of `campaign`, `mission`, `achievement`, `player_profile` and
`campaign_history`.

- For each module, `inbound.py` lists what the engine needs from it and
  `cluster.py` what it needs from the engine. Data and pure methods move; an
  impl block that touches `EngineInner` becomes a free function or an
  extension trait in the engine (orphan rule: a trait defined in the engine
  may be implemented there for a moved type).
- New crates by stability, not by subject: `robin_sim_values` (replay,
  commands, wire messages) and `robin_campaign_values` (campaign, mission,
  profile, achievement data). Fewer, larger value crates rebuild less than
  many small ones that depend on each other.
- Exit:
  - `cargo tree -p robin_replay_format -e normal` (and the same for
    `robin_replay_verifier`, `robin_highscores`, `robin_replay_admission_wasm`)
    contains no `robin_engine`.
  - The probe shows `robin_replay_format` no longer rebuilding on an engine
    edit.
- Payoff: the services and replay crates stop rebuilding on engine edits;
  phase 2 needs these types below the facade.

## Phase 2 — the client talks to an API crate, not to the engine

`robin_rs` stops depending on `robin_engine`. A new `robin_engine_api` crate
holds:
- one object-safe trait for the facade the client uses today
  (`robin_engine::engine::Engine`'s public methods, the list `rg 'impl Engine'`
  and `engine_facade_contract.rs` already define);
- the data types that cross the boundary (display state, snapshots,
  presentation views). They come from phase 1's floor where they already live.

`robin_engine` implements the trait. `robin_app` constructs the engine and hands
the client a `Box<dyn SimulationApi>`.

- Why dynamic dispatch here and nowhere else: it is the only way the client's
  compiled code stops depending on the engine's crate hash. Generics would move
  monomorphization of the whole client into `robin_app`, which then
  recompiles on every engine edit instead.
- Runtime proof: the facade is called per frame and per command, not per
  entity in the simulation; count the calls on a replay
  (`--replay` of the parity corpus with a counting wrapper) and compare
  frames per second of the parity replay runner before and after. Any
  measurable regression stops the phase.
- Tests: client tests that need a real engine take `robin_engine` as a
  dev-dependency, so only test builds see it.
- Exit:
  - `cargo tree -p robin_rs -e normal` contains no `robin_engine`.
  - The facade contract test asserts the same at the crate boundary.
  - The probe shows `robin_rs` at 0 s on an engine edit.
- Payoff: the 14.9 s `robin_rs` rebuild goes away from every engine edit
  outside test builds. The gate's lint lane stops re-checking the client for
  engine-only changes.

## Phase 3 — move the engine's pure layers down

The 65 % of engine code outside `impl EngineInner` is where the engine shrinks.
Work subsystem by subsystem, lowest first:

1. **Navigation and movement math** (`pathfinder`, the pure parts of
   `engine/movement*`, `jump`, `door_pass`) → `robin_engine_nav`, above
   `robin_engine_geometry`.
2. **Combat rules** (`bow_shot`, `combat`, the damage and hit computations in
   `engine/melee`, `engine/archery`) → `robin_engine_combat`.
3. **AI decisions** (`ai`, `ai_enemy`, `ai_friendly`, `ai_vision`,
   `ai_detectable_filter`, the decision functions in `engine/ai`) →
   `robin_engine_ai`, taking read-only views of world state as arguments.
4. **Sequences and animation data** (`sequence`, `sprite`, `animation` tables)
   → `robin_engine_sequence`.

The refactor pattern, per function: a method that both computes and mutates
splits into a pure function that moves (arguments in, result out) and a thin
method that stays in the engine and applies the result. Before moving a set, run
`cluster.py crates/robin_engine <set>`: the move is ready when its outbound list
holds only crates below.

- Exit per crate:
  - The new crate's `cargo tree -i` shows only the engine and above as
    dependents.
  - `impl_share.py` shows the engine's non-test size falling.
  - The probe's engine-edit time falls.
- Target: `robin_engine` under 5 MB of non-test source, and a one-function
  engine edit under 8 s for the engine unit.

## Phase 4 — tests move with their code

The engine's unit tests compile into its lib-test build on every gate. Tests of
code moved in phase 3 move into the new crate's own tests, where they compile
only when that crate changes and in parallel with the engine.

- Exit: the engine lane's test build time (gate log, `Finished test profile`)
  falls with each phase 3 crate.

## Phase 5 — the gate follows the graph

With finer crates, `scripts/push-gate.sh` selects suites by reverse
dependencies of the changed crates (`cargo tree -i -e normal,dev`). An edit in a
phase 3 crate gates that crate, the engine and the client tests that
dev-depend on it; an edit in the client gates the client alone.

- Exit: a client-only landing compiles nothing in the engine lane.

## Expected result

These are estimates, to be replaced by the probe's output after each phase. For a
one-function engine edit:
- the engine unit falls from 18.5 s to under 8 s (phase 3);
- `robin_rs` falls from 14.9 s to 0 s outside test builds (phase 2);
- `robin_replay_format` falls from 2.8 s to 0 s (phase 1).

The client lane's test build still recompiles the client's test target on
engine edits, because those tests use the real engine. Bringing the whole gate
under a minute for engine landings needs either that lane on its own machine or
client tests against the API with a test double. That is a coverage decision, not
part of this plan.

## Not in scope

- **`legacy_save` and the `test-helpers` mutators.** Both write `EngineInner`
  state; moving them would open the facade.
- **Replacing derives or macros for codegen size.** It changes wire-type code
  shape for no proven gain.
- **Blind refactoring without compiling.** Every phase step compiles before the
  next.
