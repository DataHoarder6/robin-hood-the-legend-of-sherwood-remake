# Script natives and the Lua surface

Where the mission-script API is defined, how a call is routed, and what the
surface does *not* offer. Read the registry rather than this page for the exact
signature of a native.

Last verified: 2026-11-15.

## One registry

`crates/robin_script_types/src/natives/signatures.rs` holds the registry macro:
every native's id, name, argument types, dispatch domain, yield policy and
whether Lua sees it. `ORIGINAL_NATIVE_COUNT` in
`crates/robin_script_types/src/natives/defs.rs` counts the natives ported from
the original game; the Rust extensions are appended after it, so no original id
moves. `NativeFn` and the name table are generated from the same macro.

Adding a capability means adding one registry row, not editing a dispatcher
list.

## Routing

`crates/robin_engine/src/natives/dispatch/mod.rs` routes by the row's
`NativeDomain` to `dispatch/{script_core,sequences,actors,ai,world,campaign}.rs`.
Each module forwards what it does not own; the chain ends in
`unreachable!(…)`, so a registered native always has an explicit arm and a
missing one panics instead of silently returning zero.

## Lua surface

`crates/robin_spellforge/src/runtime/bridge.rs` turns every registry row marked
for Lua exposure into a global of exactly its registry name, plus the alias
table in `signatures.rs`. Rows marked `no_lua` are the exceptions (world
teleport bookkeeping and similar). Lua-side helpers — `GetActor`, `GetItem`,
`GetAllActors`, `GetScroll`, `SequenceCall` and friends — live in
`crates/robin_spellforge/src/runtime/bootstrap.lua`.

## What is not there

- No `GetMoney` / `SetMoney` / `TakeMoney`. Money is the `money` persistent
  property of an NPC (`GetPersistentProperty` / `SetPersistentProperty`) and the
  party purse is `GetRansomMoney` / `SetRansomMoney` /
  `ConfiscateMoney(actor)`.
- Nothing in the dispatch chain returns a placeholder for an unimplemented
  native: an absent capability is absent from the registry.

See also: [`SPELLFORGE.md`](SPELLFORGE.md) for how a package is packaged and
snapshotted.
