# Rust port — agent notes

## Ground rules

- Never defend against missing data by inventing a value: if an answer depends
  on an object that is not there, return an error, panic, or log a warning.
- Leave a `TODO` where something is unclear or suboptimal, and prefer battle-tested
  crates over hand-rolled ones; add the dependency.

## Worktrees and concurrency

- The shared checkout's `main` is a read mirror: never commit, merge or switch
  it. Work in `.worktrees/<name>` off `fork/main`; `just doctor` and
  `docs/PARALLEL_AGENTS.md` carry the rules and the recoveries.
- **NEVER redirect Cargo's target directory** — no `CARGO_TARGET_DIR`, no
  `--target-dir`, no Cargo config that moves it. Artifacts belong to the
  checkout that produced them.
- **NEVER use `git stash`.** It is scoped to the repository, not the worktree,
  so one agent's stash lands on another agent's files. Commit to a scratch
  branch instead, or leave the changes in the tree.

## Building and testing

- This workspace does not compile on every host: `scripts/remote-check.sh SUITE`
  copies this worktree to the build host, wakes `build-host`, runs the suite in the
  pinned devShell under a per-copy lock, and prunes copies with `--prune`.
- `nix develop` gives the same shell locally, with the clang + wild that
  `.cargo/config.toml` expects; without them links are slow and warn.
- `scripts/check-quality.sh SUITE` is the only test entry point; `docs/TESTING.md`
  maps every crate to its suite (`format` is the whole-tree rustfmt gate). Bare
  `cargo test` selects only the two default members, never a gate. The
  `.githooks/pre-commit` hook formats the files a commit touches.
- Clippy belongs in batched cleanup sessions, never inside feature work.
- Run cargo unfiltered — no `| head`, no shared log path; a hidden error costs
  more than the extra lines, and parallel agents clobber fixed log names.
- Build and run in separate steps — `cargo build -p robin_rs --bin robin` without
  a timeout, then `cargo run …` with one — so neither can kill the other.
- `RUST_LOG=debug` (or `robin_rs=debug`, `trace`) drives the `env_logger` /
  `tracing` output.

## Running the game and its tools

- `README.md` has the binary, the dev game-data directory (`ROBINHOOD_DATA_DIR`)
  and the wasm workflow. Developer tools are `robin_rs` examples: `cargo run -p
  robin_rs --example <name>`.
- `docs/PLAY.md` maps playing: stable tag versus dev worktree, which machine
  runs which, the build-host wake path, the laptop store-path copy, the Windows
  tunnel, and the version check before a session.

## Replays

- Every session records to `~/.local/share/robin_hood/replays/*.rhrec.jsonl`;
  `--replay <path>` replays one, the default way to debug a gameplay defect.

## Investigating

- A read-only question that took real digging ends by writing its answer to
  `docs/slop/<topic>.md` and linking the files it names; an answer only in a
  reply is paid for again by the next session.

## Searching

- `ast-grep` for structural queries (`ast-grep run --pattern 'panic!($$$)' --lang
  rust crates/robin_rs/src/`), `rg` for text. `sed -n 'A,Bp'` on a 4000-line file
  finds one call site and hides its neighbours; prefer the pattern search.

## Committing

- Commit by pathspec, `git commit -- <files>`, in small focused commits — never
  `git add` plus a bare `git commit`, which would carry a co-worker's staged file.
- Push your branch; `fork/main` is the integration branch.

## Repo structure

- `crates/` — the workspace, gated by `docs/TESTING.md`; `assets/`, `datadirs/`,
  `mods/` — icons and fonts, game-data symlinks, mods.
- `docs/README.md` indexes the maintained guides; `docs/slop/` keeps subsystem
  evidence docs (natives, save/state constraints, trading, campaign purchases).
- New player-visible features are recorded in `docs/NEW_FEATURES.md`.
