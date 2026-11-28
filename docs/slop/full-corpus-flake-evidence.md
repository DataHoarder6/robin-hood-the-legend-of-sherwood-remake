# Issue #75 — full-corpus load flakes: exact-commit evidence

D09 in [DEFECTS.md](../DEFECTS.md) stays `deferred` (class `release`): `H14`
fails at `--jobs 2`, `H28` and `H33` fail at `--jobs 1`, and each is supposed
to pass in isolation. The artifact under load is wrong, not the code under
test. The known flake policy in [UNATTENDED.md](../UNATTENDED.md) forbids an
indefinite retry or a "fix" for a row that only flakes under load. Last
verified: 2026-11-28.

## Exact-tree run

- Integration commit: `9e09789bbdef019ef689f7739fc5c4cbff3a7e87`.
- Release binary built on `minipc` from that tree with
  `--release --no-default-features --features release`:
  `robin 0.8.0+9e09789bbdef`.
- Data root: `/home/admin/Games/robin-hood/game`.
- Evidence root on the build host:
  `/home/admin/verify-75-exact-evidence2` (an earlier partial root is
  `/home/admin/verify-75-exact-evidence`).

The isolated rechecks used `--only <row> --jobs 1` and each empty evidence
directory:

| Row | Verdict | Evidence |
| --- | --- | --- |
| H14 | `observed` | `verify-75-exact-evidence2/H14/summary.json` |
| H28 | `failed` | `verify-75-exact-evidence2/H28/summary.json` |
| H33 | `failed` | `verify-75-exact-evidence2/H33/summary.json` |

H28 and H33 did not fail on their `asserts:` clauses: the disposable
`briefing_x11.py` worker hit `Xlib.error.BadWindow` while sending Return to a
Robin window that had already gone away, and the runner reported the worker's
non-JSON output as `JSONDecodeError`. A matching failure is retained in the
earlier evidence root. These are harness-input failures, not load flakes, and
no more isolated reruns were made.

The authoritative full corpus ran once at the same sha with the gate's
`--jobs 2` and no `--only`:

- `verify-75-exact-evidence2/full/summary.json`: overall `verdict: hang`.
- `H14`, `H28` and `H33` were each `observed` in that full run.
- `H38` was `hang`: "timed out after 130s settling the session; 3 positive
  expectation(s) never became observable". The other 13 process rows were
  `observed`.

## Outcome

The required isolated H28 and H33 passes were not observed, and the full
corpus was not green, so #75 stays open and D09 stays `deferred`. This page
records the evidence only; no row assertion, `--jobs` setting, or known-flake
policy was changed.

## Reproduce the commands

```sh
python3 scripts/validation/multiplayer_e2e.py \
  --binary /absolute/path/to/robin --data /absolute/path/to/full-game \
  --evidence /tmp/opencode/recheck-H14 --snapshot "$(git rev-parse HEAD)" \
  --only H14 --jobs 1
```

Repeat with `--only H28` and `--only H33`, each with its own empty evidence
directory. Then run the full corpus on the build host:

```sh
ROBIN_E2E_BINARY=/absolute/path/to/robin \
ROBINHOOD_DATA_DIR=/absolute/path/to/full-game \
  bash scripts/check-quality.sh multiplayer-e2e
```
