# Deterministic campaign proof

Four Linux-Linux campaign claims are checked by one fixture-driven tool:
[`scripts/validation/campaign_proof.py`](../../scripts/validation/campaign_proof.py).
The tool reads a claim and an observation as JSON and decides whether the
observation proves the claim. It never invents evidence: a missing field, a
wrong schema or an absent file fails closed. Last verified: 2026-11-26.

This page owns the evidence shape and the verdict meaning. The terminology is
[GLOSSARY.md](../../GLOSSARY.md); the `Hnn` process rows stay in
[HARNESS.md](HARNESS.md). The CP rows below are *not* `Hnn` rows and add no
assertion verb to the corpus: they need no running game to check the tool, and
they do not claim a game behaviour was observed.

## Claims and commands

Each claim names one criterion and the thresholds that make it true:

| CP row | Claim fixture | Criterion |
| --- | --- | --- |
| CP01 | [`rejoin-N2.json`](../../scripts/validation/scenarios/campaign/rejoin-N2.json) | a guest dropped in mission 2 returns to mission 2 with the host's progress, the same seat identity and no guest campaign slot |
| CP02 | [`hash-chain.json`](../../scripts/validation/scenarios/campaign/hash-chain.json) | one continuous 25-tick state-hash chain over the run, transition snapshots included |
| CP03 | [`two-session.json`](../../scripts/validation/scenarios/campaign/two-session.json) | one host slot resumes in a second session with the same player assignments and no guest slot |
| CP04 | [`terminal-matrix.json`](../../scripts/validation/scenarios/campaign/terminal-matrix.json) | continue, restart, next and abort from either seat settle within budget and leave no seat on a page the other left |

Run one claim against one observation:

```sh
python3 scripts/validation/campaign_proof.py \
  --claim scripts/validation/scenarios/campaign/rejoin-N2.json \
  --observation scripts/validation/scenarios/campaign/fixture-linux-linux.json \
  --criterion rejoin
```

`--criterion` is optional but recommended: it fails closed when the claim
names a different criterion. The exit codes follow
[HARNESS.md](HARNESS.md) section 5: `0` a proof was observed, `1` an assertion
failed, `3` the claim or observation cannot be read as evidence. stdout is one
JSON object: `observed` with the proof, `failed` with the reason, or `unreadable`.
A desync reports its tick; a fork reports both mission indices; an unsettled
terminal reports its mission, action, seat and budget.

[`fixture-linux-linux.json`](../../scripts/validation/scenarios/campaign/fixture-linux-linux.json)
is an intentionally conforming **fixture**, not a recording of a played
campaign. Its sole purpose is to make the green path repeatable without two
seats. [`campaign_proof_test.py`](../../scripts/validation/campaign_proof_test.py)
mutates that observation once per assertion and 28 tests check that the tool
turns red in the exact place. Run them with:

```sh
python3 -m unittest discover -s scripts/validation -p campaign_proof_test.py
```

## Observation shape

`schema: 1` is mandatory on both inputs; every other key below is read by the
named criterion and a missing one is an `unreadable`/`failed` verdict, never a
pass. Seat ids are integers in claims and JSON object keys in hash and page
maps; the checker accepts both spellings.

| Key | Meaning |
| --- | --- |
| `campaign` | host-owned slot and mission count; context only, not evidence by itself |
| `hash_samples[]` | `{tick, mission_index, hashes: {"0": …, "1": …}}`; one compared frame per sample, in any order in the file |
| `transition_samples[]` | `{tick, from, to, hashes}`; the debriefing handoff's snapshot at that tick |
| `rejoins[]` | `{tick, seat, drop_mission_index, resume_mission_index, host_mission_index, host_progress_preserved, guest_save_writes, seat_identity}` |
| `sessions[]` | `{id, slot, mission_index, assignments, guest_save_writes}`; one row per evening |
| `terminals[]` | `{mission_index, action, pressed_by, settle_s, budget_s, pages: {"0": "gone", "1": "gone"}}` |

The rejoin check requires a hash sample before and after the return, then
checks every sample for equality and the nearest later sample's mission index
against the host's. The chain check merges frame samples and transition
snapshots onto one tick line, refuses a gap wider than `every`, and derives
each sample's mission index from the snapshots before it. The persistence check
compares slots, assignments and guest writes across the declared sessions. The
terminal check requires every mission × action × seat combination, bounds each
settle time by the claim and the row, and requires both seats to have left a
page that was shown.

## What this does and does not prove

- It **proves the checker**: the RED/GREEN tests show a broken observation is
  refused at the named tick, mission, action or seat. That is a deterministic
  harness proof, repeatable without a build host.
- It **does not prove the game**. It decides whether a supplied observation
  satisfies the claim; it cannot produce that observation. Criteria 1, 2, 4,
  5, 6 and 8 are proved against the port only when an observation from a real
  two-seat Linux run is checked here and the gate records the exact tree.
- The process runner [`multiplayer_e2e.py`](../../scripts/validation/multiplayer_e2e.py)
  currently binds faults and expectations to a frame, not to a mission index.
  A run cannot yet emit CP01–CP04 observations beyond mission two: the same
  frame trigger would fire in every mission. Emitting real observations needs
  per-mission binding, per-mission counters and per-mission deadlines in that
  runner; that file is outside this slice's ownership, so it is not changed
  here. See [the endgame plan](CAMPAIGN_ENDGAME_PLAN.md) for the owner-owned
  work item.
- Windows directions and the Windows installer are `HUMAN_GATE` under
  [ADR 0010](../adr/0010-linux-first-unattended-goal.md). A fixture or a
  Linux-only run never proves them.

## Gate status

No gate was run for this proof: no push, gate or landing belongs to this slice.
The tooling suite does not yet discover `campaign_proof_test.py`; adding it to
`scripts/check-quality.sh` is an owner-owned change, so the RED/GREEN evidence
in this page is from the direct unittest command above, not a gate verdict.
