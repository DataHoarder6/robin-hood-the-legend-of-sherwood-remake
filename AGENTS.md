# Rust port — agent notes

## Rust port notes

- Never defend against missing data by inventing a value: if an answer depends
  on an object that is not there, return an error, panic, or at least log a
  warning.
- Leave a `TODO` where something is unclear, incomplete or suboptimal to clean
  up later.
- Prefer battle-tested crates over hand-rolled implementations; add the
  dependency.

## Worktrees and concurrency

- Create worktrees in `.worktrees/<name>` in the main repo, named like the
  branch. Work entirely inside the worktree: it is a full copy of the repo, and
  other agents share the main checkout and its `target/`.
- **NEVER redirect Cargo's target directory** — no `CARGO_TARGET_DIR`, no
  `--target-dir`, no Cargo config that moves it. Artifacts belong to the
  checkout that produced them.
- **NEVER use `git stash`.** It is scoped to the repository, not the worktree,
  so one agent's stash lands on another agent's files. Commit to a scratch
  branch instead, or leave the changes in the tree.

## Building and testing

- This workspace does not compile on every host. Build and test on the build
  host: `scripts/remote-check.sh SUITE` copies this worktree there, runs the
  suite inside the pinned devShell, and holds a per-copy lock so concurrent
  agents queue instead of clobbering. `--list` and `--prune` manage the copies.
- `nix develop` gives the same shell locally (it carries clang + wild, which is
  what `.cargo/config.toml` expects; without them every link warns and uses the
  slow system linker).
- `scripts/check-quality.sh SUITE` is the only test entry point; `docs/TESTING.md`
  maps every crate to its suite. Bare `cargo test` selects only the two default
  members, which is a smoke check and never a gate.
- Run `bash scripts/check-quality.sh format` before handing work over. The
  `.githooks/pre-commit` hook formats the files a commit touches with the
  toolchain pinned in `rust-toolchain.toml`.
- Clippy belongs in batched cleanup sessions, never inside feature work.
- Run cargo unfiltered: no `| head`, `| grep`, and no redirect to a shared log
  path. A re-run to recover a hidden error costs more than the extra lines, and
  parallel agents clobber fixed log names.
- Build and run in separate steps: `cargo build -p robin_rs --bin robin` with no
  timeout, then `cargo run …` with one, so a running game cannot hang a build or
  a timeout cannot kill one.
- `RUST_LOG=debug` (or `robin_rs=debug`, `trace`) drives the `env_logger` /
  `tracing` output. `ROBINHOOD_DATA_DIR` points at game data — the development
  default is the main repository's `datadirs/demo_leicester_ecoste`, the rest
  are symlinks beside it. Game data is the one exception to staying inside a
  worktree.

## Running the game and its tools

- Main binary: `robin` (`crates/robin_rs/src/bin/robin.rs`).
- Client developer tools are `robin_rs` examples, built on demand:
  `cargo run -p robin_rs --example <name> -- <args>`; the set lives in
  `crates/robin_rs/examples/`.
- Offline modding tools are `robin_modding_tools` bins behind its `cli` feature:
  `cargo run -p robin_modding_tools --features cli --bin <name> -- <args>`.
- wasm/emscripten builds are served from `wasm-www/`; see `README.md` and the
  `wasm-dev` / `wasm-release` profiles in `.cargo/config.toml`.

## Replays

- Every session records to `~/.local/share/robin_hood/replays/*.rhrec.jsonl`;
  the newest file is the most recent run.
- `--replay <path>` replays a recording, which is the default way to debug a
  gameplay defect.

## Searching

- `ast-grep` for structural queries (`ast-grep run --pattern 'panic!($$$)' --lang
  rust crates/robin_rs/src/`), `rg` for text. `sed -n 'A,Bp'` on a 4000-line file
  finds one call site and hides its neighbours; prefer the pattern search.

## Committing

- Commit by pathspec, `git commit -- <files>`, in small focused commits. Never
  `git add` then a bare `git commit`: the index is shared with other agents and
  someone else's staged file rides along.
- Push your branch; `fork/main` is the integration branch.

## Repo structure

- `crates/` — the workspace; `docs/TESTING.md` lists every crate and the suite
  that gates it.
- `assets/`, `datadirs/`, `mods/` — icons and fonts, game-data symlinks, mods.
- `docs/README.md` indexes the maintained guides; `docs/slop/` keeps subsystem
  evidence docs (save format, Sherwood trading, campaign purchases).
- New player-visible features are recorded in `docs/NEW_FEATURES.md`.
