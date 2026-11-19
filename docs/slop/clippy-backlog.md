# The clippy backlog

Why the push gate runs no linter, what the tree still reports, and the rule for
adding the suite. `docs/TESTING.md` points here from the gate list.

Last verified: 2026-11-19 against `fork/main` `174cc2ee0` plus the cleanup on
branch `clippy-gate`, on `build-host`, inside `nix develop`.

## Why the gate had no linter

`cargo clippy` failed in the devShell:

```text
error: 'cargo-clippy' is not installed for the toolchain 'nightly-2026-10-21-aarch64-unknown-linux-gnu'.
```

`rust-toolchain.toml` pinned `rustc-codegen-cranelift-preview`, `rust-src` and
`rustfmt` but not `clippy`, so the linter was not part of the toolchain the
repository pins. It is a component now:

```toml
components = ["clippy", "rustc-codegen-cranelift-preview", "rust-src", "rustfmt"]
```

`rustfmt` was never a duplicate: `rustup which rustfmt` resolves to the pinned
toolchain, and the `/nix/store/…-rustup/bin/rustfmt` on `PATH` is rustup's proxy
for it. There is one toolchain and one source for both tools.

Clippy is host-side. It is not built for `wasm32` and has no effect on the wasm
build (`docs/slop/wasm-build-host.md`).

## The command and the remainder

The plain `cargo clippy --all-targets` is misleading in this workspace: the
default members are two small crates, so `--workspace` is required to reach the
tree, and `robin_rs`'s default features compile a small fraction of the client.

```sh
cargo clippy --locked --workspace --all-targets            # every crate, default features
cargo clippy --locked -p robin_rs --all-targets \
    --no-default-features --features release               # the shipping client
```

Measured on the two commands above:

| Configuration | Before | After |
| --- | --- | --- |
| workspace, default features | 451 sites, 39 lints | 32 sites, 8 lints |
| `robin_rs`, `release` features | not measured (the run stopped on a deny-level lint) | 44 sites, 15 lints |

The first pass ran with clippy's own `--fix` for the machine-applicable
suggestions; the rest were applied from the suggestions it printed, reviewed,
and the ones that would have broken another feature set were reverted by hand.

## What is left

Nine of the 32 remaining sites are in files another change owns, so the suite
cannot be `-D warnings` yet:

| File | Lint | Sites |
| --- | --- | --- |
| `crates/robin_engine/src/multiplayer/modal_session.rs` | `collapsible_if` | 2 |
| `crates/robin_rs/src/game_session/terminal_debriefing.rs` | `unnecessary_mut_passed` | 3 |
| `crates/robin_rs/src/http_server/pending.rs` | `doc_lazy_continuation` | 2 |
| `crates/robin_rs/src/http_server/diagnostics.rs` | `dead_code` | 2 |

The rest are ordinary work: `field_reassign_with_default` in test fixtures,
`chunks_exact_to_as_chunks`, one `default_constructed_unit_structs` that flips
between the two feature sets, and seven `dead_code` items in `observability`
that only the `script-rpc` and `multiplayer` features use.

The `release` client reports 44 sites: the ones above, 13 more in
`main_menu/multiplayer_menu.rs`, and 22 elsewhere in `robin_rs` (test fixtures,
`chunks_exact`, two test modules that sit above their code). One is a real
finding: `multiplayer/matchmaking` calls an ambient RNG without the
`#[allow(clippy::disallowed_methods)]` and comment the crate policy in
`crates/robin_rs/clippy.toml` requires.

## Adding the suite

Only when both commands above are clean. There is no `-A` on the command line
and no `--all-features`: feature choices in this workspace are deliberate.

```sh
    lint)
        cargo clippy --locked --workspace --all-targets -- -D warnings
        cargo clippy --locked -p robin_rs --all-targets \
            --no-default-features --features release -- -D warnings
        ;;
```

The push gate selects it where it selects `format`: any change under `crates/`
or to `Cargo.toml`/`rust-toolchain.toml`.

```sh
    suites+=(format client-release lint)
```
