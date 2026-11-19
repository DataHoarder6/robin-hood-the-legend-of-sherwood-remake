# The clippy gate

What `bash scripts/check-quality.sh lint` proves, what it cost to turn on, and
the two places in the tree that answer a lint with an allowance instead of a
change. `docs/TESTING.md` owns the suite table and the push-gate wiring; this
file owns the evidence.

Last verified: 2026-11-19 against `fork/main` `48f8ee879` plus the cleanup on
branch `clippy-gate`, on `build-host`, inside `nix develop`.

## What it proves

The suite is two Clippy runs at `-D warnings`:

- `--workspace --all-targets` with default features — every crate, its tests,
  examples and binaries.
- `-p robin_rs --all-targets --no-default-features --features release` — the
  shipping client.

One run would lint almost nothing: the workspace's `default-members` are two
small crates, and `robin_rs`'s `default = []` compiles a fraction of the client.
`release` is the feature set the packaged game ships.

There is no `-A` on the command line and no `--all-features`: feature choices in
this workspace are deliberate, and a lint silenced from the outside is a gate
that proves less than it claims.

## What it cost

| Configuration | Before | After |
| --- | --- | --- |
| workspace, default features | 34 sites, 9 lints | 0 |
| `robin_rs`, `release` features | 45 sites, 15 lints | 0 |

"Before" is the tree this branch picked up after rebasing onto `fork/main`; the
first pass over the branch's own six commits had already taken it from 451 sites
(39 lints) to that point.

The workspace remainder was `dead_code` 9, `field_reassign_with_default` 11,
`unnecessary_mut_passed` 3, `chunks_exact_to_as_chunks` 3, `collapsible_if` 2,
`doc_lazy_continuation` 2, `clone_on_copy` 2,
`default_constructed_unit_structs` 1 and `cloned_ref_to_slice_refs` 1. The
`release` run adds the code the `multiplayer`, `script-rpc` and desktop features
compile: `collapsible_if` 9, `field_reassign_with_default` 13,
`unnecessary_mut_passed` 3, `chunks_exact_to_as_chunks` 3, `clone_on_copy` 3,
`byte_char_slices` 3, `doc_lazy_continuation` 2, `items_after_test_module` 2,
`single_match` 1, `collapsible_match` 1, `question_mark` 1,
`unnecessary_lazy_evaluations` 1, `needless_borrow` 1,
`cloned_ref_to_slice_refs` 1, `disallowed_methods` 1.

All but one are real changes: collapsed conditions, struct literals instead of
field reassignment, `as_chunks::<4>()`, `slice::from_ref`, test modules moved
below the code they test, and `#[cfg]` gating for items only one feature set
uses.

## The allowances

Two, both narrow, both with the reason at the call site.

| Site | Lint | Why |
| --- | --- | --- |
| `crates/robin_rs/src/multiplayer.rs`, `attach_runtime` | `unnecessary_map_on_constructor` | Without `multiplayer`, `MultiplayerRuntime` is uninhabited, so `Some(runtime.into())` is unreachable and rustc lints the rewrite; the `map` is the spelling that compiles in both feature sets. |
| `crates/robin_rs/src/multiplayer/matchmaking.rs`, `encode_topic_broadcast` | `disallowed_methods` | The topic nonce must differ per broadcast (gossip suppresses repeats), so it comes from ambient randomness on purpose. Matchmaking never reaches `Engine::perform_hourglass`, so it cannot desync the simulation — the exception `crates/robin_rs/clippy.toml` describes. |

Neither is a crate-wide blanket, and neither is `#![allow(warnings)]`.

## The linter is the compiler's

`rust-toolchain.toml` pins `nightly-2026-10-21` and now lists `clippy` beside
`rustfmt`, `rust-src` and `rustc-codegen-cranelift-preview`. One toolchain, one
compiler, one linter:

```text
$ rustc --version
rustc 1.101.0-nightly (19833af72 2026-10-20)
$ cargo clippy --version
clippy 0.1.101 (19833af722 2026-10-20)
$ cargo fmt --version
rustfmt 1.11.0-nightly (19833af722 2026-10-20)
```

All three name the same build. `rustfmt` was never a second package: the only
`rustfmt` on `PATH` is `/nix/store/…-rustup/bin/rustfmt`, rustup's proxy, and
`rustup which rustfmt` resolves to the pinned toolchain's component. `clippy`
joins it the same way, so neither needs a package of its own in `flake.nix`.

Clippy is host-side: it is not built for `wasm32` and has no effect on the wasm
build (`docs/slop/wasm-build-host.md`).
