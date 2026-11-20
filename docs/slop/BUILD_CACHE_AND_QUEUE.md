# Cross-worktree build caching and build-queue safety

Why a fresh worktree misses sccache on the build host, what to configure instead,
and how to queue builds without orphans or deadlocks. Every external claim below
cites its primary source; the build host is `minipc` unless `ROBIN_BUILD_HOST`
overrides it ([`scripts/remote-check.sh`](../../scripts/remote-check.sh)).

Today: each worktree is rsync'd to `~/build/robin/<copy>` with `target` excluded,
so each copy keeps its own target dir, and the suite runs under
`flock -w 7200 <copy>.lock` plus `systemd-inhibit` ([`scripts/remote-check.sh`](../../scripts/remote-check.sh)).
sccache already runs locally; read its cache size and hit rate with
`sccache --show-stats` rather than from this file.

Last verified: 2026-11-20.

## 1. What causes the misses, and the exact configuration

**What is hashed.** For a Rust crate sccache runs `rustc --emit=dep-info` to list
source files and `rustc --print file-names` to list outputs, then reuses the
result only for the same compiler, arguments, **working directory, environment,
externs, static libraries and target spec**, while every listed source file keeps
the same contents ([sccache `docs/Rust.md`](https://github.com/mozilla/sccache/blob/main/docs/Rust.md)).
By default absolute paths must match for a hit
([sccache README, "Normalizing Paths with SCCACHE_BASEDIRS"](https://github.com/mozilla/sccache/blob/main/README.md#normalizing-paths-with-sccache_basedirs)).
A fresh copy at a new absolute path therefore misses even for byte-identical
sources. Debug profile adds a second cause: Cargo uses incremental compilation
for workspace members and path dependencies by default
([Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html#incremental)),
and incremental crates are not cached unless configured otherwise
([sccache Rust.md](https://github.com/mozilla/sccache/blob/main/docs/Rust.md)).

**Exact configuration on the build host** (one `$CARGO_HOME/config.toml` plus the
sccache server environment; machine-local settings belong there, not in the
repo's `.cargo/config.toml` — see that file's header):

- `SCCACHE_BASEDIRS` — colon-separated on Linux; **list each copy root**
  (`/home/admin/build/robin/<copy1>:/home/admin/build/robin/<copy2>:…`), not just
  the parent. The documented rule is "longest matching prefix" stripping of
  source paths, working directory, `CARGO_*` variables and `env!` values
  ([sccache Configuration.md](https://github.com/mozilla/sccache/blob/main/docs/Configuration.md#env),
  [README](https://github.com/mozilla/sccache/blob/main/README.md#normalizing-paths-with-sccache_basedirs)).
  A single basedir `/home/admin/build/robin` only strips the parent, leaving
  `<copy>` in the normalized path — that is why hits stay per-copy. Only absolute
  paths are accepted; relative ones prevent the server starting. Env takes
  precedence over the config file and some values need a server restart, so
  regenerate the list when copies are added or pruned and run
  `sccache --stop-server` (the next compile restarts it). The same list can be
  written as `basedirs = [...]` in `~/.config/sccache/config`.
- `CARGO_INCREMENTAL=0` for gate builds. With `CARGO_INCREMENTAL=1` sccache exits
  with an error unless `SCCACHE_ALLOW_INCREMENTAL` is set; `=1` leaves incremental
  crates uncached and `=cache` caches them but a hit leaves the incremental dir
  untouched, so the next edit after a hit is slower; and `CARGO_INCREMENTAL=1`
  and the profile default produce *different* keys
  ([sccache Rust.md](https://github.com/mozilla/sccache/blob/main/docs/Rust.md)).
  `CARGO_INCREMENTAL` overrides `build.incremental` / the profile
  ([Cargo config](https://doc.rust-lang.org/cargo/reference/config.html#buildincremental)).
  Zero gives one deterministic, shareable key space.
- `--remap-path-prefix=<copy root>=/robin` (repeatable, `FROM=TO`) via the build
  host's user-level `[target.x86_64-unknown-linux-gnu] rustflags`, which Cargo
  joins with the repo's `cfg(target_arch = "x86_64")` flags
  ([rustc arguments](https://doc.rust-lang.org/rustc/command-line-arguments.html#--remap-path-prefix-remap-source-paths-in-output),
  [Cargo config](https://doc.rust-lang.org/cargo/reference/config.html#buildrustflags),
  repo `.cargo/config.toml`). **Never** set a bare `RUSTFLAGS`: it fully replaces
  those config flags, dropping `-C target-cpu=x86-64-v3`. With basedirs set, a hit
  would otherwise return a crate whose debug info and panic messages name
  *another* checkout's directory
  ([README](https://github.com/mozilla/sccache/blob/main/README.md#normalizing-paths-with-sccache_basedirs)).
- `SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS=CARGO_MANIFEST_DIR,CARGO_MANIFEST_PATH`.
  This is required here: `env!("CARGO_MANIFEST_DIR")` is baked into
  `crates/robin_test_support/src/original_data.rs` and
  `crates/robin_assets/src/shipping_datadir/tests.rs` (inside `concat!` with
  `include_bytes!`). With basedirs, a hit from another copy would compile in that
  copy's manifest dir; listing these variables hashes them verbatim so only those
  crates stay per-checkout. Do **not** add `OUT_DIR` — it would stop sharing every
  crate that `include!`s generated code. A proc macro reading the variable via
  `std::env::var` is not covered by this mechanism
  ([sccache Rust.md](https://github.com/mozilla/sccache/blob/main/docs/Rust.md)).
- Keep `SCCACHE_CACHE_SIZE=20G`: the documented syntax is a number with one
  `K`/`M`/`G`/`T` suffix; `20GB` and `1.5G` are refused, and the default is 10G
  ([sccache Local.md](https://github.com/mozilla/sccache/blob/main/docs/Local.md)).
  Local storage supports only **one** sccache server at a time.
- `SCCACHE_IDLE_TIMEOUT=0` if the daemon must survive between agents' builds
  ([Configuration.md](https://github.com/mozilla/sccache/blob/main/docs/Configuration.md#env));
  otherwise the default 10-minute idle exit drops the in-memory dependency map
  ([README](https://github.com/mozilla/sccache/blob/main/README.md)).

**Caveats that remain by design.** `bin`, `dylib`, `cdylib` and `proc-macro`
crates invoke the system linker and are never cached, so the final `robin` binary
is always linked locally ([sccache README](https://github.com/mozilla/sccache/blob/main/README.md#rust)).
sccache only accepts `--emit` values `link`, `metadata`, `dep-info` and requires
`link` to be present, so a metadata-only `cargo check` (which emits `.rmeta`
([rustc arguments](https://doc.rust-lang.org/rustc/command-line-arguments.html#--emit-specifies-the-types-of-output-files-to-generate)))
is **not** a shared-cache hit; use it purely as a cheap type gate, not as a
replacement for the suites in `scripts/check-quality.sh`
([sccache Rust.md](https://github.com/mozilla/sccache/blob/main/docs/Rust.md)).
`env!` values are tracked only on Rust ≥ 1.46.

## 2. Which of the three strategies fits this repo

**Recommend (a): keep per-copy target dirs and add sccache with basedirs.**
`target` is already excluded from the rsync and the per-copy `flock` already
serialises writes to it ([`scripts/remote-check.sh`](../../scripts/remote-check.sh));
`AGENTS.md` forbids redirecting the target dir at all. sccache shares *rustc
invocations* across those dirs, which is the part that was being repeated.
Deadlock risk: none from the target dir. The lock is an fd-based advisory
`flock` released when the fd is closed or the process exits
([flock(2)](https://man7.org/linux/man-pages/man2/flock.2.html)), so a killed
agent cannot strand it; `flock` is advisory and "does not detect deadlock", so
the `-w 7200` bound is the safety net.

**(b) One shared `CARGO_TARGET_DIR` with locking fits less.** `build.target-dir`
/ `CARGO_TARGET_DIR` only name where output is placed and carry no locking
guarantee ([Cargo config](https://doc.rust-lang.org/cargo/reference/config.html#buildtarget-dir));
it also violates this repo's `AGENTS.md`. An external lock would serialise all
agents on one 12-core host, and taking it while holding a separate queue lock
(and vice versa in another agent) is the classic AB-BA deadlock; a crashed
holder is safe only if the lock itself is fd-based. It buys nothing sccache
already provides.

**(c) Hardlinking registry deps fits least.** Cargo already keeps a shared
global cache under `$CARGO_HOME` and auto-cleans it
([Cargo config, `[cache]`](https://doc.rust-lang.org/cargo/reference/config.html#cache)).
Hardlinks into a copy that rsync runs `--delete` on
([`scripts/remote-check.sh`](../../scripts/remote-check.sh)) risk edits and
unlinks propagating between copies; there is no deadlock, but silent corruption.
Registry sources are not the bottleneck — rustc invocations are.

## 3. No orphan after a killed agent: layer harness cleanup + OS TTL

**Layer 1 — harness self-cleanup.** Put the build in its own process group /
session and signal the *group*, not the child:

- Node: `spawn(..., { detached: true, stdio: 'ignore' })` makes the child the
  leader of a new process group and session (`setsid(2)`); `child.kill()` signals
  only that process, so cancel with `process.kill(-child.pid, 'SIGTERM')` and
  escalate to SIGKILL. `unref()` only stops the parent waiting — it does nothing
  for cleanup. `timeout` / `killSignal` likewise target the child
  ([Node `child_process`](https://nodejs.org/api/child_process.html#optionsdetached)).
- Python: `Popen(..., start_new_session=True)` (setsid) or `process_group=0`
  (setpgid), then `os.killpg(os.getpgid(p.pid), signal.SIGTERM)`;
  `Popen.kill()` / `terminate()` signal only the child. `communicate(timeout=…)`
  does **not** kill on timeout — on `TimeoutExpired` call `kill()` then
  `communicate()` again; `wait()` with `PIPE` can deadlock when the pipe fills
  ([Python `subprocess`](https://docs.python.org/3/library/subprocess.html#subprocess.Popen.communicate)).

**Layer 2 — OS-level TTL (the actual backstop).** Launch the build as a transient
scope with a hard runtime cap:

```sh
systemd-run --scope --unit=build-<copy> \
  -p RuntimeMaxSec=7200 -p TimeoutStopSec=30 \
  ssh minipc bash -s < run-suite.sh
```

`--scope` creates a transient `.scope` unit
([systemd-run(1)](https://man7.org/linux/man-pages/man1/systemd-run.1.html#--scope));
`RuntimeMaxSec=` is supported on scopes since v244 and terminates a scope that
has run too long
([systemd.scope(5)](https://man7.org/linux/man-pages/man5/systemd.scope.5.html#RuntimeMaxSec=));
`KillMode=` defaults to `control-group`, which kills every remaining process in
the cgroup with SIGTERM and then SIGKILL after `TimeoutStopSec=`
([systemd.kill(5)](https://man7.org/linux/man-pages/man5/systemd.kill.5.html#KillMode=),
[systemd.service(5)](https://man7.org/linux/man-pages/man5/systemd.service.5.html#TimeoutStopSec=)).
A scope has no main process — its life is bound to the existence of any process
in it — so killing the harness does **not** end the scope; the TTL is what bounds
it ([systemd.scope(5)](https://man7.org/linux/man-pages/man5/systemd.scope.5.html)).
Do **not** set `Delegate=yes`: it makes systemd refrain from managing the unit's
cgroup subtree, defeating the group-wide kill
([systemd.resource-control(5)](https://man7.org/linux/man-pages/man5/systemd.resource-control.5.html#Delegate=)).
Use a transient *service* instead of a scope when the build must survive the
calling session entirely; the service manager is then the parent process and the
same properties apply ([systemd-run(1)](https://man7.org/linux/man-pages/man1/systemd-run.1.html)).

**Layer 3 — reaper timer.** Not a third killer: systemd's TTL *is* the reaper. A
userspace timer inside the harness dies with it; keep only a cheap audit sweep
that counts orphaned `cargo`/`rustc`/`sccache` processes for the checklist below.
Without systemd, an external timer that signals the recorded process group is
the fallback, not a substitute for layer 1 (design reasoning, not a cited fact).

## 4. A build queue that cannot deadlock or starve

*This section is design reasoning over the cited primitives; no primary source
asserts a general queue discipline.*

- **One lock class, one order.** Only the per-copy fd-based `flock` exists; never
  hold it and a second lock at once. If an admission lock is ever added, acquire
  admission → copy in that order everywhere and never call another agent while
  holding either. Bounded waits (`flock -w`, `RuntimeMaxSec`) turn a hang into a
  failed job; `flock` itself does not detect deadlock
  ([flock(2)](https://man7.org/linux/man-pages/man2/flock.2.html)).
- **FIFO admission with fixed slots, not try-lock retries.** Queue by request
  timestamp and admit up to N concurrent builds (N ≤ cores / jobs); pure
  try-lock loops and priority lanes starve under load. One admission point for
  all copies keeps the ordering total.
- **Bound every wait**: lock `-w` (7200 s today), `RuntimeMaxSec` for the job,
  `TimeoutStopSec` for shutdown, and `SCCACHE_CACHE_READ_TIMEOUT_SECS` (default
  60 s) for a slow cache backend
  ([Configuration.md](https://github.com/mozilla/sccache/blob/main/docs/Configuration.md#env)).
- **Log for diagnosis**: request id, copy name, queue position, wait ms, lock
  acquire/release timestamps, PID and cgroup/scope name, sccache stats before
  and after, and exit cause (`ok` / `timeout` / `killed` / `lock-timeout`). A
  queue stuck on one copy is then visible as a single long `wait ms` row. The
  five-field gate stamp in [`scripts/land.sh`](../../scripts/land.sh) is the local
  example of a state claim that carries enough fields to diagnose a miss.
- Cancel must close the fd and signal the process group (§3); never leave a
  queued request holding a lock after its client disconnects.

## 5. Measurement checklist (before → after)

1. `sccache --show-stats` at the same point in each run; record compile requests,
   hits, misses and hit rate (baseline 42.8 %)
   ([README, "Debugging"](https://github.com/mozilla/sccache/blob/main/README.md#debugging)).
2. Wall time of one identical suite from two different copies, before and after;
   a cross-copy hit should make the second copy's time approach the first's.
3. Orphan count: `ps -eo pid,ppid,pgid,cmd | grep -E 'cargo|rustc|sccache'` before
   and after a deliberate mid-build harness kill; expect zero new PIDs with
   `PPID=1`, and the scope gone within `TimeoutStopSec`.
4. Debug-info path: force a panic or inspect the binary's strings; the source
   path must be the remapped `/robin/...`, never another copy's absolute path.
5. `env!("CARGO_MANIFEST_DIR")` behaviour: run the `robin_test_support` and
   `robin_assets` tests from two copies; neither may read the other copy's data.
6. Queue log: assert no wait exceeds the configured `-w`, every request has a
   release row, and no process survives its TTL.

See also: [`../TESTING.md`](../TESTING.md),
[`scripts/remote-check.sh`](../../scripts/remote-check.sh),
[`scripts/push-gate.sh`](../../scripts/push-gate.sh),
[`AGENTS.md`](../../AGENTS.md).

## 5. 2026-11-20 correction: tree-keyed gate copies were the bottleneck

Measured on minipc while three landings ran: sccache overall 16.56% hits, but
**Rust 0.36%** (4 hits / 1092 misses) versus C/C++ 99.46%. sccache's cache key
includes the working directory, compiler arguments, environment, externs and
target spec ([sccache Rust.md](https://github.com/mozilla/sccache/blob/main/docs/Rust.md));
a tree-keyed copy has a fresh `target/`, so each landing's rustc invocations get
new `-L dependency=...` / `--out-dir` paths and miss. `--remap-path-prefix` is
itself part of the key, so it does not reliably fix this across copies
([howardjohn, "Sharing Rust Build Cache", 2026-02](https://blog.howardjohn.info/posts/shared-rust-build/)).
Hardlinking a warm target works but needs a custom script; `mr-boxington`/`mbx`
([jdx, 2026-09](https://jdx.dev/posts/2026-09-05-introducing-mr-boxington/))
packages it but replaces `target/` with a managed symlink, which conflicts with
this repo's rule that artifacts belong to the producing checkout.

Decision: gates share one stable copy (`gate-shared`) and rely on Cargo's own
build cache ([Cargo Book, Build cache](https://doc.rust-lang.org/cargo/reference/build-cache.html)).
The copy lock prevents tree swaps; `rsync --checksum --no-times` keeps unchanged
mtimes and only changed crates' artifacts are purged. sccache stays enabled for
the cases where it does hit (C/C++, identical native invocations).

The stable gate copy removes the cold rebuild, so incremental is not needed for
speed. Gate builds keep `CARGO_INCREMENTAL=0`: a trial with
`SCCACHE_ALLOW_INCREMENTAL=1` broke a real landing when sccache rejected
clippy's `rustc -vV` probe with "incremental compilation is prohibited". See
[ADR 0007](../adr/0007-no-cargo-incremental-in-gates.md) before changing
either variable; a version probe is not proof, only a full gate is.
