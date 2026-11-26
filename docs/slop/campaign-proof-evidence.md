# Issues #55/#110 — campaign proof: exact commands and remaining gates

The checker and its four claims are in the tree:
`scripts/validation/campaign_proof.py` and the claim fixtures under
`scripts/validation/scenarios/campaign/`. Its contract, verdict meanings and
the RED/GREEN evidence are owned by
[CAMPAIGN_PROOF.md](../multiplayer/CAMPAIGN_PROOF.md). No real observation from
a two-seat run is in the tree, so no campaign criterion is claimed as proved
here. Last verified: 2026-11-26.

## Commands that exist

Check the corpus and the checker's own tests (no game needed):

```sh
python3 scripts/validation/scenario_corpus.py --check
python3 -m unittest discover -s scripts/validation -p campaign_proof_test.py
```

Run one claim against an observation from a real two-seat Linux run:

```sh
python3 scripts/validation/campaign_proof.py \
  --claim scripts/validation/scenarios/campaign/rejoin-N2.json \
  --observation <observation.json> --criterion rejoin \
  --commit "$(git rev-parse HEAD)"
```

`--commit` binds the proof to the exact integration ref: the observation must
carry a matching `commit` field or the run is `unreadable` (exit `3`). Without
the flag the checker keeps its fixture-friendly behaviour. The other claims are
`hash-chain.json`, `two-session.json` and `terminal-matrix.json` in the same
directory. Exit `0` means the supplied observation satisfies the claim; `1`
names the failing tick, mission, action or seat; `3` means the input is
unreadable. The committed
`fixture-linux-linux.json` is a **fixture**, not a recording of a played
campaign: a green run against it proves the checker, not the game.

## Blockers

1. **No observation producer.** Nothing in the tree emits the CP observation
   shape from a run; `multiplayer_e2e.py` binds faults and expectations to a
   frame, not to a mission index, and cannot emit CP01–CP04 beyond mission two.
   Capability needed: per-mission binding, counters and deadlines in that
   runner (a `scripts/validation/**` change), then a provisioned two-seat Linux
   run on the build host with full-game data.
2. **Windows cross-play and the owner's full campaign stay `HUMAN_GATE`.**
   Four directions need a real Windows seat behind `player-b-tunnel`, woken by a
   human; Wine parity is not a substitute ([UNATTENDED.md](../UNATTENDED.md),
   ADR 0010). No claim is made for them from a Linux run or a fixture.
3. **Gate status is a fact about a run, not about this page.** A campaign proof
   must name the exact integration sha: gate on the build host via
   `scripts/remote-check.sh`, then confirm origin read-back after landing. No
   gate or landing belongs to this work.

## Doc drift found while checking commands

[CAMPAIGN_PROOF.md](../multiplayer/CAMPAIGN_PROOF.md) § Gate status says the
tooling suite does not discover `campaign_proof_test.py`;
`scripts/check-quality.sh` now runs
`python3 -m unittest discover -s scripts/validation -p campaign_proof_test.py`
(commit `fd9ad2923`). The script is the source of truth; that section needs a
correction in `docs/multiplayer/`, which is outside this slice's file ownership.
