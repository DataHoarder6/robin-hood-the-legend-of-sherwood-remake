# The wasm target builds on the build host

Evidence for `docs/TESTING.md`, "Crate-to-gate matrix": why the `wasm` suite
compiled no project code before, what makes `zstd-sys` build for `wasm32`, and
the one client error that still stops the target.

Last verified: 2026-11-19 against `fork/main` `4ff153ba6`, on `build-host`, inside
`nix develop`.

## Failure

`bash scripts/check-quality.sh wasm` — three `cargo check --profile wasm-dev
--target wasm32-unknown-unknown` runs — died in its third command while
compiling the bundled C of `zstd-sys` 2.1.2:

```text
zstd/lib/dictBuilder/cover.c:332:5: error: implicit declaration of function 'qsort_r'; did you mean 'qsort'? [-Wimplicit-function-declaration]
```

The failing compiler invocation was the host `gcc`, with no `--target` flag, and
`wasm-shim/` on its include path.

## Cause

`pkgs.gcc` is in the devShell's `packages`, so nixpkgs' setup hook exports
`CC=gcc` into the shell. cc-rs (`zstd-sys/build.rs`) reads `CC` for *every*
target, so it overrode the clang default it would otherwise pick for `wasm32`,
and the host gcc compiled the C for the host:

1. `gcc` defines `__linux__`, so `cover.c` takes its `_GNU_SOURCE` branch and
   calls `qsort_r`. `zstd-sys`' own `wasm-shim/stdlib.h` precedes the system
   header and declares only `malloc`/`calloc`/`free`/`qsort`, so the call is a
   declaration error.
2. Even where it compiled, the objects were x86-64 ELF. `cargo check` never
   links, so the archive of foreign objects survived as long as no C file
   needed a declaration the shim withheld — the wasm suite was one header away
   from a green that proved nothing.

## Fix

`flake.nix` sets the per-target compiler, which cc-rs prefers over `CC`:

```nix
CC_wasm32_unknown_unknown = "${pkgs.llvmPackages.clang-unwrapped}/bin/clang";
CXX_wasm32_unknown_unknown = "${pkgs.llvmPackages.clang-unwrapped}/bin/clang++";
```

This is the correct fix for `zstd-sys` specifically, rather than a define:

- With a compiler that actually targets wasm32, `__linux__` is absent and zstd
  takes the C90 `qsort` fallback, which the shim maps to
  `rust_zstd_wasm_shim_qsort` — the function the crate ships for this target.
  Defining `_GNU_SOURCE` would ask for a `qsort_r` that neither the shim nor a
  wasm libc provides (a link error instead of a declaration error).
- `no_wasm_shim` would push `malloc`/`qsort`/`memcpy` onto a wasm libc that
  `wasm32-unknown-unknown` does not have.
- Dropping `zdict_builder` for wasm would hide `cover.c` but leave every other
  C file in the crate compiled for the wrong machine.
- The binary is unwrapped on purpose: the nix clang wrapper injects host x86-64
  flags and rejects this target (`unsupported option
  '-fzero-call-used-regs=used-gpr' for target 'wasm32-unknown-unknown'`).

`CC`/`CXX` stay `gcc`/`g++` for native builds, which is what the rest of the
shell wants.

## What now proves it

The archived objects are wasm, not x86-64:

```text
$ ar t .../build/zstd-sys/.../out/libzstd.a | head -2
4f97f2375d6aa471-debug.o
4f97f2375d6aa471-entropy_common.o
$ file <extracted members>
...-debug.o:          WebAssembly (wasm) binary version 0x1 (MVP module)
...-entropy_common.o: WebAssembly (wasm) binary version 0x1 (MVP module)
```

With the client error below patched out of a scratch copy, the target links —
both feature sets, `cargo build --profile wasm-dev --target
wasm32-unknown-unknown -p robin_rs --bin robin --no-default-features`, and the
same command with `--features audio`:

```text
target/wasm32-unknown-unknown/wasm-dev/robin.wasm: WebAssembly (wasm) binary version 0x1 (MVP module)
```

## The client error this exposed

With the C build fixed, the suite reached `robin_rs` and stopped on one real
error in the client, which this change did not touch (build configuration only):

```text
error: `long` cannot be used with `#[arg(skip)]
   --> crates/robin_rs/src/main_entry/cli.rs:190:9
    |
190 |         long,
    |         ^^^^
```

`spec` carried `#[cfg_attr(target_arch = "wasm32", arg(skip))]` and, separately,
an unconditional `#[arg(long, value_name = "PATH", requires = "headless", …)]`
on the same field, so clap's derive saw `skip` and `long` together on wasm32.

**Fixed 2026-11-19**: the attributes are one `cfg_attr` now (`arg(...)` on
native, `arg(skip)` on wasm32), so the third command compiles the client for the
browser. A second client error sat behind it — the browser `ServerHandle::connect_string`
returned `Option<String>` where the shared call site wraps the native `String`
in `Some` — and it is fixed in the same change; a
`cargo check --features audio,multiplayer` now passes.

**Since then.** `fork/main` `174cc2ee0` failed the same third command again, for a
different reason: `crates/robin_rs/src/game_session/report.rs` called
`crate::bug_bundle`, `crate::bug_report`, `crate::crash` and
`crate::diagnostic_context`, which `lib.rs` defines only off wasm32. That module
and its one call site in `game_session/flow.rs` are now gated the same way, and
the suite's last two commands build the client with the features
`wasm-www/scripts/build-runtime.mjs` ships — `audio,multiplayer`, then the
threaded plan with `-Zbuild-std` — so a desktop-only module on a browser path
fails a gate instead of a release. `docs/TESTING.md` owns the gate's contents.

**Limit.** `cargo check` compiles and archives the C but never links, so an
object-format mistake would survive it. The `cargo build` this file records is
what catches that, and it is the suite's last two commands now;
`wasm-www/scripts/build-runtime.mjs` remains the linked production build, and it
alone checks the threaded module's shared memory import — `wasm-objdump` is not
in the devShell.
