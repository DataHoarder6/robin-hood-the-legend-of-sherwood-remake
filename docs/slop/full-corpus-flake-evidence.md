# Issue #75 — full-corpus load flakes: exact-commit evidence

D09 in [DEFECTS.md](../DEFECTS.md) stays `deferred` (class `release`): `H14`
fails at `--jobs 2`, `H28` and `H33` fail at `--jobs 1`, and each passes in
isolation. The artifact under load is wrong, not the code under test. The known
flake policy in [UNATTENDED.md](../UNATTENDED.md) forbids an indefinite retry or
a "fix" for a row that only flakes under load. No full corpus run was performed
for this work and no exact-commit evidence exists in the tree. Last verified:
2026-11-26.

## What closing #75 needs, in order

1. Check out the exact merged commit in a worktree off `fork/main`; record
   `git rev-parse HEAD`. The binary and the data root must be the provisioned
   ones that commit was built against.
2. Re-run the isolated row rechecks with the exact sha as `--snapshot`:

```sh
python3 scripts/validation/multiplayer_e2e.py \
  --binary /absolute/path/to/robin --data /absolute/path/to/full-game \
  --evidence /tmp/opencode/recheck-H14 --snapshot "$(git rev-parse HEAD)" \
  --only H14 --jobs 1
```

Repeat with `--only H28` and `--only H33`, each with its own empty evidence
directory. Each must pass with its existing `asserts:` clause unchanged.

3. Re-run the full corpus on the build host at that same sha (one gate in
   flight, per [PARALLEL_AGENTS.md](../PARALLEL_AGENTS.md)):

```sh
ROBIN_E2E_BINARY=/absolute/path/to/robin \
ROBINHOOD_DATA_DIR=/absolute/path/to/full-game \
  bash scripts/check-quality.sh multiplayer-e2e
```

Retain each `summary.json` and evidence directory. Only an exact-commit full
corpus pass plus the three isolated rechecks closes the row.

## Blocker

The full-corpus suite is a provisioned build-host gate: it needs the build
host's gate slot, a robin binary built from the exact merged commit with the
release features, the licensed full-game data root and `scripts/remote-check.sh`
admission. None is available in this session; a local rerun is not the gate and
would not be evidence at the exact commit. Keep D09 `deferred` until the owner
or the authorised gate path supplies the run.
