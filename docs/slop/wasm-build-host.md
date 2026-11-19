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

With the C build fixed, the suite reaches `robin_rs` and stops on one real error
in the client, which this change did not touch (build configuration only):

```text
error: `long` cannot be used with `#[arg(skip)]
   --> crates/robin_rs/src/main_entry/cli.rs:190:9
    |
190 |         long,
    |         ^^^^
```

`spec` carries `#[cfg_attr(target_arch = "wasm32", arg(skip))]` and, separately,
an unconditional `#[arg(long, value_name = "PATH", requires = "headless", …)]`
on the same field, so clap's derive sees `skip` and `long` together on wasm32.
The attributes need to be one `cfg_attr` (`arg(...)` on native, `arg(skip)` on
wasm32). Until it lands, the `wasm` suite is red and is not in
`scripts/push-gate.sh`; with it, the third command compiles the client for the
browser and the suite is a compiler for every browser change.

**Limit.** The suite is `cargo check`: it compiles and archives the C but never
links, so the object-format mistake above would survive a green run. The
`cargo build` in this file is the check that catches it.
