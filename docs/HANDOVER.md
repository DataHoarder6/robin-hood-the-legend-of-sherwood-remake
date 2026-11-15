# Handover — co-op setup, the two blockers, and how to run a session

Written 2026-11-15, after a session that produced two reproducible defects.
Read this first; it assumes nothing.

## 1. The goal, concretely

Two players, one Linux laptop (player A) and one Windows 11 PC (player B,
administrator account `lukas`, no Rust toolchain), playing the co-op build of
this remake. Sessions are about one hour a week, so **no debugging can happen
inside the session**: every run must leave logs good enough to fix things
afterwards.

## 2. Topology

- Player B is behind CGNAT. Nothing inbound is exposed on his side, ever.
- He runs an outbound reverse tunnel to a public Linux host: the
  `brother-tunnel` account there has shell `/bin/nologin`, no PTY, no agent,
  no X11, and `authorized_keys` restricted with `permitlisten="localhost:2222"`.
  The listener binds `127.0.0.1:2222` only.
- From player A's laptop:
  `ssh -J <jump> -p 2222 lukas@localhost` reaches his Windows sshd. The tunnel
  exists for installation and diagnostics, **never** for gameplay: the game's
  multiplayer is iroh/QUIC and cannot use it.
- In-game: **player B hosts** (seat 0 owns the input order; his uplink is the
  better one). Player A joins from the multiplayer menu; matchmaking is
  serverless (iroh-gossip over the Mainline DHT) and nothing is configured by
  hand.

## 3. Where things live

| What | Where |
|---|---|
| Fork (source of truth) | `github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake`, branch `main` |
| Player B build tree | `C:\Games\rh-src` (git clone, `target\release\robin.exe`) |
| Player B game data | `C:\Games\robin-hood\game` (`ROBINHOOD_DATA_DIR`, set per-user) |
| Player B logs | `%LOCALAPPDATA%\robin_hood\logs\last.log` (+ `prev.log`) |
| Player B desync reports | `%LOCALAPPDATA%\robin_hood\reports\report-*` |
| Player A build tree | `~/Games/robin-hood/src` (clone of the fork) |
| Player A logs | `~/.local/share/robin_hood/logs/session-<timestamp>.log` |
| Player A launcher | `~/Games/robin-hood/run-fork.sh` (`robin-hood` on PATH) |
| Player A log bundle | `robin-logs` → tarball on the Desktop |

## 4. Defect 1 — the released Windows build never starts (solved)

Symptom: the CI `windows-gnu` artifact exits instantly, even for `--version`;
Windows logs `0xc0000005` at fault offset `0x3d8d376`.

Cause: MinGW `ld` records Windows TLS `SECREL32` offsets as 32-bit base
relocations (`IMAGE_REL_BASED_HIGHLOW`) inside `.text`, and the image is linked
`DYNAMIC_BASE`, so the loader adds the image delta to every TLS offset. The
first TLS read executes before `main`. Measured on that binary: 5082 `.text`
relocations, 4898 of 4898 `HIGHLOW` ones holding a value below the `0xB0`
TLS block size.

Fixes: build with MSVC (what we do now), or link the gnu build with
`-C link-arg=-Wl,--disable-dynamicbase`, or use binutils >= 2.44. A shipped exe
can be patched by clearing `DYNAMIC_BASE`/`HIGH_ENTROPY_VA` in the PE optional
header (costs ASLR, and any Velopack update overwrites the file).

Written up in `docs/WINDOWS.md` in this fork.

## 5. Defect 2 — every session died at the first win or loss (fixed)

Symptom: after a mission ends, the host shows "waiting for player", the client
says "reconnected" and freezes; menus stop responding. The client queues a
report whose `description` reads:

    Mission launch failed: multiplayer protocol failure: fatal multiplayer
    session error: reconnect joined incompatible seat PlayerId(1) mission
    `S01_Not_VL` seed 14036589514977379805 ...; expected seat PlayerId(1)
    mission `S01_Not_VL` seed 0

Cause: `crates/robin_rs/src/multiplayer/client_session.rs` (the reconnect loop)
compares the client's **join-time** Welcome against the **reconnect** Welcome
with exact equality over six fields. Mission id, seed, `SimConfig` and speech
locale are host-authored and the host *rewrites all four when a mission
starts*; a reconnect that crosses that boundary therefore always differs.
`validate_reconnect_state` then returns a fatal handshake error, the client
publishes `NetEvent::Fatal`, and the host drops the session.

Fix (branch `fix/reconnect-context`): identity is `seat` + `session_id` only —
the two fields a client cannot legitimately change. The host-authored context
is adopted with an `info` log line instead of being enforced. The regression
test is `reconnect_rejects_identity_change_and_adopts_host_context`: it now
refuses a changed seat or session and accepts the four context fields.

Status: **fixed and merged** to the fork's `main` as `b471b1a12`. Both tests
that pinned the old rule were rewritten (`reconnect_` suite passes locally and
in CI), and CI now builds the Windows artifact from that commit.

## 6. Logging (the point of the exercise)

Both launchers set `RUST_LOG=info,robin_rs=debug,robin_engine=debug,iroh=info`
and capture everything to a file. Without that, a session leaves nothing
behind — which is exactly how the first sessions went.

- Player A: `robin-hood` starts the game and writes a per-session log;
  `robin-logs` produces a tarball with logs plus queued desync reports.
- Player B: `C:\Games\rh-src\run-robin.cmd` starts the game, rotates
  `last.log` → `prev.log`, and appends a start/end banner;
  `C:\Games\rh-src\collect-logs.cmd` writes `Desktop\robin-logs.zip`.
  **Neither is installed yet** — his machine was off at the end of the
  session. Install on next connect, before the next session.

Keep two sessions of logs: the interesting failure is often reported one run
later.

## 7. Next steps, in order

1. Connect to player B's machine, install `run-robin.cmd` and
   `collect-logs.cmd`, and repoint the desktop shortcut at the `.cmd`.
2. Install the current CI build on both machines — **the same commit**, since
   multiplayer binds the engine commit and protocol version:
   - B: `gh run download --repo DataHoarder6/… --name robin-windows-x86_64-exe`,
     copy `robin.exe` over the existing one;
   - A: either the Linux AppImage from the same release, or a local build of
     the same commit (`~/Games/robin-hood/src`).
3. Play. Afterwards `robin-logs` on A, `collect-logs.cmd` on B; send both.
4. File what the logs show as a new entry in this document, then fix on the
   mini PC and let CI produce the next artifact.

Nothing about a session should require compiling on either player's machine.

## 8. Where the work happens, and how builds reach the players

- **Development: the mini PC.** `~/git/robin-hood-remake` plus a worktree per
  change under `.worktrees/`. It has no toolchain in the system profile; use
  `nix-shell -p rustup …` (the pinned `nightly-2026-10-21` is installed under
  `~/.rustup`) or let CI do the compiling. It also holds the GitHub CLI login
  used to read CI.
- **Builds: GitHub Actions on the fork**, both platforms, `windows-latest` for
  Windows (MSVC) and `ubuntu-22.04` for Linux. The Windows job exists because
  cross-building from Linux with MinGW produced defect 1.
- **Laptop and B's PC: end devices.** They run artifacts, not `cargo`. The
  laptop keeps a source tree only for the data directory and the launcher.
- Branch discipline: hotfixes land on `main` so the play build stays current;
  gameplay features go to their own branch and merge when they are ready to be
  played.

## 9. Gotchas that cost time (do not re-learn these)

- **cargo must run with cwd inside the tree.** rustup applies
  `rust-toolchain.toml` per directory; `--manifest-path` from elsewhere silently
  uses stable cargo, which rejects the repo's `codegen-backend` setting.
- **ATL is required on Windows.** `LNK1104: cannot open file 'atls.lib'` at the
  final link; the `VCTools` workload does not ship it. Add
  `Microsoft.VisualStudio.Component.VC.ATL`.
- **`vs_installer.exe`**: rejects `--wait` and an unquoted `--installPath`
  (exit 87). Drive it through `cmd /c` with one fully quoted command line.
  In PowerShell, `$args` is reserved — it silently mangles an argument array.
- **Long builds over Windows SSH die with the session.** Register a scheduled
  task (`-LogonType S4U -RunLevel Highest`) and start that.
- **A running exe blocks its own rebuild** on Windows; close the game first.
- **`ROBIN_PACKAGE_VERSION` is `option_env!`** — changing it recompiles the
  whole `robin_rs` crate (~18 min on the i5-8300H). Do not stamp casually.
- **`version_label()` vs `--version`**: the menu shows `v<ROBIN_PACKAGE_VERSION>`,
  the CLI prints `CARGO_PKG_VERSION`. They can disagree.
- **Linux/NixOS**: use `nix-shell -p …` (only it populates `PKG_CONFIG_PATH`;
  `libdbus-sys` panics otherwise), and run the binary through
  `nix run --impure nixpkgs#steam-run` (winit `dlopen`s libwayland).
- **The laptop's login shell is fish**, so send bash scripts over stdin:
  `ssh laptop 'cat > /tmp/x.sh' < x.sh && ssh laptop 'bash /tmp/x.sh'`.
- **`pkill -f <pattern>` matches its own command line** — the remote shell dies
  and the command returns nothing. Use `[t]arget/...` or `pkill -x`.
- **The fork moves without you.** `main` gained commits twice in one evening.
  A commit mismatch silently refuses the join, so pull and rebuild both sides
  together.
- Do not run two instances on one machine expecting test co-op: multiplayer
  identity is per install (`multiplayer_identity.key`).
