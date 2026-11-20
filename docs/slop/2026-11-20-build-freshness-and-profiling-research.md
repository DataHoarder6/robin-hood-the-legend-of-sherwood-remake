# Build freshness and where Rust build time goes: primary-source research

What makes Cargo recompile an unchanged tree, and how to measure build time before
changing anything. Reference doc; every external claim cites a primary source.
Scope excludes sccache configuration, linkers, codegen backends, CI path filters
and the "fewer builds" verdict: [`BUILD_CACHE_AND_QUEUE.md`](BUILD_CACHE_AND_QUEUE.md)
owns the build-host/cache/queue side, and a sibling doc owns the optimization
verdict. This doc only answers *why* and *how to measure*.

Last verified: 2026-11-20.

## 1. What makes Cargo call a crate dirty

### 1.1 The fingerprint is a hash of the unit's inputs, plus its dependencies' hashes

Cargo stores one directory per unit in `target/{debug,release}/.fingerprint/`:
a 16-hex-digit fingerprint hash, a `.json` detail file (only used to explain dirtiness),
a translated dep-info file and an `invoked.timestamp`. A missing fingerprint means
dirty; any changed fingerprint field means dirty; each unit also tracks its
dependencies' fingerprints so dirtiness propagates up the graph
([Cargo `fingerprint/mod.rs`](https://github.com/rust-lang/cargo/blob/master/src/cargo/core/compiler/fingerprint/mod.rs)).
Run `CARGO_LOG=cargo::compiler::fingerprint=trace cargo build` to print the `.json`
reason ([Cargo `fingerprint/mod.rs`](https://github.com/rust-lang/cargo/blob/master/src/cargo/core/compiler/fingerprint/mod.rs)).

Inputs that land in the unit hash include the package, version, features, profile,
compiler version and dependency hashes; a unit's `rustflags` also go into the
artifact file name, so changing Rust flags rebuilds it
([Cargo `fingerprint/mod.rs`](https://github.com/rust-lang/cargo/blob/master/src/cargo/core/compiler/fingerprint/mod.rs),
[Cargo issue #12516, maintainer comment](https://github.com/rust-lang/cargo/issues/12516#issuecomment-6842817158)).
One important exclusion: Cargo leaves `rustflags` containing `--remap-path-prefix`
out of the hash so path remapping cannot make a build less reproducible
([Cargo issue #12516, comment](https://github.com/rust-lang/cargo/issues/12516#issuecomment-6841949704)).

### 1.2 mtime, not content hashing, is the default freshness test

Two mtime comparisons decide freshness
([Cargo `fingerprint/mod.rs`](https://github.com/rust-lang/cargo/blob/master/src/cargo/core/compiler/fingerprint/mod.rs)):

1. Each unit's newest output is compared with the outputs of all its dependencies;
   a missing output or a newer dependency output makes the unit stale.
2. Each source file's mtime is compared with the unit's dep-info file, which is
   the anchor for "when was this last built"; a missing input or an input newer
   than the dep-info makes the unit stale.

`rustc --emit=dep-info` writes a Makefile-style `.d` file listing every source file
used; Cargo translates it to a compact binary file in the fingerprint directory and
uses *that* file's mtime as the reference
([Cargo `fingerprint/mod.rs`](https://github.com/rust-lang/cargo/blob/master/src/cargo/core/compiler/fingerprint/mod.rs),
[rustc `--emit`](https://doc.rust-lang.org/rustc/command-line-arguments.html#--emit-specifies-the-types-of-output-files-to-generate)).
After a build Cargo rewinds the dep-info mtime to the build's start time, recorded
via `invoked.timestamp`, so a file modified during the build is conservatively
rebuilt next time
([Cargo `fingerprint/mod.rs`](https://github.com/rust-lang/cargo/blob/master/src/cargo/core/compiler/fingerprint/mod.rs)).
"Equal mtime" is treated as stale on the mtime path, because filesystem timestamp
precision is unreliable ([Cargo `fingerprint/mod.rs`](https://github.com/rust-lang/cargo/blob/master/src/cargo/core/compiler/fingerprint/mod.rs)).
Registry and git package sources are deliberately excluded from dep-info, so they
get no mtime check; they are immutable
([Cargo `fingerprint/mod.rs`](https://github.com/rust-lang/cargo/blob/master/src/cargo/core/compiler/fingerprint/mod.rs)).

Cargo itself calls mtime tracking "notoriously imprecise and problematic" and a
balance of performance, simplicity and completeness
([Cargo `fingerprint/mod.rs`](https://github.com/rust-lang/cargo/blob/master/src/cargo/core/compiler/fingerprint/mod.rs)).
The unstable alternative is `-Z checksum-freshness` / `build.fingerprint = "content"`,
which replaces mtimes with a file checksum. It is intended for poor-mtime systems
and CI/CD, but does **not** affect build-script `rerun-if-changed` inputs, which
keep using mtimes
([Cargo unstable, `checksum-freshness`](https://doc.rust-lang.org/cargo/reference/unstable.html#checksum-freshness)).

### 1.3 Environment variables and Rust flags invalidate a crate

Cargo parses `# env-var:...` comments in dep-info files. If a referenced environment
variable changes, the crate is recompiled even when its sources did not — this
includes `env!`/`option_env!` values and generated metadata such as
`CARGO_PKG_DESCRIPTION`
([Cargo `fingerprint/mod.rs`](https://github.com/rust-lang/cargo/blob/master/src/cargo/core/compiler/fingerprint/mod.rs)).
`RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` pass flags to *every* compiler invocation;
changing them changes the unit hash
([Cargo environment variables](https://doc.rust-lang.org/cargo/reference/environment-variables.html),
[Cargo issue #12516](https://github.com/rust-lang/cargo/issues/12516#issuecomment-6842817158)).
A bare `RUSTFLAGS` **replaces** all config `rustflags`; `CARGO_TARGET_<TUPLE>_RUSTFLAGS`
merges with them ([this repo's `.cargo/config.toml`](../../.cargo/config.toml),
[Cargo environment variables](https://doc.rust-lang.org/cargo/reference/environment-variables.html)).
`CARGO_INCREMENTAL=1`/`0` forces incremental on/off; `CARGO_PROFILE_<name>_INCREMENTAL`
and `build.incremental` are the config forms
([Cargo environment variables](https://doc.rust-lang.org/cargo/reference/environment-variables.html)).

### 1.4 Build scripts and proc macros

By default Cargo re-runs a build script if *any* file in the package changes; every
script should emit at least one `rerun-if` instruction to narrow that
([Cargo build scripts, "Change detection"](https://doc.rust-lang.org/cargo/reference/build-scripts.html)).
`cargo::rerun-if-changed=PATH` uses only mtime, scans a directory recursively if given
a directory, and `rerun-if-changed=build.rs` is the way to disable blanket re-runs;
`rerun-if-env-changed=NAME` compares the variable as received by the `cargo`
invocation, not the value visible inside `build.rs`
([Cargo build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html),
[Cargo issue #12403, maintainer explanation](https://github.com/rust-lang/cargo/issues/12403)).
Since Rust 1.46, `env!`/`option_env!` in source are tracked automatically, so
`rerun-if-env-changed` is redundant for those
([Cargo build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html)).

The build-script unit is a separate `custom-build` unit: it has no dep-info file,
its `invoked.timestamp` lives in the build output directory, and Cargo rebuilds the
script itself when its sources or dependencies change
([Cargo `fingerprint/mod.rs`](https://github.com/rust-lang/cargo/blob/master/src/cargo/core/compiler/fingerprint/mod.rs)).
Build scripts and proc macros are host artifacts; all profiles default their
`build-override` to `opt-level = 0`, `codegen-units = 256`, `debug = false`, so their
compile cost is usually small but their *execution* is on the critical path
([Cargo profiles, "Build Dependencies"](https://doc.rust-lang.org/cargo/reference/profiles.html#build-dependencies)).
Incremental compilation applies only to workspace members and path dependencies
([Cargo profiles, `incremental`](https://doc.rust-lang.org/cargo/reference/profiles.html#incremental));
the `dev` profile defaults it to on, `release` to off
([Cargo profiles, defaults](https://doc.rust-lang.org/cargo/reference/profiles.html#default-profiles)).

The Cargo FAQ lists the recurring real-world causes of *spurious* rebuilds: a
`rerun-if-changed` that names a nonexistent file, different feature sets between
two successive commands (e.g. whole-workspace then single-crate), odd filesystem
timestamp behaviour, and a concurrent process touching files or deleting artifacts
([Cargo FAQ](https://doc.rust-lang.org/cargo/faq.html#why-is-cargo-rebuilding-my-code)).

## 2. Why re-creating a worktree at the same commit still rebuilds

Cargo's rebuild decision uses relative paths and mtimes, not file content, so the
same tree at a new absolute path is not recognized as the same build.

- The Cargo team states the mechanism directly: (1) Cargo uses relative paths so
  the cache is not invalidated by moving the source; (2) mtimes drive rebuild
  detection; (3) mtime is used for *overwriting* a cache entry, not creating a new
  one. The team says it is "unlikely to change the relative paths because of the
  benefits", and notes `-Z checksum-freshness` would help detection but not build
  scripts
  ([Cargo issue #12516, maintainer comment](https://github.com/rust-lang/cargo/issues/12516#issuecomment-5194388630)).
- Path dependencies are hashed relative to the workspace root:
  `SourceId::stable_hash` strips the workspace prefix "so the same source will give
  the same hash in different locations, helping reproducible builds"
  ([Cargo `source_id.rs`](https://github.com/rust-lang/cargo/blob/master/src/cargo/core/source/source_id.rs),
  [quoted by a Cargo member](https://github.com/rust-lang/cargo/issues/12516#issuecomment-1681945066)).
  Two worktrees of one repo therefore produce the *same unit hash* for their local
  crates. Under a shared build directory this has caused Cargo to silently reuse
  another checkout's artifacts and run the wrong code; the issue is open and labelled
  `C-bug`/`A-rebuild-detection`/`S-needs-design`
  ([Cargo issue #12516](https://github.com/rust-lang/cargo/issues/12516)).
- Path remapping also interacts with dep-info: `--remap-path-prefix` rewrites paths
  in the emitted `.d` file, which made Cargo fail to parse dep-info and broke
  rebuild detection; a maintainer filed a follow-up rustc issue for it
  ([Cargo issue #7211](https://github.com/rust-lang/cargo/issues/7211)).
  The rustc book notes that `.rmeta`, `.rlib`, dylibs and proc-macros retain local
  paths unless the remap scope is `all`
  ([rustc, "Remap source paths"](https://doc.rust-lang.org/rustc/remap-source-paths.html#metadata-impact)).
- Consequence of the mtime rule: a fresh checkout writes its source files at
  checkout time, so they are newer than a fingerprint recorded by an earlier build;
  the mtime comparison — not a content hash — marks them stale. (Inference from the
  rule in [Cargo `fingerprint/mod.rs`](https://github.com/rust-lang/cargo/blob/master/src/cargo/core/compiler/fingerprint/mod.rs),
  not a separate documented claim.)
- Target-directory locality matters because `build.target-dir` / `CARGO_TARGET_DIR`
  only name where output goes; they carry no locking or identity guarantee
  ([Cargo config](https://doc.rust-lang.org/cargo/reference/config.html#buildtarget-dir)).
  The Cargo team says it "has not endorsed" shared-target workflows and lists
  conflicting cache entries, lock contention and cache poisoning as known problems
  ([Cargo issue #12516, maintainer comment](https://github.com/rust-lang/cargo/issues/12516#issuecomment-5194388630)).
  This repo's [`AGENTS.md`](../../AGENTS.md) forbids redirecting the target dir at
  all; each gate copy keeps its own `target/`
  ([`scripts/remote-check.sh`](../../scripts/remote-check.sh),
  [`BUILD_CACHE_AND_QUEUE.md`](BUILD_CACHE_AND_QUEUE.md)).

## 3. `cargo build --timings` / `-Z timings`: what it shows and how to read it

`--timings` is stable (the `-Z timings` tracking issue is closed) and writes
`target/cargo-timings/cargo-timing.html` plus a timestamped copy of every report
([Cargo timings](https://doc.rust-lang.org/cargo/reference/timings.html),
[Cargo issue #7405](https://github.com/rust-lang/cargo/issues/7405)).

The report has two tables and two graphs
([Cargo timings](https://doc.rust-lang.org/cargo/reference/timings.html)):

- Table 1: units built, maximum concurrency, build time and compiler version.
- Graph 1, the "unit" Gantt chart: one bar per compiler invocation, with lines
  showing which units are unblocked when a unit finishes — read it for the critical
  path. Codegen time is highlighted lavender; build-script (`build.rs`) units are
  orange. Not all units show when codegen starts (binary units do not).
- Graph 2, concurrency over time: red "Waiting" (no CPU slot), blue "Inactive"
  (waiting on dependencies), green "Active" (running), over a CPU-usage background.
  It does **not** show concurrency *inside* `rustc`; rustc uses Cargo's jobserver,
  mostly during codegen.
- Table 2: per-unit total time, codegen time and enabled features.

Read it by looking for slow dependencies, features that could be disabled, a crate
built twice with different versions, and a single crate that serializes many others
([Cargo timings](https://doc.rust-lang.org/cargo/reference/timings.html),
[Rust Performance Book](https://nnethercote.github.io/perf-book/compile-times.html#visualization)).

Cold versus warm: the charts list units that were *built*, so a no-op build has no
bars to interpret — take the cold report from a fresh target/gate copy and use a
warm run as the "did anything rebuild?" control. (Inference from "number of units
built" in [Cargo timings](https://doc.rust-lang.org/cargo/reference/timings.html).)
Documented limitations: the unit graph's ordering can change between runs because
units finish in different orders, and the reports are "suitable for human
consumption only"; they provide no machine-readable timing data
([Cargo timings](https://doc.rust-lang.org/cargo/reference/timings.html),
[`cargo rustc --timings`](https://doc.rust-lang.org/cargo/commands/cargo-rustc.html#option-cargo-rustc---timings)).

## 4. rustc self-profiling and lower-level attribution

The rustc development guide picks the tool by altitude: `-Z self-profile` plus
measureme for a medium/high-level query view; `perf` or `tracy` for function-level
data; `cargo build --timings` for the crate graph
([rustc-dev-guide, "Profiling the compiler"](https://rustc-dev-guide.rust-lang.org/profiling.html)).

- `-Z self-profile` makes rustc emit `*.mm_profdata` into the working directory.
  Run it on one crate with `cargo +nightly rustc -- -Z self-profile`, then
  `summarize summarize <file>.mm_profdata`
  ([rustc-dev-guide](https://rustc-dev-guide.rust-lang.org/profiling.html),
  [measureme `summarize`](https://github.com/rust-lang/measureme/blob/master/summarize/README.md)).
  `summarize diff <a> <b>` compares two profiles event-by-event
  ([rustc-dev-guide](https://rustc-dev-guide.rust-lang.org/profiling.html)).
  Other measureme tools: `flamegraph`, `stack_collapse` and `crox` (Chrome trace)
  ([measureme README](https://github.com/rust-lang/measureme)).
- The summarize table columns are Self time, % of total, Item count, Cache hits,
  Blocked time and Incremental load time, sorted by Self time
  ([rustc-dev-guide, "Reading the output"](https://rustc-dev-guide.rust-lang.org/profiling.html)).
- `-Z self-profile-events` selects what is recorded. `default` is the balanced set;
  `llvm` (passes/codegen), `query-cache-hit`, `query-keys` and `function-args` are
  off by default because they greatly increase trace size; `all` enables everything
  ([Unstable book, `self-profile-events`](https://doc.rust-lang.org/unstable-book/compiler-flags/self-profile-events.html)).
- `-Z time-passes` prints the time of each rustc pass; it has no tracking issue and
  is documented as likely internal to the compiler
  ([Unstable book, `time_passes`](https://doc.rust-lang.org/unstable-book/compiler-flags/time-passes.html)).
- Linker invocation: `rustc --print link-args` (historically `-Z print-link-args`)
  prints the full linker command without disabling `--emit`; its exact format is not
  a stable guarantee
  ([rustc print options](https://doc.rust-lang.org/rustc/command-line-arguments/print-options.html#link-args),
  [Unstable book, codegen options](https://doc.rust-lang.org/unstable-book/compiler-flags/codegen-options.html)).
- `cargo rustc -- …` passes the extra flags *only* to the final crate, not its
  dependencies; use `RUSTFLAGS` for all crates
  ([`cargo rustc`](https://doc.rust-lang.org/cargo/commands/cargo-rustc.html)).

Which phases dominate: rustc's own performance model splits compilation into
pre-query analysis, query-based analysis, LLVM optimization/codegen and linking.
For optimized builds LLVM typically takes 65–80% of total compile time; linking is
a large share of small-change builds
([rust-lang/rust #48547, "A Model of the Compilation Process"](https://github.com/rust-lang/rust/issues/48547)).
The measureme example profile is consistent: `LLVM_emit_obj` is ~41% of self time,
followed by `LLVM_module_passes` and `LLVM_make_bitcode`
([measureme `summarize` README](https://github.com/rust-lang/measureme/blob/master/summarize/README.md)).
To attribute codegen/link cost, the profile knobs to *measure* (not verdicts) are
`-C codegen-units` (default 256 for incremental builds, 16 otherwise), LTO
(`true`/`fat`, `thin`, `false`, `off`) and debug info / `strip`
([Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html#codegen-units),
[Cargo profiles, `lto`](https://doc.rust-lang.org/cargo/reference/profiles.html#lto),
[Cargo profiles, `strip`](https://doc.rust-lang.org/cargo/reference/profiles.html#strip)).

## 5. How large build systems decide not to rebuild

Content-addressed reuse is the established alternative to mtime/absolute-path
fingerprints, and it has the same leakage caveats.

- **Nix.** Store paths are normally input-addressed (hash of the derivation);
  content-addressed paths hash the *output contents* and need no signature to be
  trusted across stores
  ([Nix `store make-content-addressed`](https://releases.nixos.org/nix/nix-2.13.5/manual/command-ref/new-cli/nix3-store-make-content-addressed.html)).
  `nix build --dry-run` prints what would be built or downloaded without doing it,
  and `nix flake check --no-build` evaluates a flake's checks without building them
  ([Nix `build`](https://nix.dev/manual/nix/2.28/command-ref/new-cli/nix3-build.html),
  [Nix `flake check`](https://nix.dev/manual/nix/2.28/command-ref/new-cli/nix3-flake-check)).
- **Bazel.** An action is defined by its inputs, output names, command line and
  environment variables. The remote cache has two parts: an action cache keyed by
  the action hash, and a content-addressable store keyed by each output blob's
  content digest
  ([Bazel remote caching](https://bazel.build/remote/caching),
  [Remote Execution API proto](https://github.com/bazelbuild/remote-apis/blob/main/build/bazel/remote/execution/v2/remote_execution.proto)).
  The action key changes if any input, argument or environment variable changes.
  Bazel's documented known issues are the same class as Cargo's: environment
  variables leaking into the key (only `--action_env`-whitelisted vars are included),
  and tools outside the workspace not being tracked, so two machines with different
  compilers can wrongly share hits. Bazel also warns that a target's stdout/stderr
  is not a good signal for estimating cache hits
  ([Bazel remote caching, "Known issues"](https://bazel.build/remote/caching)).

Both systems confirm the pattern: content addressing removes mtime ambiguity, but
absolute paths, environment and tool identity must still be controlled for reuse to
be correct — the same controls this repo's build-cache doc already applies to
sccache ([`BUILD_CACHE_AND_QUEUE.md`](BUILD_CACHE_AND_QUEUE.md)).

## What this repo should measure first, and with which exact command

Context: 12-core `minipc`, one gate copy per pushed tree, nightly pinned to
`nightly-2026-10-21` ([`rust-toolchain.toml`](../../rust-toolchain.toml)), and an
existing measurement checklist in
[`BUILD_CACHE_AND_QUEUE.md` §5](BUILD_CACHE_AND_QUEUE.md#5-measurement-checklist-before--after).
Run these inside `nix develop` locally or inside the pinned devShell on a gate copy;
do not redirect the target dir ([`AGENTS.md`](../../AGENTS.md)).

1. **Is time in rebuild decisions or in compiling?** Start with the diagnostic that
   explains a dirty unit — cheapest, no rebuild needed:

   ```sh
   CARGO_LOG=cargo::compiler::fingerprint=info cargo build -p robin_rs --bin robin
   ```

   Use `trace` instead of `info` to print the per-unit `.json` dirtiness reason
   ([Cargo FAQ](https://doc.rust-lang.org/cargo/faq.html#why-is-cargo-rebuilding-my-code),
   [Cargo `fingerprint/mod.rs`](https://github.com/rust-lang/cargo/blob/master/src/cargo/core/compiler/fingerprint/mod.rs)).
   If the reason names a changed env var or an absolute `rerun-if-changed` path,
   that is a rebuild-cause finding, not a compile-cost finding.

2. **Cold vs warm cost, and the critical path.** On a fresh gate copy (cold), then
   again in the same copy (warm control):

   ```sh
   cargo build --timings -p robin_rs --bin robin
   ```

   Open `target/cargo-timings/cargo-timing.html`; read the "units built", max
   concurrency and build time from table 1, and the longest bar plus its unblocking
   lines from the unit graph
   ([Cargo timings](https://doc.rust-lang.org/cargo/reference/timings.html)).
   This is the single most informative command: it separates "rebuilt everything"
   from "one slow crate serialized the graph".

3. **Attribute the slowest crate to a phase.** For the crate identified in step 2:

   ```sh
   cargo rustc -p robin_rs --lib -- -Zself-profile -Zself-profile-events=default,llvm
   summarize summarize *.mm_profdata
   ```

   Compare LLVM/codegen self time against query time; add `diff` against a second
   profile to isolate a change
   ([rustc-dev-guide](https://rustc-dev-guide.rust-lang.org/profiling.html),
   [Unstable book](https://doc.rust-lang.org/unstable-book/compiler-flags/self-profile-events.html)).
   The toolchain is nightly, so no channel override is needed
   ([`rust-toolchain.toml`](../../rust-toolchain.toml)).

4. **Isolate the link.** See the exact wild/clang invocation this repo configures:

   ```sh
   cargo rustc -p robin_rs --bin robin -- --print link-args
   ```

   Cross-check with the driver script at
   [`scripts/clang-wild-linker.sh`](../../scripts/clang-wild-linker.sh) and the
   `[target.x86_64-unknown-linux-gnu]` entry in
   [`.cargo/config.toml`](../../.cargo/config.toml)
   ([rustc print options](https://doc.rust-lang.org/rustc/command-line-arguments/print-options.html#link-args)).

5. **Audit the key inputs before blaming the compiler.**

   ```sh
   env | grep -E '^(RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|CARGO_INCREMENTAL|CARGO_TARGET_[A-Z0-9_]+_RUSTFLAGS)='
   ```

   A bare `RUSTFLAGS` would silently drop this repo's `-C target-cpu=x86-64-v3`
   and change every unit hash
   ([`.cargo/config.toml`](../../.cargo/config.toml),
   [Cargo environment variables](https://doc.rust-lang.org/cargo/reference/environment-variables.html)).
   Record `sccache --show-stats` at the same point — that baseline and its ownership
   stay in [`BUILD_CACHE_AND_QUEUE.md` §5](BUILD_CACHE_AND_QUEUE.md#5-measurement-checklist-before--after).

Report effect, not exit code: the `.html` report path, the top self-time rows and
the `sccache` hit-rate delta are the evidence, per the estate's verification rule.
