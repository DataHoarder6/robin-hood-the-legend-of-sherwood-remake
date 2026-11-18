#!/usr/bin/env python3
"""Gated process-layer multiplayer end-to-end suite (issue #6).

The contract is `docs/multiplayer/HARNESS.md`: section 2 owns the scenario
schema and the fault vocabulary, section 3 owns which layer runs a scenario,
section 5 owns this layer's execution site, evidence layout and exit contract,
section 6 owns the bootstrap. Section 1's corpus table is the coverage floor.

This is section 3's process layer: real `robin` binaries, real signals, real
per-install state, driven and asserted over loopback HTTP. The corpus is loaded
through `scenario_corpus` (one schema, one expansion) and only the scenarios the
contract forces onto this layer run here — a `log` or `exit` assertion, or
`requires: ["graphical"]`. Every other scenario is listed in `summary.json` with
the section-3 reason it stayed below the process layer, so nothing is dropped
silently.

Exit status (section 5):

  0  every `expect` of every selected scenario was observed
  1  an assertion failed, or a selected scenario cannot be expressed at this
     layer; the scenario and the expectation are named in `summary.json`
  2  the deadline was hit: a scenario budget, or a barrier wait bounded by the
     transition-watchdog deadline plus a margin. Exit 2 means a promised state
     was missed, never that the machine was slow
  3  a prerequisite is missing and nothing about the game was proved

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
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import sys
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
# A running session that has not advanced a frame for this long is holding a
# story scroll: the driver presses Return on the real window again.
SURFACE_RETRY_S = 4
MAX_DISMISSALS = 8
DEFAULT_BUDGET_S = 600
BOOT_S = 120

# Section 3: what forces a scenario onto this layer.
PROCESS_ASSERTIONS = frozenset({"log", "exit"})
GRAPHICAL = "graphical"
NETEM = "netem"

# Section 2's process column, and the verbs it cannot express from a scenario
# file: `truncate` is marked unsupported there, while `diverge` ("POST /native
# on that peer alone") and `stall-surface` ("same") need a native op or a
# surface deadline the fault schema does not carry.
PROCESS_FAULTS = frozenset({"kill", "stop", "continue", "restart"})
NETEM_FAULTS = frozenset({"duplicate", "drop", "reorder", "delay"})
UNEXPRESSIBLE_FAULTS = {
    "diverge": "section 2's process meaning is `POST /native` on that peer alone, and a "
               "fault names no native op or arguments to post",
    "truncate": "section 2 marks `truncate` unsupported at the process layer",
    "stall-surface": "a fault names no surface or deadline for the process layer to hold",
}

# Admission phases `/state` reports (.../game_session/runtime/admission.rs
# wire_name). Section 1 names barriers while `/state` names phases, so this is
# the one mapping between them; `modal-ack` has no phase or field at all and is
# therefore unprovable here.
CLIENT_CONTENT_PHASES = frozenset(
    {"peer_waiting_for_snapshot", "peer_waiting_for_begin", "waiting_for_start", "running"}
)
CLIENT_SNAPSHOT_PHASES = frozenset({"peer_waiting_for_begin", "waiting_for_start", "running"})
RUNNING_PHASES = frozenset({"running"})
POSITIVE_ASSERTIONS = frozenset({"exit", "barrier", "final-operation"})

# Section 5 evidence per run. `binary.sha256` is repeated inside each run so a
# single scenario directory is a complete record on its own.
EVIDENCE_FILES = ("scenario.json", "summary.json", "events.jsonl", "binary.sha256")


class PrerequisiteMissing(RuntimeError):
    """Section 6: nothing about the game was proved (exit 3)."""


class ScenarioDeadline(RuntimeError):
    """Section 5: a promised state was missed inside its budget (exit 2)."""


class Unprovable(RuntimeError):
    """A selected expectation has no process-layer observation (exit 1)."""


# ── Contract reading and selection (pure) ────────────────────────────────


def contract_requirements(scenarios, identifier):
    """`requires` of a scenario composed with its `extends` chain."""
    return frozenset(
        item
        for step in scenario_corpus.chain(identifier, scenarios)
        for item in scenarios[step][1].get("requires", [])
    )


def contract_expectations(scenarios, identifier):
    return [
        expectation
        for step in scenario_corpus.chain(identifier, scenarios)
        for expectation in scenarios[step][1].get("expect", [])
    ]


def contract_faults(scenarios, identifier):
    return [
        fault
        for step in scenario_corpus.chain(identifier, scenarios)
        for fault in scenarios[step][1].get("faults", [])
    ]


def unexpressible(faults, requires):
    """Why the process layer cannot express these faults, or `None`."""
    for fault in faults:
        verb = fault["fault"]
        if verb in PROCESS_FAULTS:
            continue
        if verb in NETEM_FAULTS:
            if NETEM not in requires:
                return (
                    f"section 2 gives the process layer `netem {verb}` for `{verb}`, and "
                    f"section 5 requires the scenario to declare `requires: [\"netem\"]`; "
                    f"it declares {sorted(requires)}"
                )
            continue
        if verb in UNEXPRESSIBLE_FAULTS:
            return UNEXPRESSIBLE_FAULTS[verb]
        return f"unknown fault verb `{verb}`"
    return None


def select(scenarios):
    """Split the corpus by the contract's own rule (section 3).

    Returns `(run, skipped, refused)`. Both non-running lists carry the contract
    reason, so a scenario is never dropped silently.
    """
    run, skipped, refused = [], [], []
    for identifier in sorted(scenarios):
        verbs = {item["assert"] for item in contract_expectations(scenarios, identifier)}
        requires = contract_requirements(scenarios, identifier)
        forced = bool(PROCESS_ASSERTIONS & verbs) or GRAPHICAL in requires
        if not forced:
            skipped.append({
                "id": identifier,
                "reason": (
                    "section 3 keeps it below the process layer: no log/exit "
                    f"assertion (found {sorted(verbs)}) and no graphical requirement "
                    f"(requires {sorted(requires)})"
                ),
            })
            continue
        reason = unexpressible(contract_faults(scenarios, identifier), requires)
        if reason is None:
            run.append(identifier)
        else:
            refused.append({"id": identifier, "reason": reason})
    return run, skipped, refused


def exit_code(verdicts):
    """Section 5: 0 only when every selected scenario observed its expectations."""
    return 0 if verdicts and all(verdict == "observed" for verdict in verdicts.values()) else 1


# ── Barrier observations (pure) ──────────────────────────────────────────


def barrier_reached(name, peer, sample):
    """Whether one `/state` sample shows `peer` at barrier `name`."""
    multiplayer = sample.get("multiplayer") or {}
    admission = multiplayer.get("admission")
    if name == "begin-sim":
        return multiplayer.get("begin_sim_frame") is not None
    if name == "content-ready":
        return peer["role"] == "client" and admission in CLIENT_CONTENT_PHASES
    if name == "snapshot-adopt":
        return peer["role"] == "client" and admission in CLIENT_SNAPSHOT_PHASES
    if name == "transition-commit":
        return multiplayer.get("pending_transition") is not None
    if name == "modal-ack":
        raise Unprovable(
            "section 3 grows `multiplayer` with no modal field, so `modal-ack` cannot be "
            "observed through /state and the corpus row is unprovable at this layer"
        )
    raise Unprovable(f"unknown barrier `{name}`")


def barrier_released(name, peer, sample):
    """Whether one sample shows the barrier released (it must be reached too)."""
    if not barrier_reached(name, peer, sample):
        return False
    multiplayer = sample.get("multiplayer") or {}
    if name == "begin-sim":
        return multiplayer.get("admission") in RUNNING_PHASES
    if name == "transition-commit":
        return not (multiplayer.get("pending_transition") or {}).get("awaiting")
    return True


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
    so a failure names who spoke.
    """
    needle = expectation.get("contains") or expectation.get("absent")
    counts = {peer["seat"]: texts.get(peer["seat"], "").count(needle) for peer in peers}
    observed = sum(counts.values())
    wanted = expectation["count"]
    if "contains" in expectation:
        detail = f"`{needle}` appeared {observed} times in the peers' output, expected {wanted}"
        return observed == wanted, detail, counts
    detail = f"`{needle}` appeared {observed} times in the peers' output, expected {wanted}"
    return observed == wanted, detail, counts


def evaluate_barrier(expectation, record):
    name, state = expectation["barrier"], expectation["state"]
    hits = barrier_samples(name, state, record["trace"])
    ok = bool(hits) if state != "absent" else not hits
    detail = f"`{name}` {state}: {len(hits)} of {len(record['trace'])} /state samples matched"
    return ok, detail, hits


def evaluate_exit(expectation, record):
    peers = expectation["peers"]
    nonzero = expectation["status"] == "nonzero"
    observed = {seat: record["statuses"].get(seat) for seat in peers}
    ok = all(status is not None and (status != 0) == nonzero for status in observed.values())
    return ok, f"exit status {expectation['status']} for seats {peers}: {observed}", observed


def evaluate_final_operation(expectation, record):
    observed = {
        peer["seat"]: (((record["latest"].get(peer["seat"]) or {}).get("multiplayer") or {})
                       .get("final_operation"))
        for peer in record["peers"]
    }
    ok = all(value == expectation["operation"] for value in observed.values())
    return ok, f"final operation {expectation['operation']}: {observed}", observed


def evaluate_hash_equal(expectation, record):
    """Section 1's oracle: both peers compared a frame and neither desynced."""
    peers = expectation["peers"]
    observed = {
        seat: {
            "last_compared_hash_frame": (((record["latest"].get(seat) or {})
                                          .get("multiplayer") or {}).get("last_compared_hash_frame")),
            "desync_count": (((record["latest"].get(seat) or {})
                              .get("multiplayer") or {}).get("desync_count")),
        }
        for seat in peers
    }
    ok = all(item["last_compared_hash_frame"] is not None and item["desync_count"] == 0
             for item in observed.values())
    return ok, f"hash agreement for seats {peers}: {observed}", observed


def evaluate_hash_diverged(expectation, record):
    peers = expectation["peers"]
    observed = {
        seat: (((record["latest"].get(seat) or {}).get("multiplayer") or {}).get("desync_count"))
        for seat in peers
    }
    ok = all(count is not None and count >= 1 for count in observed.values())
    return ok, f"hash divergence counted on seats {peers}: {observed}", observed


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
        self.start = spec.get("start", "launch")
        self.name = name
        self.scenario = scenario
        self.run = scenario.run
        self.install = self.run / "installs" / name
        self.cwd = self.run / "peers" / name / "cwd"
        self.log = self.run / "logs" / f"{name}.log"
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
                     str(scenario.defaults["expected_players"]),
                     "--record", str(scenario.record_path)]
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
            DISPLAY=self.scenario.display,
            WGPU_BACKEND="gl",
            RUST_LOG="info,robin_rs::game_session=debug,robin_rs::multiplayer=debug",
        )
        self.launches += 1
        argv = self.argv()
        with self.log.open("a") as handle:
            self.process = subprocess.Popen(argv, cwd=self.cwd, env=env,
                                            start_new_session=True, stdout=handle,
                                            stderr=subprocess.STDOUT)
        self.scenario.events.event("launch", peer=self.name, seat=self.seat,
                                   pid=self.process.pid, port=self.port, argv=argv,
                                   install=str(self.install), launch=self.launches)

    def alive(self):
        return self.process is not None and self.process.poll() is None

    def exit_status(self):
        return None if self.process is None else self.process.poll()

    def text(self):
        return ANSI.sub("", self.log.read_text(errors="replace"))

    def size(self):
        return self.log.stat().st_size if self.log.exists() else 0

    def signal(self, signum, why):
        if not self.alive():
            self.scenario.events.event("signal_skipped", peer=self.name, signal=signum,
                                       why=why, returncode=self.exit_status())
            return
        os.killpg(self.process.pid, signum)
        self.scenario.events.event("signal", peer=self.name, signal=signum, why=why,
                                   pid=self.process.pid)


class ScenarioRun:
    """One expanded scenario: its peers, its faults, its clock and its trace."""

    def __init__(self, identifier, expanded, binary, data, run, display, events):
        self.id = identifier
        self.expanded = expanded
        self.binary = binary
        self.data = data
        self.run = run
        self.display = display
        self.events = events
        self.defaults = expanded["defaults"]
        self.trace = []
        self.latest = {}
        self.account_data = run / "account-data"
        self.record_path = run / "host.rhrec.jsonl"
        self.connect = None
        self.peers = []
        self.faults = list(expanded["faults"])
        self.fired = []
        self.activity = {}
        self.last_change = time.monotonic()
        self.last_frame = None
        self.last_presented = time.monotonic()
        self.dismissals = 0

    # ── observation ────────────────────────────────────────────────────

    def sample(self, peer, state):
        record = {
            "peer": peer.seat,
            "name": peer.name,
            "frame": state.get("frame"),
            "map": state.get("map"),
            "multiplayer": state.get("multiplayer"),
        }
        with peer.samples.open("a") as handle:
            handle.write(json.dumps({"elapsed_s": round(time.monotonic() - self.events.started, 3),
                                     **record}) + "\n")
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
        self.last_presented = time.monotonic()
        try:
            windows = input_worker.dismiss_briefings(self.display, self.run)
        except RuntimeError as error:
            self.events.event("surface_dismissal_failed", attempt=self.dismissals,
                              error=str(error))
            return
        self.events.event("surface_dismissed", attempt=self.dismissals, frame=frame,
                          windows=windows)

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

        Positive expectations (`exit`, a reached barrier, a final operation) are
        waited for. When no peer is moving any more the run is over even if a
        promise was missed, which is an assertion failure (exit 1): an unmet
        promise in a session where nothing moves is a failed assertion, not a
        hang. The deadline only fires while the session is still moving and a
        barrier never releases.
        """
        positives = [
            expectation for expectation in self.expanded["expect"]
            if expectation["assert"] in POSITIVE_ASSERTIONS
            and expectation.get("state") != "absent"
        ]
        deadline = time.monotonic() + BARRIER_DEADLINE_S
        while time.monotonic() < deadline:
            self.poll()
            self.dispatch_faults()
            if positives and all(self.holds(item) for item in positives):
                self.events.event("settled", description="every positive expectation observed")
                return
            if self.seconds_since_change() >= QUIET_S:
                self.events.event("settled", description="no peer is moving", quiet_s=QUIET_S)
                return
            time.sleep(0.1)
        raise ScenarioDeadline(
            f"timed out after {BARRIER_DEADLINE_S}s settling the session; "
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
        self.events.event("fault", peer=peer.name, fault=verb, at=fault["at"])
        if verb == "kill":
            peer.signal(signal.SIGKILL, "corpus kill")
        elif verb == "stop":
            peer.signal(signal.SIGSTOP, "corpus stop")
        elif verb == "continue":
            peer.signal(signal.SIGCONT, "corpus continue")
        elif verb == "restart":
            self.restart(peer, fault)
        else:
            raise Unprovable(f"fault verb `{verb}` is not implemented at the process layer")

    def restart(self, peer, fault):
        """SIGKILL, then relaunch as a new process (section 2's process meaning).

        `identity: same` keeps the install home, so the transport identity key
        inside it is the same owner and the seat can be reclaimed; `fresh` gives
        the replacement a new install home, a new identity and no seat.
        """
        stop_process_group(peer.process)
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


def start_display(run):
    read_fd, write_fd = os.pipe()
    with (run / "xvfb.log").open("w") as log:
        xvfb = subprocess.Popen(["Xvfb", "-displayfd", str(write_fd), "-screen", "0",
                                 "1280x1024x24", "-nolisten", "tcp", "-ac"],
                                start_new_session=True, pass_fds=(write_fd,),
                                stdout=log, stderr=subprocess.STDOUT)
    os.close(write_fd)
    with os.fdopen(read_fd) as handle:
        display = ":" + handle.readline().strip()
    if not display[1:]:
        raise PrerequisiteMissing("X server: Xvfb published no display")
    return xvfb, display


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
        xvfb, display = start_display(run)
        children.append(xvfb)
        events.event("display", display=display)
        # Section 4: the run owns the account's data directory, so a module that
        # still resolves per-install state through `dirs::data_dir()` writes into
        # this run instead of the account.
        for directory in (run / "account-data", run / "account-config", run / "account-cache"):
            directory.mkdir(exist_ok=True)
        installed = install_facts(run / "account-data")
        before = snapshot_facts(installed)
        scenario = ScenarioRun(identifier, expanded, options.binary, options.data, run,
                               display, events)

        def interrupted(signum, _frame):
            raise DriverInterrupted(f"scenario interrupted by signal {signum}")

        signal.signal(signal.SIGINT, interrupted)
        signal.signal(signal.SIGTERM, interrupted)
        signal.signal(signal.SIGALRM, interrupted)
        signal.alarm(int(expanded["defaults"].get("budget_s", DEFAULT_BUDGET_S)))
        verify_client(options.binary, {"binary_sha256": summary["binary_sha256"]})
        scenario.start()
        results, failures = scenario.judge()
        summary["checks"] = results
        summary["failures"] = failures
        summary["trace_samples"] = len(scenario.trace)
        summary["faults_fired"] = scenario.fired
        summary["peers"] = [
            {"name": peer.name, "seat": peer.seat, "role": peer.role, "port": peer.port,
             "install": str(peer.install), "log": str(peer.log),
             "samples": str(peer.samples), "launches": peer.launches,
             "exit": peer.exit_status()}
            for peer in scenario.peers
        ]
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
            summary.setdefault("peers", [
                {"name": peer.name, "seat": peer.seat, "role": peer.role, "port": peer.port,
                 "install": str(peer.install), "log": str(peer.log),
                 "samples": str(peer.samples), "launches": peer.launches,
                 "exit": peer.exit_status()}
                for peer in scenario.peers
            ])
        summary["elapsed_s"] = round(time.monotonic() - events.started, 3)
        finish_children(children, summary)
        (run / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        events.event("summary", id=identifier, verdict=summary["verdict"],
                     failures=summary["failures"])
        events.close()
    missing = [name for name in EVIDENCE_FILES if not (run / name).exists()]
    if missing:
        raise RuntimeError(f"{run} is missing section-5 evidence: {missing}")
    return summary, run


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
    options = parser.parse_args(argv)
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
        for identifier in selected:
            expanded = scenario_corpus.expanded(identifier, scenarios, defaults, contract)
            arguments = argparse.Namespace(binary=binary, data=options.data.resolve(),
                                           snapshot=options.snapshot)
            result, run = run_scenario(identifier, expanded, arguments, evidence)
            suite["scenarios"][identifier] = {"verdict": result["verdict"],
                                              "failures": result["failures"],
                                              "evidence": str(run)}
            suite["verdicts"][identifier] = result["verdict"]
    except PrerequisiteMissing as error:
        suite["prerequisites_missing"] = [str(error)]
        print(f"multiplayer-e2e: prerequisite missing: {error}", file=sys.stderr)
    except DriverInterrupted as error:
        suite["interrupted"] = str(error)
    except Exception as error:
        suite["error"] = f"{type(error).__name__}: {error}"
        suite["traceback"] = traceback.format_exc()

    for item in refused:
        suite["verdicts"][item["id"]] = "refused"
    hangs = sorted(identifier for identifier, document in suite["scenarios"].items()
                   if document["verdict"] == "hang")
    suite["verdict"] = (
        "prerequisite-missing" if suite.get("prerequisites_missing")
        else "interrupted" if suite.get("interrupted")
        else "hang" if hangs
        else "observed" if exit_code(suite["verdicts"]) == 0 and suite["scenarios"]
        else "failed"
    )
    suite["elapsed_s"] = round(time.monotonic() - started, 3)
    (evidence / "summary.json").write_text(json.dumps(suite, indent=2) + "\n")
    print(json.dumps({"verdict": suite["verdict"], "verdicts": suite["verdicts"],
                      "refused": [item["id"] for item in refused],
                      "skipped": [item["id"] for item in skipped]}, indent=2), flush=True)

    if suite.get("prerequisites_missing"):
        return 3
    if hangs:
        return 2
    if suite.get("interrupted"):
        return 2
    return 0 if suite["verdict"] == "observed" else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
