#!/usr/bin/env python3
"""Check a Linux-Linux campaign observation against scriptable claim fixtures.

A claim names one criterion and the thresholds that make it true. An
observation is the evidence a two-seat run produced, expressed as plain JSON:
per-tick hashes, transition snapshots, rejoin records, session slots and
terminal-button records. This module decides whether that observation proves
the claim. It does not generate an observation and never invents a missing one:
a criterion whose evidence is absent fails closed with the place it broke.

The contract and the evidence schema live in
`docs/multiplayer/CAMPAIGN_PROOF.md`; the claim fixtures live in
`scripts/validation/scenarios/campaign/`. Exit status follows the harness
convention: 0 observed, 1 assertion failed, 3 an unreadable claim, observation
or criterion (a broken harness must not read as a broken campaign).
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

SCHEMA = 1
CRITERIA = ("rejoin", "hash-chain", "persistence", "terminal")


class ProofError(ValueError):
    """An observation fails a claim, or an input cannot be read as one."""


def _require(data, key, kind, where):
    if not isinstance(data, dict) or key not in data:
        raise ProofError(f"{where}: missing {key!r}")
    value = data[key]
    if not isinstance(value, kind) or isinstance(value, bool) and kind is int:
        raise ProofError(f"{where}: {key!r} must be {kind.__name__}")
    return value


def _check_schema(document, where):
    if not isinstance(document, dict):
        raise ProofError(f"{where}: expected a JSON object")
    if document.get("schema") != SCHEMA:
        raise ProofError(f"{where}: schema {document.get('schema')!r} is not {SCHEMA}")


def _hashes(sample, seats, where):
    values = _require(sample, "hashes", dict, where)
    tick = _require(sample, "tick", int, where)
    # Evidence records seat ids as strings in JSON objects; claims name them
    # as integers. Both spellings must denote the same seat.
    found = {str(seat): value for seat, value in values.items()}
    for seat in seats:
        value = found.get(str(seat))
        if not isinstance(value, str) or not value:
            raise ProofError(f"{where}: seat {seat} has no state hash at tick {tick}")
    if len({found[str(seat)] for seat in seats}) != 1:
        raise ProofError(
            f"desync at tick {tick}: seats {seats} reported {values}")
    return found


def evaluate(claim, observed):
    """One claim against one observation; return its proof or raise ProofError."""
    _check_schema(claim, "claim")
    _check_schema(observed, "observation")
    criterion = _require(claim, "criterion", str, "claim")
    if criterion not in CRITERIA:
        raise ProofError(f"claim: unknown criterion {criterion!r}; expected one of {CRITERIA}")
    checker = {
        "rejoin": _rejoin,
        "hash-chain": _hash_chain,
        "persistence": _persistence,
        "terminal": _terminal,
    }[criterion]
    return checker(claim, observed)


def _rejoin(claim, observed):
    """A guest that drops in mission N returns to N with the host's progress."""
    seat = claim.get("seat", 1)
    minimum = _require(claim, "min_mission_index", int, "claim")
    rows = _require(observed, "rejoins", list, "observation")
    candidates = [row for row in rows
                  if isinstance(row, dict) and row.get("seat") == seat
                  and isinstance(row.get("resume_mission_index"), int)
                  and row["resume_mission_index"] >= minimum]
    if not candidates:
        raise ProofError(
            f"no rejoin for seat {seat} at mission index >= {minimum}")
    row = candidates[0]
    tick = _require(row, "tick", int, "rejoin")
    index = row["resume_mission_index"]
    host_index = _require(row, "host_mission_index", int, "rejoin")
    if index != host_index:
        raise ProofError(f"fork: guest resumed mission {index} while the host owns {host_index}")
    if row.get("drop_mission_index") != index:
        raise ProofError(
            f"rejoin at tick {tick}: guest dropped in mission {row.get('drop_mission_index')} "
            f"but resumed mission {index}")
    if row.get("host_progress_preserved") is not True:
        raise ProofError(f"rejoin at tick {tick}: host progress was not preserved")
    writes = _require(row, "guest_save_writes", int, "rejoin")
    if writes > claim.get("guest_saves", 0):
        raise ProofError(f"rejoin at tick {tick}: guest save wrote {writes} local campaign slot(s)")
    if claim.get("require_same_identity", True) and row.get("seat_identity") != "same":
        raise ProofError(
            f"rejoin at tick {tick}: seat identity {row.get('seat_identity')!r} is not the same seat")

    seats = claim.get("seats", [0, 1])
    samples = _require(observed, "hash_samples", list, "observation")
    earlier = later = None
    for sample in samples:
        if not isinstance(sample, dict):
            raise ProofError("observation: hash_samples contains a non-object")
        sample_tick = _require(sample, "tick", int, "hash sample")
        _hashes(sample, seats, "hash sample")
        if sample_tick < tick:
            if earlier is None or sample_tick > earlier["tick"]:
                earlier = sample
        if sample_tick > tick and (later is None or sample_tick < later["tick"]):
            later = sample
    if earlier is None or later is None:
        raise ProofError(
            f"rejoin at tick {tick}: need hash samples before and after the return, "
            f"found {'one' if earlier else 'none'} before and {'one' if later else 'none'} after")
    resumed = _require(later, "mission_index", int, "hash sample")
    if resumed != index:
        raise ProofError(
            f"rejoin at tick {tick}: guest is in mission {resumed}, host owns {index}")
    return {"seat": seat, "tick": tick, "mission_index": index,
            "hash_samples_before": earlier["tick"], "hash_samples_after": later["tick"]}


def _hash_chain(claim, observed):
    """One continuous, equal hash chain over every mission and snapshot."""
    seats = claim.get("seats", [0, 1])
    every = _require(claim, "every", int, "claim")
    frames = _require(observed, "hash_samples", list, "observation")
    snapshots = _require(observed, "transition_samples", list, "observation")
    if len(frames) < 2:
        raise ProofError("hash chain: at least two hash samples are required")

    entries = []
    for sample in frames + snapshots:
        if not isinstance(sample, dict):
            raise ProofError("hash chain: a sample is not an object")
        tick = _require(sample, "tick", int, "hash sample")
        _hashes(sample, seats, f"hash sample at tick {tick}")
        entries.append(sample)
    entries.sort(key=lambda item: item["tick"])
    # A transition snapshot is also a hash sample at its tick: the chain has
    # one entry per tick, with the snapshot's equality checked separately.
    ticks = sorted({item["tick"] for item in entries})
    if ticks[0] != 0:
        raise ProofError(f"hash chain does not start at tick 0 (first tick {ticks[0]})")
    for index, tick in enumerate(ticks):
        if index and tick - ticks[index - 1] > every:
            raise ProofError(
                f"gap at tick {ticks[index - 1] + every}: chain jumped from "
                f"{ticks[index - 1]} to {tick} (> {every})")

    if claim.get("include_transitions", True):
        if not snapshots:
            raise ProofError("hash chain: no transition snapshot, so the handoff is unproved")
        transition_ticks = []
        for snapshot in snapshots:
            tick = _require(snapshot, "tick", int, "transition snapshot")
            source = _require(snapshot, "from", int, "transition snapshot")
            target = _require(snapshot, "to", int, "transition snapshot")
            if target != source + 1:
                raise ProofError(
                    f"transition at tick {tick} skips a mission ({source} -> {target})")
            if tick not in ticks:
                raise ProofError(f"transition snapshot at tick {tick} is not in the hash chain")
            transition_ticks.append(tick)

        for sample in sorted(frames, key=lambda item: item["tick"]):
            tick = _require(sample, "tick", int, "hash sample")
            # The snapshot at a transition tick still belongs to the mission
            # it ends; the next mission's samples start at later ticks.
            expected = 1 + sum(when < tick for when in transition_ticks)
            index = _require(sample, "mission_index", int, "hash sample")
            if index != expected:
                raise ProofError(
                    f"mission index at tick {tick}: observation says {index}, "
                    f"the chain's transitions say {expected}")
    return {"ticks": ticks, "first_tick": ticks[0], "last_tick": ticks[-1],
            "transitions": len(snapshots)}


def _persistence(claim, observed):
    """The host slot resumes across two sessions with the same assignments."""
    sessions = _require(observed, "sessions", list, "observation")
    wanted = _require(claim, "sessions", int, "claim")
    if len(sessions) < wanted:
        raise ProofError(f"persistence: observation has {len(sessions)} session(s), need {wanted}")
    chosen = sessions[:wanted]
    slot = None
    assignments = None
    indices = []
    for position, session in enumerate(chosen):
        where = f"session {position + 1}"
        if not isinstance(session, dict):
            raise ProofError(f"{where}: not an object")
        current = _require(session, "slot", str, where)
        current_assignments = _require(session, "assignments", dict, where)
        writes = _require(session, "guest_save_writes", int, where)
        indices.append(_require(session, "mission_index", int, where))
        if slot is None:
            slot, assignments = current, current_assignments
        elif claim.get("same_slot", True) and current != slot:
            raise ProofError(
                f"persistence: {where} resumed slot {current!r}, host slot is {slot!r}")
        elif claim.get("same_assignments", True) and current_assignments != assignments:
            raise ProofError(
                f"persistence: {where} changed player assignments from {assignments} "
                f"to {current_assignments}")
        if writes > claim.get("guest_saves", 0):
            raise ProofError(f"persistence: {where}: guest save wrote {writes} local campaign slot(s)")
        if position and indices[-1] < indices[-2]:
            raise ProofError(f"persistence: {where} went backwards to mission {indices[-1]}")
    if "resume_mission_index" in claim and indices[-1] != claim["resume_mission_index"]:
        raise ProofError(
            f"persistence: resume mission {indices[-1]}, expected "
            f"{claim['resume_mission_index']}")
    return {"slot": slot, "sessions": wanted, "mission_indices": indices,
            "assignments": assignments}


def _terminal(claim, observed):
    """Every action, pressed by either seat, settles on a shared page."""
    rows = _require(observed, "terminals", list, "observation")
    missions = _require(claim, "mission_indices", list, "claim")
    actions = _require(claim, "actions", list, "claim")
    seats = _require(claim, "seats", list, "claim")
    budget = _require(claim, "budget_s", int, "claim")
    page = claim.get("pages", "gone")
    by_key = {}
    for row in rows:
        if not isinstance(row, dict):
            raise ProofError("terminal: a record is not an object")
        key = (row.get("mission_index"), row.get("action"), row.get("pressed_by"))
        if key in by_key:
            raise ProofError(f"terminal: duplicate record for mission {key[0]} "
                             f"{key[1]} seat {key[2]}")
        by_key[key] = row
    combinations = slowest = 0
    for mission in missions:
        for action in actions:
            for seat in seats:
                key = (mission, action, seat)
                if key not in by_key:
                    raise ProofError(f"mission {mission} {action} seat {seat}: no terminal record")
                row = by_key[key]
                settle = row.get("settle_s")
                if not isinstance(settle, (int, float)) or isinstance(settle, bool):
                    raise ProofError(f"mission {mission} {action} seat {seat}: no settle time")
                bound = min(budget, row.get("budget_s", budget))
                if settle > bound:
                    raise ProofError(
                        f"mission {mission} {action} seat {seat}: settled in {settle}s, "
                        f"budget {bound}s")
                pages = _require(row, "pages", dict, "terminal")
                shown_pages = {str(key): value for key, value in pages.items()}
                for shown in seats:
                    state = shown_pages.get(str(shown))
                    if state != page:
                        raise ProofError(
                            f"mission {mission} {action} seat {seat}: terminal page: "
                            f"seat {shown} is on {state!r}, expected {page!r}")
                slowest = max(slowest, settle)
                combinations += 1
    return {"combinations": combinations, "slowest_settle_s": slowest}


def load(path, where):
    try:
        return json.loads(Path(path).read_text())
    except FileNotFoundError as error:
        raise ProofError(f"{where}: {path}: no such file") from error
    except OSError as error:
        raise ProofError(f"{where}: {path}: {error}") from error
    except json.JSONDecodeError as error:
        raise ProofError(f"{where}: {path}: not JSON: {error}") from error


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--claim", type=Path, required=True,
                        help="fixture naming the criterion and its thresholds")
    parser.add_argument("--observation", type=Path, required=True,
                        help="JSON evidence from a two-seat run")
    parser.add_argument("--criterion", choices=CRITERIA,
                        help="fail closed if the claim names a different criterion")
    parser.add_argument("--commit", metavar="SHA",
                        help="bind the observation to the exact integration ref it was run on")
    arguments = parser.parse_args(argv)
    try:
        claim = load(arguments.claim, "claim")
        observed = load(arguments.observation, "observation")
        _check_schema(claim, "claim")
        if arguments.criterion and claim.get("criterion") != arguments.criterion:
            raise ProofError(
                f"claim: names {claim.get('criterion')!r}, not {arguments.criterion!r}")
        if arguments.commit:
            recorded = observed.get("commit")
            if not isinstance(recorded, str) or not recorded:
                raise ProofError(
                    "observation: no commit, so it cannot be read as evidence at "
                    f"{arguments.commit}")
            if recorded != arguments.commit:
                raise ProofError(
                    f"observation: commit {recorded!r} is not the requested "
                    f"{arguments.commit!r}")
    except ProofError as error:
        print(json.dumps({"verdict": "unreadable", "reason": str(error)}, indent=2))
        return 3
    try:
        result = evaluate(claim, observed)
    except ProofError as error:
        print(json.dumps({"verdict": "failed", "reason": str(error)}, indent=2))
        return 1
    answer = {"verdict": "observed", "criterion": claim["criterion"],
              "claim": str(arguments.claim), "observation": str(arguments.observation),
              "proof": result}
    if arguments.commit:
        answer["commit"] = arguments.commit
    print(json.dumps(answer, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
