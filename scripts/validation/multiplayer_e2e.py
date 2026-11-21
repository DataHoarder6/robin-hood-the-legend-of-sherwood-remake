#!/usr/bin/env python3
"""Gated process-layer multiplayer end-to-end suite (issue #6).

The contract is `docs/multiplayer/HARNESS.md`: section 2 owns the scenario
schema, the fault vocabulary and each row's `proves` binding, section 5 owns this
layer's execution site, evidence layout and exit contract, section 6 owns the
bootstrap. Section 1's corpus table is the coverage floor and names which rows
this layer proves.

This is section 2's process layer: real `robin` binaries, real signals, real
per-install state, driven and asserted over loopback HTTP. The corpus is loaded
through `scenario_corpus` (one schema, one expansion) and only the rows whose
`proves.layer` is `process` run here — a `log` or `exit` assertion is what makes
a row one. Every other scenario is listed in `summary.json` with the layer its
`proves` names, so nothing is dropped silently.

Exit status (section 5):

  0  every `expect` of every selected scenario was observed
  1  an assertion failed, or a selected row was refused because this layer
     cannot express its faults. A refusal is a harness gap, not a clean run:
     the row is listed under `refused` in `summary.json` and keeps the suite red
     while section 1 still claims that cell
  2  the deadline was hit: a scenario budget, a barrier wait bounded by the
     transition-watchdog deadline plus a margin, or a settle window that covers
     the transport's idle timeout and the client's reconnect budget. Exit 2
     means a promised state was missed, never that the machine was slow
  3  a prerequisite is missing, the corpus or its contract cannot be read, and
     nothing about the game was proved

Nothing is installed and nothing is skipped.

  ROBINHOOD_DATA_DIR=/path/to/full-game \\
    python3 scripts/validation/multiplayer_e2e.py \\
      --binary target/debug/robin --data "$ROBINHOOD_DATA_DIR" \\
      --snapshot "$(git rev-parse HEAD)" --evidence /tmp/mp-e2e

The runner confines itself to a loopback-only user/network namespace with
`unshare --user --map-root-user --net` and starts its own Xvfb inside it, so a
run never shares a display or a network with another session.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import functools
import json
import multiprocessing
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import traceback
import urllib.error
import urllib.request

import input_worker
import scenario_corpus
from lifecycle_gate import require_python_xlib
from runtime_evidence import (
    DriverInterrupted,
    digest,
    finish_children,
    snapshot_client,
    stop_process_group,
    verify_client,
)
import supervised

ROOT = Path(__file__).resolve().parents[2]
CORPUS = ROOT / "scripts/validation/scenarios"
OVERLAY = ROOT / "assets/core-datadir"
ANSI = re.compile(r"\x1b\[[0-9;]*m")

# Section 5: "a barrier wait is bounded by the transition-watchdog deadline (#4)
# plus a margin". The deadline is the snapshot barrier's commit budget — 60 s
# plus one second per 64 KiB of payload (.../multiplayer/native/server_protocol.rs
# TRANSITION_COMMIT_MS and TRANSITION_PAYLOAD_BYTES_PER_SECOND). These scenarios
# carry no payload near that, so the fixed part plus a margin bounds the wait.
TRANSITION_WATCHDOG_S = 60
WATCHDOG_MARGIN_S = 30
BARRIER_DEADLINE_S = TRANSITION_WATCHDOG_S + WATCHDOG_MARGIN_S
# A run settles once no peer has advanced a frame or its admission phase for
# this long: longer than INITIAL_CONNECT_TIMEOUT (15 s,
# crates/robin_rs/src/multiplayer/client_session.rs), so a peer that is silently
# retrying its first handshake has spent its budget and said so before the
# driver calls the session settled. Log growth is not a progress signal: a
# reconnecting peer writes a schedule line per second forever.
QUIET_S = 25
# After the last injected fault, every row is read one initial-handshake budget
# later: the killed or refused peer has had its full chance to report, and a
# negative promise ("the host released no BeginSim") is only worth reading after
# the reaction it denies has had time to happen. A reaction later than that
# belongs to a different fault, not to this one.
SETTLE_S = 20
# A peer whose session dropped keeps retrying for the client's reconnect budget
# (.../multiplayer/client_session.rs RECONNECT_BUDGET) before it publishes the
# fatal that ends the process, and each attempt may be asleep for the capped
# backoff (MAX_RECONNECT_BACKOFF) when the budget runs out. The drop itself is
# only noticed when the transport's idle timeout expires: noq's default
# `max_idle_timeout` is 30 s (noq-proto TransportConfig::default), which iroh
# keeps. Section 1's bounded reconnect is observable only after that whole
# window, so the settle deadline covers it.
TRANSPORT_IDLE_S = 30
RECONNECT_BUDGET_S = 60
RECONNECT_BACKOFF_MAX_S = 10
SESSION_DEADLINE_S = max(BARRIER_DEADLINE_S,
                         TRANSPORT_IDLE_S + RECONNECT_BUDGET_S
                         + RECONNECT_BACKOFF_MAX_S + WATCHDOG_MARGIN_S)
# A running session that has not advanced a frame for this long is holding a
# story scroll: the driver presses Return on the real window again.
SURFACE_RETRY_S = 4
MAX_DISMISSALS = 8
DEFAULT_BUDGET_S = 600
BOOT_S = 120

# Section 2: the layer this suite is, named by each row's `proves`.
PROCESS_LAYER = "process"

# Section 2's process column, and the verbs it cannot express from a scenario
# file: `truncate` is marked unsupported there, while `diverge` ("POST /native
# on that peer alone") and `stall-surface` ("same") need a native op or a
# surface deadline the fault schema does not carry. The `netem` verbs are the
# fourth: the table names `netem` for them, but this runner applies faults with
# signals and relaunches only, so selection refuses them by name instead of
# letting `apply` read a harness gap as a game failure.
PROCESS_FAULTS = frozenset({"kill", "stop", "continue", "restart",
                            "mission-result", "console", "command", "native", "key"})
NETEM_FAULTS = frozenset({"duplicate", "drop", "reorder", "delay"})
NETEM_GAP = ("section 2 names `netem {verb}` for the process layer and this suite has no "
             "netem driver, so the row is a harness gap")
UNEXPRESSIBLE_FAULTS = {
    "diverge": "section 2's process meaning is `POST /native` on that peer alone, and a "
               "fault names no native op or arguments to post",
    "truncate": "section 2 marks `truncate` unsupported at the process layer",
    "stall-surface": "a fault names no surface or deadline for the process layer to hold",
}

# Admission phases `/state` reports (.../game_session/runtime/admission.rs
# wire_name). Section 1 names barriers while `/state` names phases, so this is
# the one mapping between them; `modal-ack` has no phase or field at all and is
# therefore unprovable here. A barrier is *reached* when the peer is at it and
# *released* when it is past it, so `running` is never a reached content or
# snapshot barrier — it means the peer has moved on.
#
# The two admission phases are transient: a joining peer's snapshot and
# `BeginSim` arrive in the same handful of frames, so a 100 ms sample can miss
# both (a run watched `peer_waiting_for_snapshot` → `waiting_for_start` with no
# sample between, and a row that anchored a fault on `content-ready` waited out
# its whole deadline). A fault may therefore be scheduled only on a fact that
# persists — a frame counter, a set `begin_sim_frame`, a seat — and a pre-ack
# drop is proved at transport, where it is applied in process.
CLIENT_PHASE_ORDER = (
    "peer_waiting_for_snapshot",
    "peer_waiting_for_begin",
    "waiting_for_start",
    "running",
)
CLIENT_CONTENT_PHASE = "peer_waiting_for_snapshot"
CLIENT_SNAPSHOT_PHASE = "peer_waiting_for_begin"
RUNNING_PHASES = frozenset({"running"})
POSITIVE_ASSERTIONS = frozenset({"exit", "barrier", "final-operation", "mission",
                                  "hash-equal", "presentation"})
# `exit` states a peer is gone: zero or nonzero. `running` states it is still
# there, which is how a row says "the host survived this fault" — `/state`
# evidence alone cannot, because the samples stop when the process does.
# `hash-equal` is positive because it only holds once a cadence was observed at
# all; a row whose whole promise is "the peers agree and the session keeps
# running" would otherwise wait out the deadline instead of settling when the
# agreement it names has been watched.
EXIT_RUNNING = "running"

# Section 5 evidence per run. `binary.sha256` is repeated inside each run so a
# single scenario directory is a complete record on its own.
EVIDENCE_FILES = ("scenario.json", "summary.json", "events.jsonl", "hashes.json",
                  "binary.sha256")


class PrerequisiteMissing(RuntimeError):
    """Section 6: nothing about the game was proved (exit 3)."""


class ScenarioDeadline(RuntimeError):
    """Section 5: a promised state was missed inside its budget (exit 2)."""


class Unprovable(RuntimeError):
    """A selected expectation has no process-layer observation (exit 1)."""


# ── Contract reading and selection (pure) ────────────────────────────────


def contract_faults(scenarios, identifier):
    return [
        fault
        for step in scenario_corpus.chain(identifier, scenarios)
        for fault in scenarios[step][1].get("faults", [])
    ]


def unexpressible(faults):
    """Why the process layer cannot express these faults, or `None`."""
    for fault in faults:
        verb = fault["fault"]
        if verb in PROCESS_FAULTS:
            continue
        if verb in NETEM_FAULTS:
            return NETEM_GAP.format(verb=verb)
        if verb in UNEXPRESSIBLE_FAULTS:
            return UNEXPRESSIBLE_FAULTS[verb]
        return f"unknown fault verb `{verb}`"
    return None


def proves_layer(scenarios, identifier):
    """The layer the row's `proves` names (the corpus gate has checked it)."""
    try:
        return scenarios[identifier][1]["proves"]["layer"]
    except (KeyError, TypeError) as error:
        raise scenario_corpus.ContractError(
            f"{scenarios[identifier][0]}: no `proves` binding, so nothing proves it"
        ) from error


def select(scenarios):
    """Split the corpus by the binding its own row declares (section 2).

    Returns `(run, skipped, refused)`. Both non-running lists carry the reason, so
    a scenario is never dropped silently. A refused row is a harness gap: it is
    proved by this suite, which cannot express it, and section 1 still claims the
    cell, so it keeps the suite red until one side changes.
    """
    run, skipped, refused = [], [], []
    for identifier in sorted(scenarios):
        layer = proves_layer(scenarios, identifier)
        if layer != PROCESS_LAYER:
            skipped.append({
                "id": identifier,
                "prover": layer,
                "reason": (
                    f"section 1 binds {identifier} to {layer!r} and its `proves` names the "
                    "Rust test that executes it; this suite runs only process rows"
                ),
            })
            continue
        reason = unexpressible(contract_faults(scenarios, identifier))
        if reason is None:
            run.append(identifier)
        else:
            refused.append({"id": identifier, "kind": "harness-gap", "reason": reason})
    return run, skipped, refused


def exit_code(verdicts):
    """Section 5: 0 only when every selected scenario observed its expectations."""
    return 0 if verdicts and all(verdict == "observed" for verdict in verdicts.values()) else 1


def suite_verdict(prerequisites_missing, interrupted, hangs, refused, verdicts):
    """The suite's one verdict, worst first.

    A refused row never ran, but section 1 still claims its cell with this suite
    as the prover, so a refusal is a harness gap and is never a clean run.
    """
    if prerequisites_missing:
        return "prerequisite-missing"
    if interrupted:
        return "interrupted"
    if hangs:
        return "hang"
    if refused:
        return "harness-gap"
    return "observed" if exit_code(verdicts) == 0 and verdicts else "failed"


SUITE_EXIT = {"prerequisite-missing": 3, "interrupted": 2, "hang": 2}


def suite_exit_code(verdict):
    """Section 5's exit contract, from the verdict alone."""
    return SUITE_EXIT.get(verdict, 0 if verdict == "observed" else 1)


# ── Barrier observations (pure) ──────────────────────────────────────────


def _phase_after(admission, phase):
    """Whether `admission` is past `phase` in the client's admission order."""
    try:
        return CLIENT_PHASE_ORDER.index(admission) > CLIENT_PHASE_ORDER.index(phase)
    except ValueError:
        return False


def barrier_reached(name, peer, sample):
    """Whether one `/state` sample shows `peer` at barrier `name`."""
    multiplayer = sample.get("multiplayer") or {}
    admission = multiplayer.get("admission")
    if name == "begin-sim":
        return multiplayer.get("begin_sim_frame") is not None
    if name == "content-ready":
        return peer["role"] == "client" and admission == CLIENT_CONTENT_PHASE
    if name == "snapshot-adopt":
        return peer["role"] == "client" and admission == CLIENT_SNAPSHOT_PHASE
    if name == "transition-commit":
        return multiplayer.get("pending_transition") is not None
    if name == "modal-ack":
        raise Unprovable(
            "section 3 grows `multiplayer` with no modal field, so `modal-ack` cannot be "
            "observed through /state and the corpus row is unprovable at this layer"
        )
    raise Unprovable(f"unknown barrier `{name}`")


def barrier_released(name, peer, sample):
    """Whether one sample shows the barrier released: the peer is past it."""
    multiplayer = sample.get("multiplayer") or {}
    admission = multiplayer.get("admission")
    if name == "begin-sim":
        return (barrier_reached(name, peer, sample)
                and multiplayer.get("admission") in RUNNING_PHASES)
    if name == "content-ready":
        return peer["role"] == "client" and _phase_after(admission, CLIENT_CONTENT_PHASE)
    if name == "snapshot-adopt":
        return peer["role"] == "client" and _phase_after(admission, CLIENT_SNAPSHOT_PHASE)
    if name == "transition-commit":
        pending = multiplayer.get("pending_transition")
        return pending is not None and not pending.get("awaiting")
    raise Unprovable(f"unknown barrier `{name}`")


def barrier_samples(name, state, trace, peer_seat=None):
    """Indices of the trace where `name` is `reached` or `released`."""
    hits = []
    for index, (peer, sample) in enumerate(trace):
        if peer_seat is not None and peer["seat"] != peer_seat:
            continue
        if state == "reached" and barrier_reached(name, peer, sample):
            hits.append({"seat": peer["seat"], "sample": index})
        elif state == "released" and barrier_released(name, peer, sample):
            hits.append({"seat": peer["seat"], "sample": index})
    return hits


# ── Assertion evaluation (pure) ──────────────────────────────────────────


def evaluate_log(expectation, texts, peers):
    """`log` over every peer's stderr, in seat order.

    Section 2's shape carries no peer field, so the promise is about the
    session's aggregated output; per-peer counts are recorded beside the verdict
    so a failure names who spoke. `count` is exact; `at_least` is the floor a
    line printed once per retry needs, because its exact number is a schedule.
    """
    needle = expectation.get("contains") or expectation.get("absent")
    counts = {peer["seat"]: texts.get(peer["seat"], "").count(needle) for peer in peers}
    observed = sum(counts.values())
    if "at_least" in expectation:
        wanted = expectation["at_least"]
        ok = observed >= wanted
        detail = f"`{needle}` appeared {observed} times in the peers' output, expected at least {wanted}"
        return ok, detail, counts
    wanted = expectation["count"]
    ok = observed == wanted
    detail = f"`{needle}` appeared {observed} times in the peers' output, expected {wanted}"
    return ok, detail, counts


def evaluate_barrier(expectation, record):
    name, state = expectation["barrier"], expectation["state"]
    # `absent` is denied by any *reached* sample: `barrier_samples` only knows
    # reached and released, so asking it for "absent" would count nothing and
    # pass the row for free.
    hits = barrier_samples(name, "reached" if state == "absent" else state, record["trace"])
    ok = not hits if state == "absent" else bool(hits)
    detail = f"`{name}` {state}: {len(hits)} of {len(record['trace'])} /state samples matched"
    return ok, detail, hits


def evaluate_exit(expectation, record):
    seats = expectation["peers"]
    status = expectation["status"]
    observed = {seat: record["statuses"].get(seat) for seat in seats}
    if status == EXIT_RUNNING:
        ok = all(value is None for value in observed.values())
    else:
        nonzero = status == "nonzero"
        ok = all(value is not None and (value != 0) == nonzero
                 for value in observed.values())
    return ok, f"exit status {status} for seats {seats}: {observed}", observed


def evaluate_final_operation(expectation, record):
    observed = {
        peer["seat"]: (((record["latest"].get(peer["seat"]) or {}).get("multiplayer") or {})
                       .get("final_operation"))
        for peer in record["peers"]
    }
    ok = all(value == expectation["operation"] for value in observed.values())
    return ok, f"final operation {expectation['operation']}: {observed}", observed


def still_booting(latest, seats):
    """Whether any seat has not yet answered `/state` with a frame.

    A peer that is still building its mission publishes no frame and no
    admission phase, so the session looks quiet for as long as its boot takes.
    That silence is the peer's, not the session's: a row that settled on it
    would read a slow boot as a stopped session.
    """
    return any((latest.get(seat) or {}).get("frame") is None for seat in seats)


def presentation_samples(record, seat):
    """Every `presentation` document one seat's samples carried, in order.

    `/state.presentation` is null unless the terminal flow is showing a page, so
    the list is the page's appearances and disappearances as they were observed.
    """
    return [sample.get("presentation")
            for peer, sample in record["trace"] if peer["seat"] == seat]


def evaluate_presentation(expectation, record):
    """The terminal page a seat showed, or that it left the one it showed.

    `shown` is a page appearing (of the named kind, when the row names one);
    `gone` is a page that appeared and a later sample with nothing on screen,
    which is what "this seat left the debriefing" means and what "the page was
    never reached" must not pass for.
    """
    state, kind = expectation["state"], expectation.get("kind")
    observed, problems = {}, []
    for seat in expectation["peers"]:
        samples = presentation_samples(record, seat)
        pages = [page for page in samples if page]
        named = [page for page in pages if kind is None or kind in (page.get("kind") or {})]
        left = any(page and None in samples[index + 1:]
                   for index, page in enumerate(samples))
        observed[seat] = {"samples": len(samples), "pages": len(pages),
                          "matching": len(named), "left": left}
        wanted = f"a page of kind {kind}" if kind else "a page"
        if not named:
            problems.append(f"seat {seat}: {wanted} never appeared in {len(samples)} samples")
        elif state == "gone" and not left:
            problems.append(f"seat {seat}: {wanted} appeared but was never left")
    detail = f"presentation {state} for seats {expectation['peers']}: {observed}"
    if problems:
        detail += "; " + "; ".join(problems)
    return not problems, detail, observed


def multiplayer_samples(record, seat):
    """Every `(frame, multiplayer block)` of one seat, in sampling order."""
    return [(sample.get("frame"), sample.get("multiplayer") or {})
            for peer, sample in record["trace"] if peer["seat"] == seat]


def hash_cadence(samples, every):
    """Why one seat's compared frames do not honour a `every`-frame cadence.

    `/state` reports only `last_compared_hash_frame`, so the cadence is read off
    consecutive samples: every compared frame is on the declared boundary, the
    counter never runs backwards, and when the peer advanced by less than two
    boundaries the counter must advance by exactly one. A larger step is only
    honest if the peer advanced far enough that the samples could not have seen
    the boundary in between, which is why the sampling interval is bounded here
    rather than assumed.

    A snapshot transition starts a new simulation generation: its frame counter,
    and with it the hash schedule, begins again (`begin_sim_frame` names the
    boundary). The rules therefore apply within one generation, and each
    generation that compares a hash must show a cadence of its own — a restart
    whose replacement mission compares once proves nothing about agreement.
    """
    if not samples:
        return ["no /state sample"]
    problems, compared_anywhere = [], set()
    for generation in _hash_generations(samples):
        problems += _generation_cadence(generation, every)
        compared_anywhere |= {multiplayer.get("last_compared_hash_frame")
                              for _, multiplayer in generation
                              if multiplayer.get("last_compared_hash_frame") is not None}
    if not compared_anywhere:
        problems.append("no hash comparison was ever observed")
    return problems


def _hash_generations(samples):
    """Split one seat's samples where a new simulation generation begins."""
    generations, current, marker = [], [], None
    for sample in samples:
        value = (sample[1] or {}).get("begin_sim_frame")
        if current and value != marker:
            generations.append(current)
            current = []
        marker = value
        current.append(sample)
    if current:
        generations.append(current)
    return generations


def _generation_cadence(samples, every):
    """`hash_cadence`'s rules inside one generation."""
    problems = []
    frames = [frame for frame, _ in samples]
    compared = [multiplayer.get("last_compared_hash_frame") for _, multiplayer in samples]
    for value in compared:
        if value is not None and value % every:
            problems.append(f"compared frame {value} is not a multiple of {every}")
    seen = sorted({value for value in compared if value is not None})
    if not seen:
        return problems
    if len(seen) < 2:
        problems.append(
            f"only one compared frame ({seen[0]}) was observed, so the {every}-frame "
            "cadence is unobservable")
    for index in range(1, len(samples)):
        before, after = compared[index - 1], compared[index]
        if before is None or after is None or after <= before:
            continue
        step = after - before
        if step % every:
            problems.append(
                f"compared frames jumped {before} -> {after}, a step that is not a "
                f"multiple of {every}")
        elif (frames[index] is not None and frames[index - 1] is not None
              and frames[index] - frames[index - 1] < 2 * every and step != every):
            problems.append(
                f"compared frames jumped {before} -> {after} while the peer advanced only "
                f"{frames[index] - frames[index - 1]} frames")
    return problems


def evaluate_hash_equal(expectation, record):
    """Section 1's oracle: the peers stay equal on the declared hash cadence."""
    peers, every = expectation["peers"], expectation["every"]
    observed, problems = {}, []
    for seat in peers:
        samples = multiplayer_samples(record, seat)
        desync = max((multiplayer.get("desync_count") or 0
                      for _, multiplayer in samples), default=0)
        compared = sorted({multiplayer.get("last_compared_hash_frame")
                           for _, multiplayer in samples
                           if multiplayer.get("last_compared_hash_frame") is not None})
        observed[seat] = {"last_compared_hash_frame": compared[-1] if compared else None,
                          "compared_frames": len(compared),
                          "desync_count": desync}
        if desync:
            problems.append(f"seat {seat}: {desync} desync(s) observed")
        problems += [f"seat {seat}: {problem}" for problem in hash_cadence(samples, every)]
    detail = (f"hash agreement for seats {peers} every {every}: {observed}"
              + ("" if not problems else "; " + "; ".join(problems)))
    return not problems, detail, observed


def evaluate_hash_diverged(expectation, record):
    peers = expectation["peers"]
    observed = {
        seat: max((multiplayer.get("desync_count") or 0
                   for _, multiplayer in multiplayer_samples(record, seat)), default=0)
        for seat in peers
    }
    ok = all(count >= 1 for count in observed.values())
    return ok, f"hash divergence counted on seats {peers}: {observed}", observed


def evaluate_mission(expectation, record):
    """Each named seat has been through `count` missions and they are together.

    `/state` reports the map the mission built, one map per mission, so a seat's
    distinct maps are the missions it entered. The seats must also be on the
    same one now: a session that handed the campaign to a second local save
    would leave them on different missions, and a seat left behind would show
    fewer maps than the others.
    """
    seats, count = expectation["peers"], expectation["count"]
    maps = {seat: [] for seat in seats}
    for peer, sample in record["trace"]:
        if peer["seat"] in maps and sample.get("map"):
            if sample["map"] not in maps[peer["seat"]]:
                maps[peer["seat"]].append(sample["map"])
    current = {seat: (record["latest"].get(seat) or {}).get("map") for seat in seats}
    entered = {seat: len(maps[seat]) for seat in seats}
    together = len(set(current.values())) == 1 and current[seats[0]] is not None
    ok = all(value >= count for value in entered.values()) and together
    detail = (f"missions for seats {seats}: entered {entered} of {count}; "
              f"now on {current}; maps {maps}")
    return ok, detail, {"entered": entered, "current": current, "maps": maps}


def evaluate_expectation(expectation, record):
    """One `expect` entry against everything the run recorded."""
    verb = expectation["assert"]
    if verb == "log":
        return evaluate_log(expectation, record["texts"], record["peers"])
    if verb == "barrier":
        return evaluate_barrier(expectation, record)
    if verb == "exit":
        return evaluate_exit(expectation, record)
    if verb == "final-operation":
        return evaluate_final_operation(expectation, record)
    if verb == "hash-equal":
        return evaluate_hash_equal(expectation, record)
    if verb == "hash-diverged":
        return evaluate_hash_diverged(expectation, record)
    if verb == "mission":
        return evaluate_mission(expectation, record)
    if verb == "presentation":
        return evaluate_presentation(expectation, record)
    raise Unprovable(f"assertion `{verb}` is not implemented at the process layer")


# ── Evidence layout and prerequisites (pure) ─────────────────────────────


def run_directory(evidence, identifier):
    """The first free `<evidence>/<id>/<run>`; section 5 refuses a used one.

    `evidence` itself is checked by `require_empty`, so an operator never mixes
    two sessions' artifacts.
    """
    root = Path(evidence) / identifier
    root.mkdir(parents=True, exist_ok=True)
    run = 1
    while (root / str(run)).exists():
        run += 1
    path = root / str(run)
    path.mkdir()
    return path


def peer_summaries(scenario):
    """One entry per peer process: where it ran and how it ended."""
    return [
        {"name": peer.name, "seat": peer.seat, "role": peer.role, "port": peer.port,
         "install": str(peer.install), "log": str(peer.log),
         "samples": str(peer.samples), "launches": peer.launches,
         "exit": peer.exit_status()}
        for peer in scenario.peers
    ]


def hash_manifest(scenario):
    """Section 5's per-peer hash manifest, from the samples already collected.

    `/state` carries the compared frame and the desync count, not the hash value,
    so that is what a manifest can honestly record. The host's mission
    recordings travel beside it: the recorder writes one directory per mission
    into the install's `replays/`, so a run that re-enters its mission leaves two
    attempts and a host killed at frame zero leaves none. No `--record` names
    them, because a named directory cannot hold a second mission of the same
    process.
    """
    trace = scenario.record()["trace"] if scenario is not None else []
    return {
        "schema": 2,
        "samples": [
            {"seat": peer["seat"],
             "frame": sample.get("frame"),
             "compared_frame": (sample.get("multiplayer") or {}).get("last_compared_hash_frame"),
             "desync_count": (sample.get("multiplayer") or {}).get("desync_count")}
            for peer, sample in trace
        ],
        "recordings": [] if scenario is None else mission_recordings(scenario),
    }


def mission_recordings(scenario):
    """One entry per mission recording the host install holds, oldest first."""
    host = next((peer for peer in scenario.peers if peer.role == "host"), None)
    if host is None:
        return []
    root = Path(host.install) / "replays"
    if not root.is_dir():
        return []
    recordings = []
    for directory in sorted(path for path in root.iterdir()
                            if path.is_dir() and path.name.endswith(".mission")):
        mission = directory / "mission.json"
        recordings.append({
            "path": str(directory.relative_to(scenario.run)),
            "files": sorted(path.name for path in directory.iterdir() if path.is_file()),
            "mission_sha256": digest(mission) if mission.is_file() else None,
        })
    return recordings


def require_empty(path):
    """Section 5: a non-empty evidence directory is refused, not merged."""
    path = Path(path)
    if path.exists() and any(path.iterdir()):
        raise PrerequisiteMissing(f"evidence: {path} is not empty")


def require_prerequisites(binary, data, overlay=OVERLAY, which=shutil.which,
                          xlib=require_python_xlib):
    """Section 6: name every missing item instead of installing or skipping."""
    missing = []
    if not Path(binary).is_file():
        missing.append(f"binary: {binary} is not a file")
    if not Path(data).is_dir():
        missing.append(f"data root: {data} is not a directory")
    elif not any(entry.name.casefold() == "data" and entry.is_dir()
                 for entry in Path(data).iterdir()):
        missing.append(f"data root: {data} contains no Data/ (case-insensitive)")
    if not Path(overlay).is_dir():
        missing.append(f"core overlay: {overlay} is not staged in this checkout")
    for tool in ("unshare", "ip", "Xvfb"):
        if which(tool) is None:
            missing.append(f"executable: {tool} is not on PATH")
    try:
        supervised.require_supervisor()
    except supervised.SupervisionUnavailable as error:
        missing.append(str(error))
    try:
        xlib()
    except RuntimeError as error:
        missing.append(f"python-xlib: {error}")
    return missing


def install_facts(account_data):
    """Section 4: the `data_dir()/robin_hood` root a run must leave untouched.

    The account's data directory is a run-private one (`XDG_DATA_HOME`), not the
    real account's: a module that still resolves per-install state through
    `dirs::data_dir()` then writes where the runner can see it, instead of
    polluting the account and racing another agent's session on the same host.
    """
    return Path(account_data) / "robin_hood"


def snapshot_facts(path):
    """What a directory holds, without reading its contents."""
    if not path.exists():
        return {"path": str(path), "exists": False}
    entries = sorted(
        (entry.name, entry.stat().st_mtime_ns, entry.stat().st_size)
        for entry in path.iterdir()
    )
    return {"path": str(path), "exists": True, "entries": entries}


# ── Process plumbing ─────────────────────────────────────────────────────


def free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def request(port, path, body=None, timeout=5):
    payload = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=payload,
                                 headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        raw = error.read()
        try:
            return json.loads(raw)
        except json.JSONDecodeError:
            raise RuntimeError(f"HTTP {error.code} from {path}: {raw[:400]!r}") from error


# `queue_terminal_modal_dismissal` answers a step with this exact refusal when
# the seat owns no terminal page: the debriefing was already answered (by this
# seat, or by the host's decision on its copy) and the mission-end leaderboard
# is now the surface. It is the one refusal a resolve step treats as an answer.
TERMINAL_PAGE_ABSENT = (
    "blocked by mission-end leaderboard; dismiss it in the game before stepping")


def dismissal_has_no_page(reply):
    """Whether a step was refused because the seat has no terminal page left."""
    return isinstance(reply, dict) and reply.get("error") == TERMINAL_PAGE_ABSENT


class Transcript:
    """`events.jsonl`, mirrored to stdout for an operator watching the run."""

    def __init__(self, path, started):
        self.started = started
        self.handle = path.open("w", buffering=1)

    def event(self, kind, **fields):
        record = {"elapsed_s": round(time.monotonic() - self.started, 3), "event": kind, **fields}
        self.handle.write(json.dumps(record) + "\n")
        print(json.dumps(record), flush=True)

    def close(self):
        self.handle.close()


class Peer:
    """One production process with its own install home, HTTP port and log."""

    def __init__(self, spec, name, scenario):
        self.seat = spec["seat"]
        self.role = spec["role"]
        # One X server per peer: a window is how a seat presses a key, and two
        # windows on one display cannot say whose key it was.
        self.xvfb, self.display = start_display(scenario.run, scenario.id)
        self.start = spec.get("start", "launch")
        self.name = name
        self.scenario = scenario
        self.run = scenario.run
        self.install = self.run / "installs" / name
        self.cwd = self.run / "peers" / name / "cwd"
        self.log = self.run / "logs" / f"{name}.log"
        self.logs = [self.log]
        self.samples = self.run / "samples" / f"{name}.jsonl"
        self.process = None
        self.port = free_port()
        self.launches = 0
        for directory in (self.install, self.cwd, self.log.parent, self.samples.parent):
            directory.mkdir(parents=True, exist_ok=True)
        shutil.copytree(OVERLAY, self.cwd / "assets" / "core-datadir", dirs_exist_ok=True)
        (self.install / "mods").mkdir(exist_ok=True)

    def argv(self):
        scenario = self.scenario
        argv = [str(scenario.binary), "--no-sound", "--mp-nickname", self.name,
                "--http-server", str(self.port), "--mp-browser-join-links", "false",
                "--mission", scenario.defaults["mission"]]
        if self.role == "host":
            argv += ["--server", "--mp-expected-players",
                     str(scenario.defaults["expected_players"])]
            if scenario.expanded.get("campaign"):
                # A campaign is one host-owned session: the host names it, and a
                # joiner learns it from the lobby listing, never from its own
                # flags, so a client needs no local campaign save.
                argv += ["--mp-campaign"]
        else:
            argv += ["--connect", json.dumps(scenario.connect)]
        return argv

    def launch(self):
        env = os.environ.copy()
        for key in ("ROBINHOOD_SAVE_DIR", "ROBINHOOD_OVERLAY_DATA_DIRS", "ROBIN_WAIT_FOR_COMMAND",
                    "WAYLAND_DISPLAY"):
            env.pop(key, None)
        env.update(
            XDG_DATA_HOME=str(self.scenario.account_data),
            XDG_CONFIG_HOME=str(self.run / "account-config"),
            XDG_CACHE_HOME=str(self.run / "account-cache"),
            ROBINHOOD_DATA_DIR=str(self.scenario.data),
            ROBINHOOD_INSTALL_HOME=str(self.install),
            ROBINHOOD_MODS_DIR=str(self.install / "mods"),
            DISPLAY=self.display,
            WGPU_BACKEND="gl",
            RUST_LOG="info,robin_rs::game_session=debug,robin_rs::multiplayer=debug",
        )
        self.launches += 1
        argv = self.argv()
        # A cancelled agent kills its own process group, not a detached child's
        # new session. The transient unit is the real owner: its TTL bounds the
        # host even when nobody remains to call `signal()` or the run's finally.
        unit = supervised.unit_name(f"{self.scenario.id}-{self.name}-{self.launches}")
        budget = int(self.scenario.expanded.get("budget_s", DEFAULT_BUDGET_S)) + 30
        wrapped = supervised.command(argv, unit=unit, budget_s=budget, env=env)
        with self.log.open("a") as handle:
            self.process = subprocess.Popen(wrapped, cwd=self.cwd, env=env,
                                            start_new_session=True, stdout=handle,
                                            stderr=subprocess.STDOUT)
        # `argv` in the evidence is the game's argv; the unit is recorded beside
        # it so a stopped host names the run it belonged to.
        self.process.unit = unit
        self.process.name = self.name
        self.scenario.events.event("launch", peer=self.name, seat=self.seat,
                                   pid=self.process.pid, port=self.port, argv=argv,
                                   install=str(self.install), display=self.display,
                                   launch=self.launches, unit=unit, budget_s=budget)

    def alive(self):
        return self.process is not None and self.process.poll() is None

    def exit_status(self):
        return None if self.process is None else self.process.poll()

    def text(self):
        """Every launch of this seat, in launch order.

        A `restart` with a fresh identity writes a new log file, but the seat is
        the same seat: an expectation about what "this peer" said must see the
        process it was before the restart too.
        """
        return "".join(ANSI.sub("", path.read_text(errors="replace")) for path in self.logs)

    def size(self):
        return self.log.stat().st_size if self.log.exists() else 0

    def signal(self, signum, why):
        if not self.alive():
            self.scenario.events.event("signal_skipped", peer=self.name, signal=signum,
                                       why=why, returncode=self.exit_status())
            return
        # Stop the unit first: killing only the wrapper's process group leaves
        # the service running until its TTL, which is a hang on a healthy run.
        unit = getattr(self.process, "unit", None)
        failures = supervised.stop(unit) if unit else []
        os.killpg(self.process.pid, signum)
        self.scenario.events.event("signal", peer=self.name, signal=signum, why=why,
                                   pid=self.process.pid, unit=unit, cleanup_failures=failures)


class ScenarioRun:
    """One expanded scenario: its peers, its faults, its clock and its trace."""

    def __init__(self, identifier, expanded, binary, data, run, events):
        self.id = identifier
        self.expanded = expanded
        self.binary = binary
        self.data = data
        self.run = run
        self.events = events
        self.defaults = expanded["defaults"]
        self.trace = []
        self.latest = {}
        self.account_data = run / "account-data"
        self.connect = None
        self.peers = []
        self.faults = list(expanded["faults"])
        self.resolves = [dict(step) for step in expanded.get("resolve", [])]
        self.fired = []
        self.last_fault_at = time.monotonic()
        self.activity = {}
        self.last_change = time.monotonic()
        self.last_frame = None
        self.last_presented = time.monotonic()
        self.dismissals = 0
        self.post_terminal_dismissals = 0
        self.terminal_page_seen = False
        self.pages_seen = set()

    # ── observation ────────────────────────────────────────────────────

    def sample(self, peer, state):
        record = {
            "peer": peer.seat,
            "name": peer.name,
            "frame": state.get("frame"),
            "map": state.get("map"),
            "multiplayer": state.get("multiplayer"),
            "presentation": state.get("presentation"),
        }
        with peer.samples.open("a") as handle:
            handle.write(json.dumps({"elapsed_s": round(time.monotonic() - self.events.started, 3),
                                     **record}) + "\n")
        if record["presentation"]:
            self.terminal_page_seen = True
            self.pages_seen.add(peer.seat)
        self.trace.append(({"seat": peer.seat, "role": peer.role}, record))
        self.latest[peer.seat] = record

    def poll(self):
        for peer in self.peers:
            if not peer.alive():
                continue
            try:
                state = request(peer.port, "/state", timeout=2)
            except (OSError, ValueError, RuntimeError, urllib.error.URLError):
                continue
            if isinstance(state, dict) and "frame" in state:
                self.sample(peer, state)
        self.seconds_since_change()
        self.keep_moving()
        self.resolve()

    def keep_moving(self):
        """Dismiss the in-game surface so the session's frame can advance.

        A running co-op session holds its frame for a story scroll until every
        seat acknowledges it, so an unattended pair sits at frame 0 forever
        (`multiplayer: announced story modal opening ... kind=PopupText`). Section
        3 gives this layer the graphical surface: the driver presses Return on
        every real Robin window, retrying at a bounded rate and recording each
        attempt. Nothing here steps the timeline; a session that still does not
        move is a hang, not something the driver papers over.
        """
        for peer in self.peers:
            # A presented terminal page is a decision to make, not a story
            # scroll: Return would answer it with the default and steal the
            # choice the row is about.
            if (self.latest.get(peer.seat) or {}).get("presentation"):
                self.last_presented = time.monotonic()
                return
        if self.terminal_page_seen and self.post_terminal_dismissals >= 2:
            # The page after the debriefing is the mission-end leaderboard's
            # notice, which needs an OK to release the session. Two attempts
            # cover it on both seats; past that a further Return would be input
            # on the campaign map the row is there to watch reopen.
            return
        multiplayer = (self.latest.get(0) or {}).get("multiplayer") or {}
        if multiplayer.get("admission") not in RUNNING_PHASES:
            return
        frame = (self.latest.get(0) or {}).get("frame")
        if frame != self.last_frame:
            self.last_frame = frame
            self.last_presented = time.monotonic()
            return
        if time.monotonic() - self.last_presented < SURFACE_RETRY_S:
            return
        if self.dismissals >= MAX_DISMISSALS:
            if self.dismissals == MAX_DISMISSALS:
                self.dismissals += 1
                self.events.event("surface_stalled", frame=frame,
                                  attempts=MAX_DISMISSALS,
                                  detail="no frame advance after dismissing every surface")
            return
        self.dismissals += 1
        if self.terminal_page_seen:
            self.post_terminal_dismissals += 1
        self.last_presented = time.monotonic()
        windows = []
        for peer in self.peers:
            # Each seat has its own display, so the surface is dismissed on the
            # one that is showing it.
            if not peer.alive():
                continue
            try:
                windows += input_worker.dismiss_briefings(peer.display, self.run)
            except RuntimeError as error:
                self.events.event("surface_dismissal_failed", attempt=self.dismissals,
                                  peer=peer.name, error=str(error))
        self.events.event("surface_dismissed", attempt=self.dismissals, frame=frame,
                          windows=windows)

    def booting(self):
        """Whether a live peer has not yet published a frame (see `still_booting`)."""
        return still_booting(self.latest, [peer.seat for peer in self.peers if peer.alive()])

    def seconds_since_change(self):
        """Seconds since any live peer last advanced its frame or admission phase."""
        changed = False
        for peer in self.peers:
            if not peer.alive():
                continue
            state = self.latest.get(peer.seat) or {}
            marker = (state.get("frame"), (state.get("multiplayer") or {}).get("admission"))
            if self.activity.get(peer.seat) != marker:
                self.activity[peer.seat] = marker
                changed = True
        if changed:
            self.last_change = time.monotonic()
        return time.monotonic() - self.last_change

    def record(self):
        return {
            "texts": {peer.seat: peer.text() for peer in self.peers},
            "peers": [{"seat": peer.seat, "role": peer.role} for peer in self.peers],
            "trace": self.trace,
            "statuses": {peer.seat: peer.exit_status() for peer in self.peers},
            "latest": self.latest,
        }

    # ── waits ──────────────────────────────────────────────────────────

    def wait(self, description, predicate, seconds=BOOT_S):
        """Poll until `predicate`, bounded by this layer's deadline (section 5).

        Returns the predicate's value, so a wait that observes something (a
        connect string, an endpoint) hands it to the caller instead of re-reading
        state that may have moved.
        """
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            self.poll()
            self.dispatch_faults()
            value = predicate()
            if value:
                self.events.event("observed", description=description)
                return value
            time.sleep(0.1)
        raise ScenarioDeadline(f"timed out after {seconds:g}s waiting for {description}")

    def settle(self):
        """Wait until the scenario's promises can be judged, or it is a hang.

        Three rules meet here. The timeline comes first: while a fault is still
        due, the session is not over. Then every row is read `SETTLE_S` after its
        last fault, so the killed or refused peer has had its window even when a
        promise already holds — a negative promise ("the host released no
        BeginSim") is only worth reading after the reaction it denies has had
        time to happen. Only then are positive expectations waited for; and while
        one of them is an `exit`, the session is never called settled for being
        quiet, because a peer whose drop opened its reconnect budget sits silent
        for that budget before it publishes the fatal (section 1's bounded
        reconnect). Every other unmet promise is a failed assertion (exit 1) once
        nothing moves: the deadline only fires while the session is still moving
        and a barrier never releases. A peer that has published no frame yet is
        still building its mission, so its silence is not the session's and the
        quiet rule waits for it.
        """
        positives = [
            expectation for expectation in self.expanded["expect"]
            if expectation["assert"] in POSITIVE_ASSERTIONS
            and expectation.get("state") != "absent"
            and not (expectation["assert"] == "exit"
                     and expectation["status"] == EXIT_RUNNING)
        ]
        exit_pending = any(expectation["assert"] == "exit" for expectation in positives)
        # A row may promise a state the game reaches on its own schedule — the
        # modal acknowledgement budget is five minutes — so the scenario's own
        # `settle_deadline_s` extends this wait and nothing else does.
        deadline = time.monotonic() + max(SESSION_DEADLINE_S,
                                          self.expanded.get("settle_deadline_s", 0))
        while time.monotonic() < deadline:
            self.poll()
            self.dispatch_faults()
            if self.faults or self.resolves:
                time.sleep(0.1)
                continue
            if time.monotonic() - self.last_fault_at < SETTLE_S:
                time.sleep(0.1)
                continue
            if positives and all(self.holds(item) for item in positives):
                self.events.event("settled", description="every positive expectation observed")
                return
            if not exit_pending and not self.booting() and \
                    self.seconds_since_change() >= QUIET_S:
                self.events.event("settled", description="no peer is moving",
                                  quiet_s=QUIET_S)
                return
            time.sleep(0.1)
        raise ScenarioDeadline(
            f"timed out after {max(SESSION_DEADLINE_S, self.expanded.get('settle_deadline_s', 0))}s "
            "settling the session; "
            f"{len(positives) - sum(self.holds(item) for item in positives)} "
            "positive expectation(s) never became observable"
        )

    def holds(self, expectation):
        ok, _, _ = evaluate_expectation(expectation, self.record())
        return ok

    # ── faults ─────────────────────────────────────────────────────────

    def due(self, fault):
        at = fault["at"]
        if "frame" in at:
            # The session's frame counter, which is the host's: `join_at_frame`
            # schedules a join against the same timeline, and a peer's own
            # counter is meaningless before it has been admitted.
            state = self.latest.get(0) or {}
            return state.get("frame") is not None and state["frame"] >= at["frame"]
        # A barrier trigger counts the barrier's observations across the session:
        # the barrier is a session event, and a client-side barrier never appears
        # on the host's own samples. A frame trigger reads the host's counter.
        seen = len(barrier_samples(at["barrier"], "reached", self.trace))
        return seen > at["after"]

    def dispatch_faults(self):
        for fault in list(self.faults):
            if not any(peer.seat == fault["peer"] for peer in self.peers):
                continue
            if not self.due(fault):
                continue
            self.faults.remove(fault)
            self.apply(fault)

    def peer(self, seat):
        return next(peer for peer in self.peers if peer.seat == seat)

    def apply(self, fault):
        peer = self.peer(fault["peer"])
        verb = fault["fault"]
        self.fired.append(fault)
        self.last_fault_at = time.monotonic()
        self.events.event("fault", peer=peer.name, fault=verb, at=fault["at"])
        if verb == "kill":
            peer.signal(signal.SIGKILL, "corpus kill")
        elif verb == "stop":
            peer.signal(signal.SIGSTOP, "corpus stop")
        elif verb == "continue":
            peer.signal(signal.SIGCONT, "corpus continue")
        elif verb == "restart":
            self.restart(peer, fault)
        elif verb == "mission-result":
            self.end_mission(peer, fault)
        elif verb == "console":
            self.post(peer, "console", "/console",
                      {"command": fault["command"]}, f"console {fault['command']}")
        elif verb == "command":
            self.post(peer, "command", "/command", fault["command"], "player_command")
        elif verb == "native":
            self.post(peer, "native", "/native",
                      {"op": fault["op"], "args": fault["args"]}, "native")
        elif verb == "key":
            input_worker.client_key(peer.display, fault["key"], self.run)
            self.events.event("key", peer=peer.name, key=fault["key"])
        else:
            raise Unprovable(f"fault verb `{verb}` is not implemented at the process layer")

    def post(self, peer, name, path, body, description):
        """One dev-surface request, so a refusal names itself instead of timing out."""
        reply = self.answer(peer, path, body, description)
        if not isinstance(reply, dict) or "error" in reply:
            raise RuntimeError(f"{peer.name}: {path} refused {description}: {reply}")
        self.events.event(name, peer=peer.name, request=body, reply=reply)
        return reply

    def answer(self, peer, path, body, description):
        """Send one request and return its reply, refusal included.

        A caller that can act on a specific refusal reads it here; every other
        caller goes through `post`, where a refusal fails the row.
        """
        try:
            return request(peer.port, path, body)
        except (OSError, ValueError, RuntimeError, urllib.error.URLError) as error:
            raise RuntimeError(f"{peer.name}: {path} {description} failed: {error}") from error

    def end_mission(self, peer, fault):
        """The host ends the mission through its own terminal flow."""
        self.post(peer, "mission_result", "/mission-result", {"result": fault["result"]},
                  f"mission-result {fault['result']}")

    def resolve(self):
        """Press the debriefing button whose page a seat is actually showing.

        `/state.presentation` publishes the exact `ModalKind` the terminal flow
        validates a dismissal against, so the runner never invents the text
        index. The dismissal travels on the step endpoint, which is the one
        surface that queues a terminal decision — a client's is forwarded to the
        host, exactly as the keyboard would forward it.

        A press that loses the race to the session's own decision is not a
        failed row: the page it names is already gone, the step's promise is
        answered, and the evidence says so.

        The list is a sequence, not a set: a step whose seat has no page yet
        holds every later one. Rows depend on that — the host's decision ends
        the page on every seat, so a client's answer only exists if it is
        pressed while the page is still the session's open question.
        """
        if not self.resolves:
            return
        waiting = []
        for step in list(self.resolves):
            if not any(peer.seat == step["peer"] for peer in self.peers):
                continue
            peer = self.peer(step["peer"])
            presentation = (self.latest.get(peer.seat) or {}).get("presentation")
            if presentation and peer.alive():
                body = {
                    "n": 1,
                    "auto_dismiss": False,
                    "synchronized_multiplayer": True,
                    "dismissals": [{"kind": presentation["kind"], "result": step["result"]}],
                }
                description = f"terminal dismissal {step['result']}"
                reply = self.answer(peer, "/step-forward", body, description)
                if dismissal_has_no_page(reply):
                    self.events.event("resolve_answered_by_session", peer=peer.name,
                                      why="the decision closed the page before the press landed",
                                      reply=reply)
                    self.resolves.remove(step)
                    self.last_fault_at = time.monotonic()
                    continue
                if not isinstance(reply, dict) or "error" in reply:
                    raise RuntimeError(f"{peer.name}: /step-forward refused {description}: {reply}")
                self.events.event("resolve", peer=peer.name, request=body, reply=reply)
                self.resolves.remove(step)
                self.last_fault_at = time.monotonic()
                continue
            if peer.seat in self.pages_seen:
                # The session answered this seat's page before the runner could
                # press it (the host's decision closes every seat's copy); that
                # is the same promise, and pressing a page that is gone is not.
                self.events.event("resolve_answered_by_session", peer=peer.name,
                                  why="the page was already gone at the next sample")
                self.resolves.remove(step)
                continue
            waiting.append(peer.name)
            break
        if waiting and not self.faults and (
                time.monotonic() - self.last_fault_at > BARRIER_DEADLINE_S):
            raise ScenarioDeadline(
                "no page appeared for " + ", ".join(waiting)
                + f" within {BARRIER_DEADLINE_S}s of the last fault, so it cannot be resolved")

    def restart(self, peer, fault):
        """`SIGKILL`, then relaunch as a new process (section 2's process meaning).

        `identity: same` keeps the install home, so the transport identity key
        inside it is the same owner and the seat can be reclaimed; `fresh` gives
        the replacement a new install home, a new identity and no seat.
        """
        peer.signal(signal.SIGKILL, "corpus restart")
        try:
            peer.process.wait(timeout=10)
        except subprocess.TimeoutExpired as error:
            raise ScenarioDeadline(f"{peer.name} survived SIGKILL") from error
        self.events.event("peer_stopped", peer=peer.name, returncode=peer.exit_status())
        if fault["identity"] == "fresh":
            suffix = f"-fresh{peer.launches}"
            for path in (self.run / "installs" / (peer.name + suffix),
                         self.run / "peers" / (peer.name + suffix) / "cwd"):
                path.mkdir(parents=True, exist_ok=True)
            shutil.copytree(OVERLAY, self.run / "peers" / (peer.name + suffix)
                            / "cwd" / "assets" / "core-datadir", dirs_exist_ok=True)
            (self.run / "installs" / (peer.name + suffix) / "mods").mkdir(exist_ok=True)
            peer.install = self.run / "installs" / (peer.name + suffix)
            peer.cwd = self.run / "peers" / (peer.name + suffix) / "cwd"
            peer.name = peer.name + suffix
            peer.log = self.run / "logs" / f"{peer.name}.log"
            peer.logs.append(peer.log)
            peer.samples = self.run / "samples" / f"{peer.name}.jsonl"
        self.activity.pop(peer.seat, None)
        peer.launch()

    # ── the scenario script ────────────────────────────────────────────

    def start(self):
        host_spec = next(item for item in self.expanded["peers"] if item["role"] == "host")
        host = Peer(host_spec, "host", self)
        self.peers.append(host)
        host.launch()
        self.connect = self.wait("host publishes its connect string", self.connect_string,
                                 seconds=BOOT_S)
        (self.run / "connect.json").write_text(json.dumps(self.connect, indent=2) + "\n")
        self.events.event("connect_string", value=self.connect)
        for spec in self.expanded["peers"]:
            if spec["role"] == "host":
                continue
            self.join(spec)
        if self.faults:
            self.wait("every fault's trigger", lambda: not self.faults,
                      seconds=BARRIER_DEADLINE_S + BOOT_S)
        self.settle()

    def connect_string(self):
        multiplayer = (self.latest.get(0) or {}).get("multiplayer") or {}
        value = multiplayer.get("connect_string")
        return json.loads(value) if value else None

    def join(self, spec):
        start = spec.get("start", "launch")
        if "join_at_frame" in start:
            self.wait(f"host reaches frame {start['join_at_frame']} before the join",
                      lambda: ((self.latest.get(0) or {}).get("frame") or 0)
                      >= start["join_at_frame"])
        elif "join_at_barrier" in start:
            name = start["join_at_barrier"]
            self.wait(f"barrier `{name}` before the join",
                      lambda: bool(barrier_samples(name, "reached", self.trace)))
        peer = Peer(spec, f"peer{spec['seat']}", self)
        self.peers.append(peer)
        peer.launch()

    # ── judgement ──────────────────────────────────────────────────────

    def judge(self):
        record = self.record()
        results, failures = [], []
        for expectation in self.expanded["expect"]:
            try:
                ok, detail, observed = evaluate_expectation(expectation, record)
            except Unprovable as error:
                ok, detail, observed = False, str(error), None
            results.append({"assert": expectation["assert"], "expectation": expectation,
                            "ok": ok, "detail": detail, "observed": observed})
            if not ok:
                failures.append(f"{self.id}: {detail}")
        return results, failures


# ── orchestration ────────────────────────────────────────────────────────


def loopback_only():
    interfaces = json.loads(subprocess.check_output(["ip", "-j", "link", "show"], timeout=10))
    return sorted(item["ifname"] for item in interfaces) == ["lo"]


def confine(argv):
    """Re-exec inside a loopback-only user/network namespace (section 5)."""
    if loopback_only():
        return
    if shutil.which("unshare") is None:
        raise PrerequisiteMissing("executable: unshare is not on PATH")
    os.execvp("unshare", ["unshare", "--user", "--map-root-user", "--net",
                          sys.executable, str(Path(__file__).resolve()), *argv])


def start_display(run, scope):
    # Xvfb used to publish its display over an inherited `-displayfd` pipe. A
    # systemd service cannot inherit that pipe, so allocate a display number
    # under a lock and wait for the socket instead. The lock is held only while
    # choosing a number — never while starting or running the server — so two
    # agents cannot pick the same display and cannot deadlock on each other.
    display_lock = Path(tempfile.gettempdir()) / "robin-xvfb-display.lock"
    with display_lock.open("a+") as lock:
        import fcntl
        fcntl.flock(lock, fcntl.LOCK_EX)
        used = {p.name[1:] for p in Path("/tmp/.X11-unix").glob("X*")} if Path("/tmp/.X11-unix").exists() else set()
        number = next((n for n in range(100, 300) if str(n) not in used), None)
        if number is None:
            raise PrerequisiteMissing("X server: no display number left between 100 and 299")
        lock.write(f"{number} {os.getpid()}\n")
        lock.flush()
        fcntl.flock(lock, fcntl.LOCK_UN)
    display = f":{number}"
    unit = supervised.unit_name(f"{scope}-xvfb-{number}")
    env = os.environ.copy()
    argv = supervised.command(
        ["Xvfb", display, "-screen", "0", "1280x1024x24", "-nolisten", "tcp", "-ac"],
        unit=unit, budget_s=1800, env=env,
    )
    with (run / "xvfb.log").open("w") as log:
        xvfb = subprocess.Popen(argv, cwd=run, env=env, start_new_session=True,
                                stdout=log, stderr=subprocess.STDOUT)
    xvfb.unit = unit
    xvfb.name = "xvfb"
    deadline = time.monotonic() + 15
    socket_path = Path("/tmp/.X11-unix") / f"X{number}"
    while time.monotonic() < deadline:
        if socket_path.exists():
            return xvfb, display
        if xvfb.poll() is not None:
            break
        time.sleep(0.1)
    supervised.stop(unit)
    raise PrerequisiteMissing(f"X server: {display} never became ready; see {run / 'xvfb.log'}")


def run_scenario(identifier, expanded, options, evidence):
    run = run_directory(evidence, identifier)
    # Section 5: the expanded scenario is evidence, not something the reader
    # must reconstruct from `extends`.
    (run / "scenario.json").write_text(json.dumps(expanded, indent=2) + "\n")
    (run / "binary.sha256").write_text(digest(options.binary) + "\n")
    events = Transcript(run / "events.jsonl", time.monotonic())
    summary = {
        "id": identifier,
        "layer": "process",
        "chain": expanded["chain"],
        "requires": expanded.get("requires", []),
        "snapshot": options.snapshot,
        "binary_sha256": digest(options.binary),
        "data": str(options.data),
        "verdict": None,
        "checks": [],
        "failures": [],
    }
    children, scenario = [], None
    try:
        # Section 4: the run owns the account's data directory, so a module that
        # still resolves per-install state through `dirs::data_dir()` writes into
        # this run instead of the account.
        for directory in (run / "account-data", run / "account-config", run / "account-cache"):
            directory.mkdir(exist_ok=True)
        installed = install_facts(run / "account-data")
        before = snapshot_facts(installed)
        scenario = ScenarioRun(identifier, expanded, options.binary, options.data, run,
                               events)

        def interrupted(signum, _frame):
            raise DriverInterrupted(f"scenario interrupted by signal {signum}")

        signal.signal(signal.SIGINT, interrupted)
        signal.signal(signal.SIGTERM, interrupted)
        signal.signal(signal.SIGALRM, interrupted)
        signal.alarm(int(expanded.get("budget_s", DEFAULT_BUDGET_S)))
        verify_client(options.binary, {"binary_sha256": summary["binary_sha256"]})
        scenario.start()
        results, failures = scenario.judge()
        summary["checks"] = results
        summary["failures"] = failures
        summary["trace_samples"] = len(scenario.trace)
        summary["faults_fired"] = scenario.fired
        summary["peers"] = peer_summaries(scenario)
        after = snapshot_facts(installed)
        summary["install_isolation"] = {"before": before, "after": after,
                                        "unchanged": before == after}
        if before != after:
            summary["failures"].append(
                f"{identifier}: per-install state changed outside ROBINHOOD_INSTALL_HOME: {installed}"
            )
        summary["verdict"] = "observed" if not summary["failures"] else "failed"
    except DriverInterrupted:
        raise
    except ScenarioDeadline as error:
        summary["verdict"] = "hang"
        summary["failures"].append(f"{identifier}: {error}")
    except Unprovable as error:
        summary["verdict"] = "unprovable"
        summary["failures"].append(f"{identifier}: {error}")
    except Exception as error:
        summary["verdict"] = "failed"
        summary["failures"].append(f"{identifier}: {type(error).__name__}: {error}")
        summary["traceback"] = traceback.format_exc()
    finally:
        signal.alarm(0)
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        if scenario is not None:
            summary.setdefault("peers", peer_summaries(scenario))
        (run / "hashes.json").write_text(json.dumps(hash_manifest(scenario), indent=2) + "\n")
        summary["elapsed_s"] = round(time.monotonic() - events.started, 3)
        servers = [peer.xvfb for peer in scenario.peers] if scenario is not None else []
        supervised.cleanup(children + servers, summary, event=events.event)
        (run / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        events.event("summary", id=identifier, verdict=summary["verdict"],
                     failures=summary["failures"])
        events.close()
    missing = [name for name in EVIDENCE_FILES if not (run / name).exists()]
    if missing:
        raise RuntimeError(f"{run} is missing section-5 evidence: {missing}")
    return summary, run


def run_one(identifier, *, scenarios, defaults, contract, binary, data, snapshot, evidence):
    """One scenario, with everything it needs spelled out.

    A module-level function with keyword arguments rather than a closure: the
    parallel path pickles this callable, and a closure cannot be pickled.
    """
    expanded = scenario_corpus.expanded(identifier, scenarios, defaults, contract)
    arguments = argparse.Namespace(binary=binary, data=data, snapshot=snapshot)
    return run_scenario(identifier, expanded, arguments, evidence)


def run_selected(selected, run, jobs):
    """Run every selected scenario, up to `jobs` at a time, in selection order.

    A scenario owns its display, its install-isolated data directory and its
    ports, so two of them do not collide. They do share the host's cores, which
    is why `jobs` is the caller's decision and not this function's: the gate
    runs one at a time because its faults fire at frame N, and a starved client
    reaches frame N late.

    Processes, not threads: a scenario installs signal handlers, and Python only
    allows that on a process's main thread. `fork` keeps the child's `__main__`
    — and therefore the callable being pickled — identical to the parent's.
    """
    if jobs <= 1 or len(selected) <= 1:
        return [run(identifier) for identifier in selected]
    context = multiprocessing.get_context("fork")
    with concurrent.futures.ProcessPoolExecutor(max_workers=jobs,
                                                mp_context=context) as pool:
        futures = [pool.submit(run, identifier) for identifier in selected]
        return [future.result() for future in futures]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", type=Path, required=True,
                        help="pinned robin executable built with the multiplayer release features")
    parser.add_argument("--data", type=Path, required=True,
                        help="game-data root containing Data/ (ROBINHOOD_DATA_DIR)")
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--snapshot", required=True,
                        help="source revision the supplied executable was built from")
    parser.add_argument("--scenarios", type=Path, default=CORPUS)
    parser.add_argument("--only", action="append", metavar="ID", default=None,
                        help="run one selected scenario (operator debugging; the gate runs all)")
    parser.add_argument("--jobs", type=int, default=1, metavar="N",
                        help="run up to N scenarios at once (default 1). A scenario is "
                             "frame-timed and shares the host's cores, so raise this only "
                             "after measuring the whole corpus on the host.")
    options = parser.parse_args(argv)
    if options.jobs < 1:
        parser.error(f"--jobs takes a positive count, not {options.jobs}")
    raw = list(sys.argv[1:]) if argv is None else list(argv)

    missing = require_prerequisites(options.binary, options.data)
    if missing:
        for item in missing:
            print(f"multiplayer-e2e: prerequisite missing: {item}", file=sys.stderr)
        return 3
    confine(raw)

    evidence = options.evidence.resolve()
    try:
        require_empty(evidence)
    except PrerequisiteMissing as error:
        print(f"multiplayer-e2e: {error}", file=sys.stderr)
        return 3
    evidence.mkdir(parents=True, exist_ok=True)
    try:
        contract = scenario_corpus.load_contract()
        defaults, scenarios = scenario_corpus.load_corpus(options.scenarios)
    except scenario_corpus.ContractError as error:
        print(f"multiplayer-e2e: {error}", file=sys.stderr)
        return 3
    problems = scenario_corpus.validate(defaults, scenarios, contract)
    if problems:
        for problem in problems:
            print(f"multiplayer-e2e: corpus: {problem}", file=sys.stderr)
        return 3
    selected, skipped, refused = select(scenarios)
    promised = sorted(identifier for identifier in scenarios
                      if proves_layer(scenarios, identifier) == PROCESS_LAYER)
    if sorted(set(selected) | {item["id"] for item in refused}) != promised:
        print("multiplayer-e2e: a process row has no outcome in this suite: "
              f"{sorted(set(promised) ^ (set(selected) | {item['id'] for item in refused}))}",
              file=sys.stderr)
        return 3
    if options.only:
        unknown = sorted(set(options.only) - set(selected))
        if unknown:
            print(f"multiplayer-e2e: --only {unknown}: not selected at the process layer",
                  file=sys.stderr)
            return 3
        selected = [identifier for identifier in selected if identifier in set(options.only)]

    suite = {"suite": "multiplayer-e2e", "layer": "process",
             "snapshot": options.snapshot, "selected": selected, "skipped": skipped,
             "refused": refused, "scenarios": {}, "verdicts": {}}
    started = time.monotonic()
    try:
        binary = snapshot_client(options.binary.resolve(strict=True), evidence, suite)
        (evidence / "binary.sha256").write_text(suite["binary_sha256"] + "\n")
        subprocess.run(["ip", "link", "set", "lo", "up"], check=True, timeout=10)
        run = functools.partial(run_one, scenarios=scenarios, defaults=defaults,
                                contract=contract, binary=binary,
                                data=options.data.resolve(),
                                snapshot=options.snapshot, evidence=evidence)
        for identifier, (result, run_dir) in zip(selected,
                                                 run_selected(selected, run, options.jobs)):
            suite["scenarios"][identifier] = {"verdict": result["verdict"],
                                              "failures": result["failures"],
                                              "evidence": str(run_dir)}
            suite["verdicts"][identifier] = result["verdict"]
    except PrerequisiteMissing as error:
        suite["prerequisites_missing"] = [str(error)]
        print(f"multiplayer-e2e: prerequisite missing: {error}", file=sys.stderr)
    except DriverInterrupted as error:
        suite["interrupted"] = str(error)
    except Exception as error:
        suite["error"] = f"{type(error).__name__}: {error}"
        suite["traceback"] = traceback.format_exc()

    hangs = sorted(identifier for identifier, document in suite["scenarios"].items()
                   if document["verdict"] == "hang")
    suite["harness_gaps"] = [item["id"] for item in refused]
    suite["verdict"] = suite_verdict(bool(suite.get("prerequisites_missing")),
                                     bool(suite.get("interrupted")), hangs, refused,
                                     suite["verdicts"])
    suite["elapsed_s"] = round(time.monotonic() - started, 3)
    (evidence / "summary.json").write_text(json.dumps(suite, indent=2) + "\n")
    print(json.dumps({"verdict": suite["verdict"], "verdicts": suite["verdicts"],
                      "refused": [item["id"] for item in refused],
                      "skipped": [item["id"] for item in skipped]}, indent=2), flush=True)

    return suite_exit_code(suite["verdict"])


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
