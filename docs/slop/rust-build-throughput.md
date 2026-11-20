# Rust build throughput with 4–5 concurrent agents

Read-only research, 2026-11-20. All claims cite repo files (with the worktree
that holds them), git history, the live build host, or official docs. No builds
were run and no file other than this one was changed.

## Version note — three different "current" states exist

This matters before any number or rule is quoted:

- The **main checkout** `~/git/robin-hood-remake` and its rewrite
  `~/git/robin-hood-remake-public` carry an older 113-line
  `scripts/remote-check.sh`: default host `minipc` (`~/git/robin-hood-remake/scripts/remote-check.sh:19`),
  `rsync -a --delete --exclude target --exclude .worktrees` (`:99-104`), and
  `flock -w 7200 … nix develop --command bash scripts/check-quality.sh` (`:110-112`).
  They have no `docs/PARALLEL_AGENTS.md` and their `docs/TESTING.md` has no
  build-throughput paragraph (verified by `ls`/`rg`; the two `TESTING.md` files
  have identical md5).
- **Integration (`fork/main`) and every agent worktree** carry the newer
  machinery quoted below: per-copy `rsync` with checksum + fingerprint purge,
  build slots, `SCCACHE_BASEDIRS=$(pwd)`. Verified with
  `git show fork/main:scripts/remote-check.sh` and
  `.worktrees/reinforcements/scripts/remote-check.sh`.
- **Unmerged branches** (`cache-reuse`, `build/gate-speedup`) add cross-copy
  sccache sharing, path remapping, tree-keyed gate copies and transient scopes.
  They are *not* on `fork/main` (verified with `git branch -a --contains` on
  `c0da2fc3c`, `67d812756`, `879adde8f`, `964fc931e`, `3552d38a2`, `10e5dab20`).

## What exists today

### Per-worktree copies, not shared build trees

- The remote copy's name is the worktree directory name:
  `copy_name()` defaults to `basename "$repository"`
  (`.worktrees/reinforcements/scripts/remote-check.sh:43-51`); copies live under
  `~/build/robin/<name>` (`:34`, `:78`). The stated reason is that a second
  agent's rsync "would swap the source out from under a running compile"
  (`:4-8`).
- The sync is
  `rsync -a --no-times --checksum --delete --itemize-changes`
  with `--exclude target --exclude .worktrees --exclude .git`
  (`:90-110`). `--no-times` makes a changed file arrive with a *new* mtime and
  `--checksum` keeps unchanged files at their old mtime (`:90-93`).
- Changed crates have their build state purged:
  `rm -rf "target/debug/build/\$crate" target/debug/.fingerprint/"\$crate"-*`
  (`:111-119`). The comment explains why: Cargo judges freshness by mtime, and
  a merge checkout can leave stale rmeta looking fresh — "measured 2026-11-18 …
  on three gates in a row" (`:95-102`).
- The copy gets a real git repository (objects pushed with
  `git … push … HEAD:refs/robin/check`, then `git read-tree HEAD`) so the
  build's `ROBIN_GIT_HASH` is valid and `nix develop` can hash the git flake
  (`:121-151`).
- The repo rule forbids the obvious shortcut: "NEVER redirect Cargo's target
  directory — no `CARGO_TARGET_DIR`, no `--target-dir`, no Cargo config that
  moves it. Artifacts belong to the checkout that produced them"
  (`.worktrees/reinforcements/AGENTS.md:16-20`).

### Serialisation and core allocation

- Each copy has its own fd-based `flock -w 7200 $lock` under
  `systemd-inhibit`; two agents on the *same* copy queue (`:20-21`, `:258`,
  `:267-268`, `:278-279`).
- Each gate also takes one of `ROBIN_BUILD_SLOTS` (default **3**) slots and runs
  `bash scripts/build-slot.sh $slot_dir $slots` before `nix develop` (`:22-24`,
  `:35-36`, `:270-271`, `:281-282`). `build-slot.sh` holds the slot for the
  process lifetime, counts held slots, sets
  `jobs=$(( cores / held ))`, clamps to ≥1 and exports `CARGO_BUILD_JOBS`
  (`.worktrees/reinforcements/scripts/build-slot.sh:7-18`, `:36`, `:64-71`).
  The stated failure it prevents: "twelve cores run twenty-four compilers and
  both builds finish later than either would have alone" (`:7-11`).
- The unmerged `build/gate-speedup` branch adds FIFO admission with
  `BUILD_SLOT_WAIT_SECS` (default 7200) and stale-ticket reaping
  (`.worktrees/gate-speedup/scripts/build-slot.sh:12-22`); its guidance is
  "Keep **one gate in flight** across agents: queue behind the other agent's
  `runbg` unit instead of starting a competing gate"
  (`.worktrees/gate-speedup/docs/PARALLEL_AGENTS.md:84-99`).

### The pinned shell

- Each gate invokes `nix develop --command …` fresh
  (`.worktrees/reinforcements/scripts/remote-check.sh:271`, `:282`); there is no
  persistent devShell or profile in the repo. The flake pins `nixpkgs` to
  `github:NixOS/nixpkgs/nixos-unstable` (`flake.nix:4`) and builds a `mkShell`
  with git, rustup, clang, wild, gcc and the runtime libraries (`flake.nix:41-69`).
  The toolchain is `nightly-2026-10-21` with cranelift, rust-src, rustfmt and
  the wasm target (`rust-toolchain.toml:1-10`). The repo's own `.cargo/config.toml`
  deliberately excludes machine-local settings: "Machine-local settings like
  `rustc-wrapper = sccache` or `jobs = N` belong in your user-level
  ~/.cargo/config.toml" (`.cargo/config.toml:1-7`); it supplies the x86-64
  `target-cpu=x86-64-v3` flag and the clang+wild linker (`:16-23`).
- No measurement of flake-evaluation cost exists in the repo or history
  (searched `git log --all` for build/cache timing commits); do not assume it
  is a bottleneck.

### sccache: installed on the build host, but not effectively shared on the integration path

- Machine config `services.build-host` is enabled **only on minipc**
  (`~/nixos-config/hosts/minipc.nix:23`, `:26-29`). Its module installs
  `pkgs.sccache` and writes the build user's `~/.cargo/config.toml` with
  `[build] rustc-wrapper = "sccache"` and `SCCACHE_CACHE_SIZE = "20G"`
  (`~/nixos-config/modules/system/build-host.nix:45-61`). The module explicitly
  does **not** set `SCCACHE_BASEDIRS`: "it must name the copy being built, and
  only the caller knows that. `scripts/remote-check.sh` exports it per copy"
  (`:24-26`). Its own comment records the pre-cache measurement: 28 copies,
  99.5 GB total, 77 GB of `target/`, "and no compiler cache" (`:11-16`).
- What the integration/worktree script exports is exactly
  `env SCCACHE_BASEDIRS=\$(pwd)` — *one copy root*
  (`.worktrees/reinforcements/scripts/remote-check.sh:269`, `:280`). A stale
  or wrong basedir silently disables sharing; the repo documented that failure
  in `48f8ee879` (2026-11-19): "A base that does not exist is not an error to
  sccache: every path stays absolute, every copy gets its own key."
- The build host's devShell **does** use sccache: live check on 2026-11-20,
  `cd ~/build/robin/reinforcements && nix develop --command bash -c 'command -v sccache'`
  returned `/etc/profiles/per-user/admin/bin/sccache`; the wrapper comes from
  home-manager `home.packages`, not from the repo's `flake.nix` (which has no
  sccache entry, `flake.nix:42-67`). Live `rustc` processes were prefixed
  `sccache …`. The runtime host (`oci-vm`) has no sccache binary and no
  `~/.cargo/config.toml` — builds are not run there by design
  (`remote-check.sh:4-8`).
- Live cache state on minipc (2026-11-20, `sccache --show-stats`): local disk
  `/home/admin/.cache/sccache`, 17 GiB of 20 GiB; 2833 compile requests,
  312 hits (13.73% overall), **5 Rust hits vs 1959 Rust misses (0.25%)**,
  277 C/C++ hits (99.64%). Two cargo and five rustc processes and all three
  slot files were active at probe time. The measured "no cross-copy Rust reuse"
  is therefore still true on the deployed path, even though sccache is running.
- The *unmerged* fix is fully specified: build the basedir list from every live
  copy directory, restart the daemon when the set changes, then export
  `SCCACHE_BASEDIRS=<all roots>`, `CARGO_INCREMENTAL=0`,
  `SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS=CARGO_MANIFEST_DIR,CARGO_MANIFEST_PATH`,
  `SCCACHE_IDLE_TIMEOUT=0` and per-root
  `--remap-path-prefix=<copy root>=/robin` in the target-specific rustflags
  (`.worktrees/cache-reuse/scripts/remote-check.sh:274-300`, `:311-339`;
  `.worktrees/gate-speedup/scripts/sccache-basedirs.sh:1-33`). A live daemon on
  minipc already carries that multi-root list (its `/proc/<pid>/environ`), so
  an experiment is running — but no before/after wall-time result is recorded.
- Cargo's own registry cache is shared globally under `$CARGO_HOME` and is not
  the bottleneck
  (`.worktrees/cache-reuse/docs/slop/BUILD_CACHE_AND_QUEUE.md:120-126`).

## Measurement after the multi-basedir change (2026-11-20)

`scripts/test_build_host.sh` passes 9/9 fixture tests, including the basedir
list, per-root remap flags and the absence of literal quotes in the
`SCCACHE_BASEDIRS` value. An `engine` suite from a fresh `build-cache` copy ran
green on the build host with the new script.

`sccache --show-stats` on minipc, before/after that single run:

| | Rust hits | Rust misses | Rust hit rate |
|---|---|---|---|
| before | 6 | 3257 | 0.18% |
| after | 461 | 1076 | 29.99% |

This is cumulative daemon state, not a controlled A/B: the daemon's basedir set
was already multi-root during the experiment, and other agents compiled in the
same window. It does establish that cross-copy Rust hits now occur; the old
single-basedir path produced essentially none. A controlled before/after on two
identical copies is still owed.

## What was measured and rejected

- **Cold copy vs warm copy.** "about 5 minutes of dependencies on the build
  host, against 3-30 s for a rebuild in a warm one (12 cores, 2026-11-17)";
  the prescription is "Keep one copy per branch and reuse it"
  (`.worktrees/reinforcements/docs/TESTING.md:97-101`). Same numbers in commit
  `3c2b5aeb2` ("docs(tooling): the cold copy is the build cost, not the compiler").
- **sccache, 310 s → 222 s** for a from-scratch rebuild, measured as a
  *machine-local `rustc-wrapper`* (`.worktrees/reinforcements/docs/TESTING.md:102-103`;
  commit `3c2b5aeb2`). This is a warm-cache-within-one-copy result, not a
  cross-copy-hit result: the commit says the wrapper took the cold rebuild to
  222 s; the deployed single-basedir config is why it did not go further.
- **Shared `CARGO_TARGET_DIR`:** "an out-of-tree shared `CARGO_TARGET_DIR`
  reused nothing across copies (511 crates recompiled)" and "A compiler cache
  is not the answer" (`.worktrees/reinforcements/docs/TESTING.md:102-105`).
  This decision also appears on `fork/main` (verified via
  `git show fork/main:docs/TESTING.md`), so any shared-target proposal
  contradicts a documented, measured decision *and* `AGENTS.md:16-20`.
- **Other recorded numbers, not throughput measurements:** `target/` 13.1 GiB
  with 8.4 GiB incremental (runtime, 2026-11-17;
  `.worktrees/reinforcements/docs/PARALLEL_AGENTS.md:140-160`); 28 copies /
  99.5 GB / 77 GB target (minipc, 2026-11-19; `build-host.nix:11-16`); sccache
  hit-rate "baseline 42.8 %" in the measurement checklist
  (`.worktrees/cache-reuse/docs/slop/BUILD_CACHE_AND_QUEUE.md:208-212`);
  live minipc disk 112 GiB free of 458 GiB and cache 17/20 GiB (live probe).
  No doc or commit records a measured gain from build slots, tree-keyed copies,
  persistent shells, or a second build host.

## Why concurrent agents cannot reuse each other's work

1. **Artifacts are per-copy by construction.** rsync excludes `target`
   (`remote-check.sh:103-106`); the copy name is the worktree name (`:43-51`).
   Cargo's fingerprints and rmeta in one copy are never visible to another;
   `AGENTS.md:16-20` forbids moving them.
2. **sccache keys differ per copy on the deployed path.** sccache by default
   requires absolute paths to match; `SCCACHE_BASEDIRS` strips a prefix before
   hashing and needs *each copy root*, because a single parent leaves `<copy>`
   in the key (sccache README, "Normalizing Paths with SCCACHE_BASEDIRS":
   https://github.com/mozilla/sccache/blob/main/README.md#normalizing-paths-with-sccache_basedirs).
   `SCCACHE_BASEDIRS=$(pwd)` therefore gives each agent a distinct key space;
   the live 0.25% Rust hit rate is the consequence. The unmerged branches fix
   this by listing all roots and remapping paths.
3. **The crate under edit is the one sccache cannot cache.** Incremental
   compilation is on by default for workspace members and path dependencies in
   dev/test profiles (Cargo Profiles:
   https://doc.rust-lang.org/cargo/reference/profiles.html#incremental), and
   incrementally compiled crates are not cached by default
   (sccache Rust.md: https://github.com/mozilla/sccache/blob/main/docs/Rust.md).
   Binaries, dylibs, cdylibs and proc macros also invoke the linker and are
   never cached (same source). So the unshared tail is exactly the agents'
   changed crates plus final links.
4. **Staleness is handled destructively, not by sharing.** The mtime-based
   `.fingerprint`/`build` purge (`remote-check.sh:90-119`) is local to a copy;
   it exists because a stale rmeta can be linked instead of recompiled. No
   equivalent cross-copy invalidation exists.
5. **Admission is deliberately serialised:** per-copy `flock` plus three
   global slots (`remote-check.sh:20-24`, `:267-282`; `build-slot.sh:64-71`).
   They avoid clobbering and core oversubscription, not duplicate compilation.

## External primary sources (short)

- Cargo stores builds in `target`/`build` directories and names sccache as the
  third-party way to "share built dependencies across different workspaces";
  `CARGO_TARGET_DIR` only *moves* output (Cargo Build Cache:
  https://doc.rust-lang.org/cargo/reference/build-cache.html). A shared target
  dir therefore carries no sharing guarantee — the repo measured 511 recompiles.
- sccache caches by compiler, arguments, working directory, environment,
  externs, static libs, target spec and source contents; it supports local disk
  or cloud backends, defaults to local disk, and local storage supports only
  one server at a time (Rust.md above; Local.md:
  https://github.com/mozilla/sccache/blob/main/docs/Local.md; README:
  https://github.com/mozilla/sccache/blob/main/README.md). Sharing across hosts
  would need a remote backend, not a bigger local dir.
- Debug info and `env!` paths from a cross-copy hit name the *source* checkout
  unless paths are remapped and `SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS` is set
  (sccache README, "Normalizing Paths with SCCACHE_BASEDIRS"; Rust.md).

## What is worth adding — options and tradeoffs

| Option | Repo evidence / implication | Verdict |
| --- | --- | --- |
| **(a) Tune slots / cores** | 3 slots × `nproc/held` jobs already exist (`remote-check.sh:35`, `build-slot.sh:64-71`); `build/gate-speedup` adds FIFO + bounded waits (unmerged). No timing measurement exists. | Keep 3 as the safe default; land FIFO/one-gate-in-flight for 4–5 agents. Tradeoff: more queuing latency, less oversubscription. Do not change slot count without a before/after wall-time record. |
| **(b) Shared sccache local cache for all copies** | Already *installed* (`build-host.nix:45-61`), already on the devShell PATH, but deployed keys are per-copy (`remote-check.sh:269`, `:280`) and live Rust hit rate is 0.25%. The 310→222 s measurement never exercised true cross-copy hits. | **Highest value.** Land the `cache-reuse` / `gate-speedup` basedir+remap+`CARGO_INCREMENTAL=0` work on `fork/main`, keep one server, then measure per the checklist (`BUILD_CACHE_AND_QUEUE.md:208-223`). This contradicts the old "compiler cache is not the answer" conclusion only for cold copies; the warm-copy rule stands. |
| **(c) Persistent nix devShell/profile** | `nix develop` runs once per gate (`remote-check.sh:271`, `:282`); no eval timing recorded; flake is small (`flake.nix:1-73`). | Not worth doing on evidence. Measure first; this would add state that must be kept in sync with the pinned toolchain and flake input. |
| **(d) Pre-warm a base `target/` per branch** | Already the documented practice: one copy per branch, `--sync` into it, 3–30 s warm vs ~5 min cold (`TESTING.md:97-105`). Unmerged tree-keyed gate copies with a 7-day TTL (`gate-speedup/.../PARALLEL_AGENTS.md:89-95`) generalise it. | Extend, don't reinvent: land tree-keyed gate copies only with a disk budget (77 GB of target across 28 copies; 112 GiB free). Do not share a target dir (measured failure + `AGENTS.md`). |
| **(e) Reduce suite scope** | The gates already select single named tests (`check-quality.sh:25-28`, `:72-78`); TESTING.md says "Select one ignored test at a time … Do not select every ignored test" (`:104-117`) and "iterate on one test in the gate's own copy, then run the whole suite once" (`:84-95`). | Keep for iteration; never replace the named gate — `test_quality_suites.py` enforces every member has a gate (`TESTING.md:45-50`). Small wins, no architectural risk. |
| **(f) More build hosts / distribute** | One configured cache host (`hosts/minipc.nix:23`, `:29`); builds never run on the runtime or laptop (`remote-check.sh:4-8`); local sccache disk allows one server per host; cross-host sharing needs a remote backend (sccache docs above). | Not justified by any measurement. Adding a second host duplicates the 20 GiB cache and the warm-copy problem; revisit only if (b) is landed and the queue is still the measured bottleneck. |

## Recommendation

1. Land the *narrow* cross-copy sccache change from `cache-reuse` on
   `fork/main`: multi-root `SCCACHE_BASEDIRS`, per-root remap,
   `CARGO_INCREMENTAL=0`, verbatim env deps, daemon restart on basedir-set
   change. Keep local-disk storage and a single server. Then record
   `sccache --show-stats` and one identical suite from two copies before/after;
   today's baseline is a 0.25% Rust hit rate and 5 minutes for a cold copy.
2. Keep per-worktree copies, warm target dirs, the changed-crate fingerprint
   purge, per-copy flock and the three build slots. Do **not** introduce a
   shared `CARGO_TARGET_DIR` or redirect target dirs.
3. For 4–5 agents, add the FIFO/one-gate-in-flight discipline from
   `build/gate-speedup` so land gates stop competing; leave slot count at 3
   until a measurement says otherwise.
4. Update `docs/TESTING.md` with the new result: the 2026-11-17 paragraph
   "A compiler cache is not the answer" (`:102-105`) is a cold-copy conclusion
   from a single-basedir wrapper, not evidence against a correctly basedired
   shared cache.
