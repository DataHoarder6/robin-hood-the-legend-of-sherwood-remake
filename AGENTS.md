# Rust port — agent notes

## Ground rules

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
  maps every crate to its suite (`format` is the whole-tree formatting gate).
  Bare `cargo test` selects only the two default members, never a gate. The
  `.githooks/pre-commit` hook formats the files a commit touches.
- Clippy belongs in batched cleanup sessions, never inside feature work.
- Run cargo unfiltered — no `| head`, no shared log path. Recovering a hidden
  error costs more than the extra lines, and parallel agents clobber fixed
  log names.
- Build and run in separate steps: `cargo build -p robin_rs --bin robin` with no
  timeout, then `cargo run …` with one, so a running game cannot hang a build or
  a timeout cannot kill one.
- `RUST_LOG=debug` (or `robin_rs=debug`, `trace`) drives the `env_logger` /
  `tracing` output.

## Running the game and its tools

- `README.md` has the binary, the dev game-data directory (`ROBINHOOD_DATA_DIR`)
  and the wasm workflow. Developer tools are `robin_rs` examples, built on
  demand with `cargo run -p robin_rs --example <name>`.

## Replays

- Every session records to `~/.local/share/robin_hood/replays/*.rhrec.jsonl`;
  `--replay <path>` replays one, which is the default way to debug a gameplay
  defect.

## Investigating

- A read-only question that took real digging ("what exists for X", "which
  natives can do Y") ends by writing its answer to `docs/slop/<topic>.md` and
  linking the files it names. An answer that lives only in a reply is paid for
  again by the next session.

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
  evidence docs (natives, save/state constraints, trading, campaign purchases).
- New player-visible features are recorded in `docs/NEW_FEATURES.md`.
