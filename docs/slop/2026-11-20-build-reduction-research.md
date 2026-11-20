# Reducing builds across many worktrees — primary-source research

Last verified: 2026-11-20. Question: with ~1800 crates and ~20 worktrees on one
12-core host, what actually reduces builds? Two separate levers:

- **Avoid triggering** — do no compiler work at all for a tree or a path.
- **Make cheaper** — still compile, but reuse or shorten the work.

The build-host configuration is owned by
[`BUILD_CACHE_AND_QUEUE.md`](BUILD_CACHE_AND_QUEUE.md): §1 is the earlier
multi-copy sccache recipe, §5 is the current 2026-11-20 correction (one stable
`gate-shared` copy, Cargo incremental enabled). This file does not restate
either recipe. It records the primary claims behind them, the gaps in them, and
the options not already in this repo. Repo state comes from
[`AGENTS.md`](../../AGENTS.md), [`docs/TESTING.md`](../TESTING.md),
[`docs/PARALLEL_AGENTS.md`](../PARALLEL_AGENTS.md), `Cargo.toml`,
`.cargo/config.toml` and `scripts/push-gate.sh`.

## 1. Avoid triggering a build

### Path-filter the trigger, not just the suites

`on.push.paths` / `paths-ignore` run a workflow only when changed files match;
branch and path filters must *both* be satisfied
([GitHub Actions workflow syntax](https://docs.github.com/en/actions/writing-workflows/workflow-syntax-for-github-actions#onpushpull_requestpull_request_targetpathspaths-ignore)).
Official caveats that defeat naive use:

- A job skipped by path filtering stays **Pending**, so a PR that requires that
  check cannot merge
  ([same doc, path filters](https://docs.github.com/en/actions/writing-workflows/workflow-syntax-for-github-actions#onpushpull_requestpull_request_targetpathspaths-ignore)).
- Diffs use two-dot for pushes and three-dot for PRs; >1000 commits or a diff
  timeout forces a run; >3000 files can suppress one
  ([same doc, Git diff comparisons](https://docs.github.com/en/actions/writing-workflows/workflow-syntax-for-github-actions#onpushpull_requestpull_request_targetpathspaths-ignore)).
- Path filters are not evaluated for tag pushes
  ([same doc](https://docs.github.com/en/actions/writing-workflows/workflow-syntax-for-github-actions#onpushpull_requestpull_request_targetpathspaths-ignore)).

Here the landing gate is `scripts/push-gate.sh`, not Actions: it already maps
path prefixes to suites, so a Python-only diff pays for no Rust build. Four
Actions workflows exist, none is the landing gate, and only
`deploy-static-workers.yml` has a `paths` filter. Adding filters to *required*
Actions checks would trade build time for blocked landings.

### Affected-target selection

Bazel's `rdeps(u, x)` finds reverse dependencies of `x` inside universe `u`;
`somepath` / `allpaths` show the dependency chain
([Bazel query guide](https://bazel.build/query/guide#reverse-dependencies)).
Cargo's equivalent is `cargo tree --invert/-i`, which shows reverse deps; in a
workspace it needs `--workspace` to span members
([cargo-tree(1)](https://doc.rust-lang.org/cargo/commands/cargo-tree.html#tree-options)).
Cargo's own warning applies: `cargo tree` is close to, but does not guarantee,
what a build compiles
([cargo-tree(1), Feature Unification](https://doc.rust-lang.org/cargo/commands/cargo-tree.html#description)).
Use reverse deps to *add* suites conservatively, never to skip a proof the gate
owes; `push-gate.sh` stays the single definition of what a landing owes
([`docs/TESTING.md`](../TESTING.md#testing-and-refactor-gates)).

### `cargo check` as a preflight

`cargo check` compiles without the final code-generation step and is faster
than `cargo build`; it saves metadata for reuse, but errors emitted only during
code generation are not reported
([cargo-check(1)](https://doc.rust-lang.org/cargo/commands/cargo-check.html#description)).
It uses the dev profile by default
([Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html#profile-selection)).
Here `client-check` is in flight as a local-only suite; keep it a preflight,
not a replacement. See §2 for why sccache cannot share its results.

### Tree/hash-keyed caching

Two established models:

- Nix: input-addressed store paths hash the derivation graph; content-addressed
  store paths hash the path contents, which enables deduplication, early cutoff
  and unprivileged closure copying
  ([Nix 2.4 release notes](https://nix.dev/manual/nix/2.24/release-notes/rl-2.4)).
  Binary cache stores (HTTP, local, S3) and substituters are the sharing layer
  ([Nix store types](https://nix.dev/manual/nix/2.24/store/types/)).
- Bazel: a remote cache is an **action cache** (action hash → result metadata)
  plus a **content-addressable store** of output files; the disk cache exists
  specifically for "switching branches and/or working on multiple workspaces of
  the same project, such as multiple checkouts"
  ([Bazel remote caching](https://bazel.build/remote/caching#disk-cache)).
  Its known failure modes — environment variables (e.g. `PATH`) leaking into
  action hashes and tools outside the workspace not being tracked — are the
  exact class of problem a key must handle
  ([same doc](https://bazel.build/remote/caching#known-issues)).

Cargo has no shipped equivalent. The per-user compiled-artifact cache
([cargo#5931](https://github.com/rust-lang/cargo/issues/5931)) is labelled
`A-caching` / `S-needs-design` — **not accepted**. Cargo maintainers have
prototyped a CAS-backed build cache: a five-worktree Alacritty benchmark went
from 2 m 53 s to 42 s with aggressive caching, and from 5.8 GB to 3.5 GB total
with 596 MB of cache, but the design is still being revised
([cargo#5931, maintainer comments](https://github.com/rust-lang/cargo/issues/5931)).
Do not plan around it shipping. Here, the "same tree builds once" half is now
done by one stable locked `gate-shared` copy and Cargo's own freshness check —
not by a hash store and not by tree-keyed copies, which were removed today
([`BUILD_CACHE_AND_QUEUE.md`, §5](BUILD_CACHE_AND_QUEUE.md)).
A Cargo-native CAS is not available.

## 2. Make the builds that remain cheaper

### Cargo's cache model and dep-info

Cargo splits final artifacts (`target-dir`) from intermediates (`build-dir`);
both default to `target/`, the split landed in Cargo 1.100, and the build-dir
layout is internal and may change
([Build cache](https://doc.rust-lang.org/cargo/reference/build-cache.html)).
Next to each artifact a `.d` dep-info file lists rebuild dependencies; paths
are absolute by default and `build.dep-info-basedir` can strip a prefix
([Build cache, Dep-info files](https://doc.rust-lang.org/cargo/reference/build-cache.html#dep-info-files),
[Cargo config](https://doc.rust-lang.org/cargo/reference/config.html#builddep-info-basedir)).
The build-dir split does not delete intermediates or stop per-worktree
multiplication
([cargo#16804, maintainer reply](https://github.com/rust-lang/cargo/issues/16804)).

### sccache: what is actually hashed, and the two contradictions

sccache computes a crate's key with `rustc --emit=dep-info` (source files) and
`rustc --print file-names` (outputs), then reuses it only for the same compiler,
arguments, working directory, environment, externs, static libraries and target
spec while every listed source keeps the same contents. A file the dep-info did
not list does not invalidate the entry
([sccache `docs/Rust.md`](https://github.com/mozilla/sccache/blob/main/docs/Rust.md)).
This is a shared-cache design, not Cargo fingerprint state.

- **Contradiction 1 — "sccache cannot share across worktrees".** Official docs
  say the opposite: by default absolute paths must match, and `SCCACHE_BASEDIRS`
  exists to strip base directories so builds from "different absolute root
  paths", "Git worktrees" and "multiple project checkouts" share
  ([sccache README](https://github.com/mozilla/sccache/blob/main/README.md#normalizing-paths-with-sccache_basedirs),
  [Configuration.md](https://github.com/mozilla/sccache/blob/main/docs/Configuration.md#env)).
  But basedirs are necessary, not sufficient: the key also contains the compiler
  arguments, working directory, externs and target spec, so a copy with a fresh
  `target/` still misses even when its sources are identical
  ([sccache Rust.md](https://github.com/mozilla/sccache/blob/main/docs/Rust.md)).
  That is why tree-keyed copies measured 0.36 % Rust hits here. The earlier
  "compiler cache is not the answer" note in `docs/TESTING.md` described that
  multi-copy setup; the current design keeps sccache for the C/C++ and native
  hits it does deliver. `--remap-path-prefix` is itself an argument in that key.
- **Contradiction 2 — "cargo check hits the shared cache".** Only `link`,
  `metadata` and `dep-info` are accepted emit values and `link` must be present
  ([sccache `docs/Rust.md`](https://github.com/mozilla/sccache/blob/main/docs/Rust.md)).
  `cargo check` emits `.rmeta` via `--emit=metadata` with no link
  ([rustc `--emit`](https://doc.rust-lang.org/rustc/command-line-arguments.html#--emit-specifies-the-types-of-output-files-to-generate)),
  so it is never a hit. Treat it purely as a cheap type gate.

Other primary facts relevant here:

- Crates that invoke the system linker (`bin`, `dylib`, `cdylib`,
  `proc-macro`) are never cached; the README suggests converting a large `bin`
  to a `lib` plus a thin wrapper
  ([sccache README, Rust caveats](https://github.com/mozilla/sccache/blob/main/README.md#rust)).
  `robin_rs` already has an `rlib` plus thin bins, so this lever is spent.
- Incremental crates are not cached by default; `CARGO_INCREMENTAL=1` makes
  sccache exit unless `SCCACHE_ALLOW_INCREMENTAL=1` (incremental crates go
  uncached) or `=cache` (they are cached, but a hit leaves the incremental dir
  untouched). `CARGO_INCREMENTAL=1` and the profile default also produce
  different keys
  ([sccache `docs/Rust.md`](https://github.com/mozilla/sccache/blob/main/docs/Rust.md)).
  `CARGO_INCREMENTAL` overrides the profile
  ([Cargo config](https://doc.rust-lang.org/cargo/reference/config.html#buildincremental)).
  `=1` plus `SCCACHE_ALLOW_INCREMENTAL=1` was trialled with the stable copy and
  reverted: it broke a real landing gate. Gate builds keep `=0`;
  [ADR 0007](../adr/0007-no-cargo-incremental-in-gates.md) records why.
- With basedirs, a hit can return an artifact built in another checkout: debug
  info and panic messages name that directory unless paths are remapped, and
  `env!("CARGO_MANIFEST_DIR")` needs `SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS`
  ([README](https://github.com/mozilla/sccache/blob/main/README.md#normalizing-paths-with-sccache_basedirs),
  [Rust.md](https://github.com/mozilla/sccache/blob/main/docs/Rust.md)).
- New: `SCCACHE_CLIENT_SIDE=1` runs the compile in the client and uses the daemon
  only as a cache gateway; the docs call it "the recommended mode" and expect
  it to become the only supported configuration
  ([Configuration.md](https://github.com/mozilla/sccache/blob/main/docs/Configuration.md#env)).
- Cache size accepts one suffix (`20G`), not `20GB` or `1.5G`; one local server
  at a time
  ([sccache `docs/Local.md`](https://github.com/mozilla/sccache/blob/main/docs/Local.md)).
- Cargo implements its own jobserver, so `RUSTC_WRAPPER=sccache` under Cargo
  inherits it automatically
  ([sccache README, jobserver](https://github.com/mozilla/sccache/blob/main/README.md#interaction-with-gnu-make-jobserver)).

### rustc path remapping and incremental

`--remap-path-prefix FROM=TO` is textual and repeatable; the last matching
remapping wins
([rustc Remap source paths](https://doc.rust-lang.org/rustc/remap-source-paths.html)).
`--remap-path-scope` selects scopes (macro, diagnostics, debuginfo, coverage,
object); `all` is the default. **Scopes do not apply to rustc metadata**:
`.rmeta`, `.rlib`, dylibs and proc macros keep local paths unless the default
`all` scope is used
([same doc](https://doc.rust-lang.org/rustc/remap-source-paths.html#metadata-impact)).
The repo sets no scope, so it uses `all` — correct. Linker-generated paths
(Windows PDB, Apple OSO) are not remapped; Linux ELF is unaffected
([same doc, caveats](https://doc.rust-lang.org/rustc/remap-source-paths.html#caveats-and-limitations)).
`-C incremental` saves state for recompiles but inhibits optimizations (e.g.
more codegen units) and is not recommended for release
([rustc codegen options](https://doc.rust-lang.org/rustc/codegen-options/index.html#incremental)).
Incremental therefore helps a stable copy but is mutually exclusive with
cross-copy sccache sharing of those crates; pick one, with the official
trade-off above.

### Profiles, debuginfo, strip, codegen units

Cargo dev defaults: `opt-level = 0`, `debug = true`, `strip = "none"`,
`incremental = true`, `codegen-units = 256`; release uses `opt-level = 3`,
`incremental = false`, `codegen-units = 16`
([Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html#default-profiles)).
`line-tables-only` is the minimal debuginfo that still gives file/line in
backtraces, and `strip = "debuginfo"` removes debug sections at link time while
leaving backtraces mostly intact
([Cargo profiles, debug](https://doc.rust-lang.org/cargo/reference/profiles.html#debug),
[rustc `-C strip`](https://doc.rust-lang.org/rustc/codegen-options/index.html#strip)).
The repo already uses `line-tables-only`, `split-debuginfo = "unpacked"` and
`strip = "debuginfo"`; nothing cheaper remains without losing panic locations.

Two **unmeasured cold-build costs** in `Cargo.toml`: `profile.dev.package."*"`
sets `opt-level = 3` for every dependency, and `profile.dev.build-override`
sets `opt-level = 3` for build scripts and proc macros. Cargo's own defaults for
build dependencies are `opt-level = 0`, `codegen-units = 256`, `debug = false`
explicitly "to compile quickly"
([Cargo profiles, Build Dependencies](https://doc.rust-lang.org/cargo/reference/profiles.html#build-dependencies)),
and higher `opt-level` trades longer compiler time for faster runtime code
([same doc, opt-level](https://doc.rust-lang.org/cargo/reference/profiles.html#opt-level)).
These were chosen for debug runtime; measure a gate-only dev profile with the
Cargo defaults before changing them, because the cold build pays for ~1800
optimized crates.

### Codegen backends and linkers

Cranelift "has the potential to improve compilation times in debug mode" but
does not support full SIMD or reliable panic unwinding; `-Cpanic=abort` is its
default where unwinding is unsupported
([rustc_codegen_cranelift README](https://github.com/rust-lang/rustc_codegen_cranelift)).
The repo already uses Cranelift for workspace crates and explicit LLVM overrides
for unwind/SIMD cases; a blanket switch either way is not new.

On x86_64 Linux, rustc uses `cc` as a linker driver and already tries `lld`
(`-Clinker-features=+lld` is default, `-lld` is the stable opt-out)
([rustc codegen options, linker-features](https://doc.rust-lang.org/rustc/codegen-options/index.html#linker-features)).
LLD claims >2× the GNU gold linker on large programs
([LLD docs](https://lld.llvm.org/#performance)). Wild's own docs say it is fast
for iterative development but incremental linking "isn't yet implemented"
([wild README](https://github.com/wild-linker/wild)). Mold's own August 2026
benchmark claims it links ~1.9× faster than wild and ~4.9× faster than lld at
the median
([mold README, Benchmark](https://github.com/rui314/mold#benchmark)).
That is a vendor benchmark; the repo chose Wild already. Measure on one gate
copy before switching — switching linkers does not reduce the *number* of
builds.

### Measurement

`cargo build --timings` (also on `cargo check`) writes
`target/cargo-timings/cargo-timing.html`; the docs state the reports are
"suitable for human consumption only" and provide no machine-readable timing
data
([cargo-build(1)](https://doc.rust-lang.org/cargo/commands/cargo-build.html),
[cargo-check(1), --timings](https://doc.rust-lang.org/cargo/commands/cargo-check.html#--timings)).
Use it to find a dominant crate — phase 3 of the
[gate-throughput design](../superpowers/specs/2026-11-20-gate-throughput-design.md) —
not as a gate metric.
For the cache itself, `sccache --show-stats` is the source of truth
([sccache README, Debugging](https://github.com/mozilla/sccache/blob/main/README.md#debugging)).

### One third-party option worth a trial

Kache's own docs target this exact shape: keep one `target/` per worktree, share
an artifact store, seed new targets with already-built dependency units, and
import isolated incremental sessions between worktrees using copy-on-write
clones rather than writable hardlinks
([Kache worktrees docs](https://ninja.kunobi.com/docs/kache/getting-started/worktrees)).
A hit requires compatible source, toolchain, target, features and build
settings, and "Cargo Fresh" is not a Kache hit
([same doc](https://ninja.kunobi.com/docs/kache/getting-started/worktrees#three-kinds-of-reuse)).
It is a newer third-party tool with a daemon; trial it for **interactive**
worktrees, not the single stable gate copy, and not as a replacement for the
current setup on faith.

## 3. Verdict for this repo

| Idea | Worth it here? | One-line reason (primary source above) |
| --- | --- | --- |
| One stable locked `gate-shared` copy + Cargo incremental + one gate in flight | **Already in place — keep** | sccache keys include working dir and compiler args, so a fresh tree-keyed copy misses; Cargo's own freshness check on one copy reuses unchanged artifacts ([sccache Rust.md](https://github.com/mozilla/sccache/blob/main/docs/Rust.md), [Cargo build cache](https://doc.rust-lang.org/cargo/reference/build-cache.html)). |
| Tree-keyed gate copies (the removed design) | **No — removed today** | Fresh `target/` per tree gives new working-dir/out-dir keys and a Cargo rebuild; measured 0.36 % Rust hits ([`BUILD_CACHE_AND_QUEUE.md`, §5](BUILD_CACHE_AND_QUEUE.md)). |
| `push-gate.sh` path→suite selection; no builds for no gated path | Already in place | Equivalent of path-filtered CI without the Pending-check trap ([GHA docs](https://docs.github.com/en/actions/writing-workflows/workflow-syntax-for-github-actions#onpushpull_requestpull_request_targetpathspaths-ignore)). |
| `client-check` / `cargo check` preflight | In flight — keep as preflight only | Faster, skips codegen, but codegen-only errors are missed and sccache cannot cache metadata-only emits ([cargo-check](https://doc.rust-lang.org/cargo/commands/cargo-check.html#description), [sccache Rust.md](https://github.com/mozilla/sccache/blob/main/docs/Rust.md)). |
| `SCCACHE_CLIENT_SIDE=1` on the build host | Yes, for the C/C++ hits that remain | Upstream calls it the recommended and future-only mode ([Configuration.md](https://github.com/mozilla/sccache/blob/main/docs/Configuration.md#env)). |
| Measure a gate profile with Cargo-default `opt-level` for deps and build deps | Trial — biggest untested cost | Cargo deliberately does not optimize build deps "to compile quickly"; ~1800 deps are optimized today ([profiles](https://doc.rust-lang.org/cargo/reference/profiles.html#build-dependencies)). |
| Path filters on non-required Actions jobs | Yes, minor | Avoid CI builds on docs-only pushes; never filter required checks ([GHA docs](https://docs.github.com/en/actions/writing-workflows/workflow-syntax-for-github-actions#onpushpull_requestpull_request_targetpathspaths-ignore)). |
| Refine suite selection with `cargo tree -i --workspace` | Trial, conservatively | Reverse deps can drop a heavy suite; Cargo warns the tree is not exactly what a build compiles ([cargo-tree](https://doc.rust-lang.org/cargo/commands/cargo-tree.html#tree-options)). |
| Trial Kache for interactive worktrees | Trial | Only tool whose own docs target per-worktree target dirs and incremental state ([Kache docs](https://ninja.kunobi.com/docs/kache/getting-started/worktrees)). |
| Measure mold against Wild on one copy | Trial, do not switch on faith | Mold's own benchmark claims it is faster, but Wild is already configured ([mold](https://github.com/rui314/mold#benchmark), [wild](https://github.com/wild-linker/wild)). |
| `cargo build --timings` on a gate copy | Yes, as a measurement | Human HTML only; use it to find the dominant crate ([cargo-check](https://doc.rust-lang.org/cargo/commands/cargo-check.html#--timings)). |
| Shared `CARGO_TARGET_DIR` across worktrees | **No** | Repo forbids it; sharing is unsafe/serialising and Cargo uses relative workspace paths in stable hashes, causing collisions ([cargo#16804](https://github.com/rust-lang/cargo/issues/16804), [cargo#5931](https://github.com/rust-lang/cargo/issues/5931)). |
| Wait for Cargo's built-in content-addressed cache | **No** | `S-needs-design`, not accepted; maintainer prototype is promising but unshipped ([cargo#5931](https://github.com/rust-lang/cargo/issues/5931)). |
| `CARGO_INCREMENTAL=0` as a universal rule; `debug = 0`; whole-workspace backend/linker swaps; sccache distributed compilation | **No** | Each is correct only for another design or already chosen; one 12-core host has no second machine to distribute to ([sccache Rust.md](https://github.com/mozilla/sccache/blob/main/docs/Rust.md), [profiles](https://doc.rust-lang.org/cargo/reference/profiles.html#debug), [cranelift](https://github.com/rust-lang/rustc_codegen_cranelift), [README](https://github.com/mozilla/sccache/blob/main/README.md)). |
