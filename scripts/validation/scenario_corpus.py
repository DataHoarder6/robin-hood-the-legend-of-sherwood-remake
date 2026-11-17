#!/usr/bin/env python3
"""Validate the multiplayer transition scenario corpus.

The contract is `docs/multiplayer/HARNESS.md`: section 1 owns the coverage
matrix and the corpus table (which id covers which cell, at which layer),
section 2 owns the scenario schema, the fault vocabulary and the assertion
vocabulary. This module reads that document rather than restating it, so the
matrix and the corpus cannot drift apart: every `Hnn` cell in the matrix must
resolve to a file in the corpus directory.

Run `python3 scripts/validation/scenario_corpus.py` (or `--check`) to validate,
`--print H05` to see one scenario expanded through its `extends` chain.

Exit status follows the harness convention in section 5: 0 the corpus is
consistent, 1 the corpus is broken, 3 the contract or its sources cannot be
read (a broken checkout must never read as a broken corpus).

Three places where the document is narrower than a validator would like, and
what this file does about each:

* Section 3 says a scenario names no layer and the runner picks the lowest
  layer that can express it. Section 2 says the runner rejects "a fault verb
  its layer cannot express". A verb's meaning is per layer (`kill` drops
  channels at core, shuts the runtime at transport), so a verb cannot be
  checked without a layer. This validator takes the layer from section 1's
  corpus table — the document's own per-row label — and rejects a fault verb
  that column has no entry for. Assertions are not layer-bound here: section 3
  lets the runner climb, and section 5 makes the run report where it ran.
* Section 2 fixes the shape of `hash-equal`, `transition` and
  `transition-id-equal` by example. `barrier`, `modal`, `log`, `final-operation`
  and `exit` are named with their values but not their keys, so the shapes
  below are the smallest ones the prose supports.
* `transition.payload` is not checked against `SnapshotTransitionPayload`:
  the document's own example names `LoadRestart`, which is a terminal
  debriefing action, not a payload variant. The runner resolves payload names;
  the corpus only has to agree with the document.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCENARIO_DIRECTORY = ROOT / "scripts/validation/scenarios"
HARNESS = ROOT / "docs/multiplayer/HARNESS.md"
GAME_OPERATION = ROOT / "crates/robin_engine/src/game_operation.rs"

SCHEMA = 1

# Section 1: the three layers and the five barriers a scenario may name.
LAYERS = ("core", "transport", "process")
BARRIERS = (
    "content-ready",
    "snapshot-adopt",
    "begin-sim",
    "transition-commit",
    "modal-ack",
)
BARRIER_STATES = ("reached", "released", "absent")

# Section 2, the fault table. `diverge` is the only verb any layer cannot
# express: transport has no engine to mutate directly.
FAULT_VERBS = {
    "kill": frozenset(LAYERS),
    "stop": frozenset(LAYERS),
    "continue": frozenset(LAYERS),
    "duplicate": frozenset(LAYERS),
    "drop": frozenset(LAYERS),
    "reorder": frozenset(LAYERS),
    "delay": frozenset(LAYERS),
    "diverge": frozenset({"core", "process"}),
    "truncate": frozenset({"core", "transport"}),
    "restart": frozenset(LAYERS),
    "stall-surface": frozenset(LAYERS),
}
MESSAGE_VERBS = frozenset({"drop", "duplicate", "reorder", "delay"})
TRUNCATE_BYTES = 1
RESTART_IDENTITY = ("same", "fresh")

# Section 2, the assertion vocabulary, with the keys each one may carry.
# `required` and `optional` are the whole allowed field set, so an invented
# key is an unknown field.
EXPECT_FIELDS = {
    "hash-equal": ({"peers", "every"}, set()),
    "hash-diverged": ({"peers"}, set()),
    "barrier": ({"barrier", "state"}, set()),
    "transition": ({"payload", "acked", "committed"}, set()),
    "transition-id-equal": ({"peers"}, set()),
    "modal": ({"count"}, set()),
    "log": (set(), {"contains", "absent", "count"}),
    "final-operation": ({"operation"}, set()),
    "exit": ({"peers", "status"}, set()),
    "count": ({"subject", "name", "times"}, set()),
    "seat": ({"seat", "selected", "owns"}, set()),
}
COUNT_SUBJECTS = ("barrier", "transition", "operation", "reconnect")

REQUIRES = frozenset({"game-data", "graphical", "netem"})
EXIT_STATUS = ("zero", "nonzero")

DEFAULTS_FIELDS = {"schema", "mission", "seed", "expected_players", "peer_count"}
SCENARIO_FIELDS = {"schema", "id", "extends", "peers", "faults", "expect", "requires", "budget_s"}
PEER_FIELDS = {"seat", "role", "start", "timeline"}
TIMELINE_FIELDS = {"at", "command"}
AT_FIELDS = {"frame"}
BARRIER_AT_FIELDS = {"barrier", "after"}
JOIN_FRAME_FIELDS = {"join_at_frame"}
JOIN_BARRIER_FIELDS = {"join_at_barrier"}
FAULT_FIELDS = {"at", "peer", "fault", "message", "ms", "bytes", "identity"}

TABLE_ROW = re.compile(r"^\|.*\|\s*$")
SEPARATOR = re.compile(r"^\|[\s:|-]+\|\s*$")
SCENARIO_ID = re.compile(r"\bH\d{2}\b")


class ContractError(RuntimeError):
    """The document or the sources it points at cannot be read."""


@dataclasses.dataclass(frozen=True)
class Contract:
    """What the document promises: covered cells, per-id layers, GameCode names."""

    covered: frozenset
    layers: dict
    game_codes: frozenset


def _tables(text):
    """Every Markdown table in `text`, as a list of rows of cells."""
    lines = text.splitlines()
    tables = []
    index = 0
    while index < len(lines) - 1:
        if TABLE_ROW.match(lines[index]) and SEPARATOR.match(lines[index + 1]):
            header = _cells(lines[index])
            rows = []
            index += 2
            while index < len(lines) and TABLE_ROW.match(lines[index]):
                rows.append(_cells(lines[index]))
                index += 1
            tables.append((header, rows))
        else:
            index += 1
    return tables


def _cells(line):
    return [cell.strip() for cell in line.strip().strip("|").split("|")]


def load_contract(harness=HARNESS, game_operation=GAME_OPERATION) -> Contract:
    try:
        text = harness.read_text()
        source = game_operation.read_text()
    except OSError as error:
        raise ContractError(f"{error}") from error

    matrix = corpus = None
    for header, rows in _tables(text):
        if header[:1] == ["Transition"]:
            matrix = rows
        elif header[:2] == ["id", "layer"]:
            corpus = rows
    if matrix is None:
        raise ContractError(f"{harness}: no coverage matrix table")
    if corpus is None:
        raise ContractError(f"{harness}: no corpus table")

    covered = frozenset(
        scenario for row in matrix for cell in row[1:] for scenario in SCENARIO_ID.findall(cell)
    )
    if not covered:
        raise ContractError(f"{harness}: the coverage matrix names no scenario")

    layers = {}
    for row in corpus:
        identifier, layer = row[0], row[1]
        if layer not in LAYERS:
            raise ContractError(f"{harness}: {identifier} has unknown layer {layer!r}")
        layers[identifier] = layer

    missing = sorted(covered - set(layers))
    if missing:
        raise ContractError(f"{harness}: matrix cells {missing} have no corpus row")

    match = re.search(r"pub enum GameCode \{(.*?)\n\}", source, re.S)
    if match is None:
        raise ContractError(f"{game_operation}: no GameCode enum")
    game_codes = frozenset(re.findall(r"^\s{4}([A-Za-z_][A-Za-z0-9_]*)", match.group(1), re.M))
    if not game_codes:
        raise ContractError(f"{game_operation}: GameCode has no variants")
    return Contract(covered, layers, game_codes)


def load_corpus(directory=SCENARIO_DIRECTORY):
    """Return (defaults, scenarios); each scenario is (file name, document)."""
    paths = sorted(directory.glob("*.json"))
    defaults_path = directory / "defaults.json"
    if defaults_path not in paths:
        raise ContractError(f"{directory}: no defaults.json")
    defaults = _read(defaults_path)
    scenarios = {
        path.stem: (path.name, _read(path)) for path in paths if path != defaults_path
    }
    return defaults, scenarios


def _read(path):
    try:
        return json.loads(path.read_text())
    except json.JSONDecodeError as error:
        raise ContractError(f"{path.name}: not JSON: {error}") from error


def _is_int(value, minimum):
    return isinstance(value, int) and not isinstance(value, bool) and value >= minimum


class Checker:
    def __init__(self, contract, defaults):
        self.contract = contract
        self.defaults = defaults if isinstance(defaults, dict) else {}
        self.errors = []

    def fail(self, where, message):
        self.errors.append(f"{where}: {message}")

    def fields(self, where, value, allowed, required=()):
        if not isinstance(value, dict):
            self.fail(where, "must be an object")
            return False
        unknown = sorted(set(value) - set(allowed))
        if unknown:
            self.fail(where, f"unknown field(s) {unknown}")
        missing = sorted(set(required) - set(value))
        if missing:
            self.fail(where, f"missing field(s) {missing}")
        return not missing

    def at(self, where, value):
        if not isinstance(value, dict):
            return self.fail(where, "must be an object")
        if "frame" in value:
            if not self.fields(where, value, AT_FIELDS, AT_FIELDS):
                return
            if not _is_int(value["frame"], 0):
                self.fail(where, f"frame must be a non-negative integer, not {value['frame']!r}")
            return
        if not self.fields(where, value, BARRIER_AT_FIELDS, BARRIER_AT_FIELDS):
            return
        if value["barrier"] not in BARRIERS:
            self.fail(where, f"unknown barrier {value['barrier']!r}")
        if not _is_int(value["after"], 0):
            self.fail(where, f"after must be a non-negative integer, not {value['after']!r}")

    def seats(self, where, value, seats):
        if not isinstance(value, list) or not value:
            return self.fail(where, "must be a non-empty list of seats")
        for seat in value:
            if seat not in seats:
                self.fail(where, f"seat {seat!r} is not declared by this scenario")

    def scenario(self, identifier, name, document):
        where = name
        if not self.fields(where, document, SCENARIO_FIELDS, {"schema", "id", "peers"}):
            return
        if document["schema"] != SCHEMA:
            self.fail(where, f"schema {document['schema']!r} is not {SCHEMA}")
        if document["id"] != identifier:
            self.fail(where, f"id {document['id']!r} does not match its file name")
        if identifier not in self.contract.layers:
            self.fail(where, f"id {identifier} has no row in the corpus table")
        if "extends" in document and not isinstance(document["extends"], str):
            self.fail(where, "extends must be a scenario id")
        if "budget_s" in document and not _is_int(document["budget_s"], 1):
            self.fail(where, "budget_s must be a positive integer")
        if "requires" in document:
            required = document["requires"]
            if not isinstance(required, list) or any(item not in REQUIRES for item in required):
                self.fail(where, f"requires must be a list of {sorted(REQUIRES)}")
            elif len(set(required)) != len(required):
                self.fail(where, "requires repeats an entry")
        seats = self.peers(where, document)
        if seats is None:
            return
        self.faults(where, identifier, document, seats)
        self.expect(where, document, seats)

    def peers(self, where, document):
        peers = document["peers"]
        if not isinstance(peers, list) or not peers:
            self.fail(where, "peers must be a non-empty list")
            return None
        seats = []
        hosts = []
        for index, peer in enumerate(peers):
            at = f"{where}: peers[{index}]"
            if not self.fields(at, peer, PEER_FIELDS, {"seat", "role"}):
                continue
            if not _is_int(peer["seat"], 0):
                self.fail(at, f"seat must be a non-negative integer, not {peer['seat']!r}")
                continue
            if peer["seat"] in seats:
                self.fail(at, f"seat {peer['seat']} is declared twice")
            seats.append(peer["seat"])
            if peer["role"] not in ("host", "client"):
                self.fail(at, f"unknown role {peer['role']!r}")
            elif peer["role"] == "host":
                hosts.append(peer["seat"])
            if "start" in peer:
                self.start(at, peer["start"])
            if "timeline" in peer:
                self.timeline(at, peer["timeline"])
        declared = self.defaults.get("peer_count")
        if len(peers) != declared:
            self.fail(where, f"has {len(peers)} peers, defaults declare {declared}")
        if hosts != [0]:
            self.fail(where, f"exactly seat 0 must be the host, found {hosts}")
        return set(seats)

    def start(self, where, value):
        at = f"{where}: start"
        if value == "launch":
            return
        if isinstance(value, dict) and "join_at_frame" in value:
            if not self.fields(at, value, JOIN_FRAME_FIELDS, JOIN_FRAME_FIELDS):
                return
            if not _is_int(value["join_at_frame"], 0):
                self.fail(at, "join_at_frame must be a non-negative integer")
            return
        if isinstance(value, dict) and "join_at_barrier" in value:
            if not self.fields(at, value, JOIN_BARRIER_FIELDS, JOIN_BARRIER_FIELDS):
                return
            if value["join_at_barrier"] not in BARRIERS:
                self.fail(at, f"unknown barrier {value['join_at_barrier']!r}")
            return
        self.fail(at, 'must be "launch", {"join_at_frame": N} or {"join_at_barrier": name}')

    def timeline(self, where, value):
        if not isinstance(value, list):
            return self.fail(f"{where}: timeline", "must be a list")
        for index, entry in enumerate(value):
            at = f"{where}: timeline[{index}]"
            if not self.fields(at, entry, TIMELINE_FIELDS, TIMELINE_FIELDS):
                continue
            self.at(f"{at}.at", entry["at"])
            if not isinstance(entry["command"], str) or not entry["command"]:
                self.fail(at, "command must be a player command name")

    def faults(self, where, identifier, document, seats):
        if "faults" not in document:
            return
        faults = document["faults"]
        if not isinstance(faults, list):
            return self.fail(f"{where}: faults", "must be a list")
        layer = self.contract.layers.get(identifier)
        for index, fault in enumerate(faults):
            at = f"{where}: faults[{index}]"
            if not self.fields(at, fault, FAULT_FIELDS, {"at", "peer", "fault"}):
                continue
            self.at(f"{at}.at", fault["at"])
            if fault["peer"] not in seats:
                self.fail(at, f"peer {fault['peer']!r} is not a declared seat")
            verb = fault["fault"]
            if verb not in FAULT_VERBS:
                self.fail(at, f"unknown fault verb {verb!r}")
                continue
            if layer is not None and layer not in FAULT_VERBS[verb]:
                self.fail(at, f"layer {layer!r} cannot express fault verb {verb!r}")
            if verb in MESSAGE_VERBS:
                if not isinstance(fault.get("message"), str) or not fault["message"]:
                    self.fail(at, f"{verb!r} must name a NetMsg variant in message")
            elif "message" in fault:
                self.fail(at, f"{verb!r} names a peer, not a message")
            if verb == "delay":
                if not _is_int(fault.get("ms"), 1):
                    self.fail(at, "'delay' must carry a positive ms")
            elif "ms" in fault:
                self.fail(at, f"{verb!r} does not take ms")
            if verb == "truncate":
                if not _is_int(fault.get("bytes"), TRUNCATE_BYTES):
                    self.fail(at, "'truncate' must carry a positive bytes")
            elif "bytes" in fault:
                self.fail(at, f"{verb!r} does not take bytes")
            if verb == "restart":
                if fault.get("identity") not in RESTART_IDENTITY:
                    self.fail(at, "'restart' must name identity same or fresh")
            elif "identity" in fault:
                self.fail(at, f"{verb!r} does not take identity")

    def expect(self, where, document, seats):
        if "expect" not in document:
            return self.fail(where, "a scenario must assert something")
        expectations = document["expect"]
        if not isinstance(expectations, list) or not expectations:
            return self.fail(f"{where}: expect", "must be a non-empty list")
        for index, expectation in enumerate(expectations):
            at = f"{where}: expect[{index}]"
            if not isinstance(expectation, dict):
                self.fail(at, "must be an object")
                continue
            verb = expectation.get("assert")
            if verb not in EXPECT_FIELDS:
                self.fail(at, f"unknown assertion {verb!r}")
                continue
            required, optional = EXPECT_FIELDS[verb]
            allowed = required | optional | {"assert"}
            if not self.fields(at, expectation, allowed, required | {"assert"}):
                continue
            if verb == "log":
                self.log(at, expectation)
            elif verb == "barrier":
                if expectation["barrier"] not in BARRIERS:
                    self.fail(at, f"unknown barrier {expectation['barrier']!r}")
                if expectation["state"] not in BARRIER_STATES:
                    self.fail(at, f"unknown barrier state {expectation['state']!r}")
            elif verb in ("hash-equal", "hash-diverged", "transition-id-equal", "exit"):
                self.seats(f"{at}.peers", expectation["peers"], seats)
                if verb == "hash-equal" and not _is_int(expectation["every"], 1):
                    self.fail(at, "every must be a positive integer")
                if verb == "exit" and expectation["status"] not in EXIT_STATUS:
                    self.fail(at, f"unknown exit status {expectation['status']!r}")
            elif verb == "transition":
                self.transition_payload(at, expectation["payload"])
                self.seats(f"{at}.acked", expectation["acked"], seats)
                if not isinstance(expectation["committed"], bool):
                    self.fail(at, "committed must be a boolean")
            elif verb == "modal":
                if not _is_int(expectation["count"], 1):
                    self.fail(at, "count must be a positive integer")
            elif verb == "final-operation":
                if expectation["operation"] not in self.contract.game_codes:
                    self.fail(at, f"unknown GameCode {expectation['operation']!r}")
            elif verb == "count":
                if expectation["subject"] not in COUNT_SUBJECTS:
                    self.fail(at, f"unknown count subject {expectation['subject']!r}")
                if not isinstance(expectation["name"], str) or not expectation["name"]:
                    self.fail(at, "count must name what it counts")
                if not _is_int(expectation["times"], 0):
                    self.fail(at, "times must be a non-negative integer")
            elif verb == "seat":
                self.seats(f"{at}.seat", [expectation["seat"]], seats)
                selected = expectation.get("selected")
                if selected is not None and (
                    not isinstance(selected, list)
                    or not all(isinstance(item, str) and item for item in selected)
                ):
                    self.fail(at, "selected must be a list of character ids")
                owns = expectation.get("owns")
                if owns is not None and not isinstance(owns, bool):
                    self.fail(at, "owns must be a boolean")

    def transition_payload(self, where, payload):
        """A payload is `Save` or `{"CampaignExit": <GameCode>}`."""
        if isinstance(payload, str):
            if payload not in ("Save",):
                self.fail(where, f"unknown transition payload {payload!r}")
            return
        if not isinstance(payload, dict) or set(payload) != {"CampaignExit"}:
            self.fail(where, 'payload must be "Save" or {"CampaignExit": <GameCode>}')
            return
        code = payload["CampaignExit"]
        if code not in self.contract.game_codes:
            self.fail(where, f"unknown GameCode {code!r} in CampaignExit")

    def log(self, where, expectation):
        choices = [key for key in ("contains", "absent") if key in expectation]
        if len(choices) != 1:
            return self.fail(where, "log asserts exactly one of contains or absent")
        text = expectation[choices[0]]
        if not isinstance(text, str) or not text:
            self.fail(where, f"{choices[0]} must be a non-empty string")
        if "count" in expectation:
            if not _is_int(expectation["count"], 0):
                self.fail(where, "count must be a non-negative integer")
            elif choices[0] == "absent" and expectation["count"] != 0:
                self.fail(where, "absent cannot require a non-zero count")


def chain(identifier, scenarios):
    """The extends chain, root first. Raises ValueError naming the break."""
    order = []
    current = identifier
    while True:
        if current in order:
            raise ValueError("extends cycle " + " -> ".join(order + [current]))
        if current not in scenarios:
            raise ValueError(f"extends {current!r} is not a scenario in the corpus")
        order.append(current)
        parent = scenarios[current][1].get("extends")
        if parent is None:
            return list(reversed(order))
        current = parent


def validate(defaults, scenarios, contract):
    """Every reason the corpus breaks the contract, in a stable order."""
    checker = Checker(contract, defaults)
    checker.fields("defaults.json", defaults, DEFAULTS_FIELDS, DEFAULTS_FIELDS)
    if isinstance(defaults, dict) and set(defaults) == DEFAULTS_FIELDS:
        if defaults["schema"] != SCHEMA:
            checker.fail("defaults.json", f"schema {defaults['schema']!r} is not {SCHEMA}")
        if not isinstance(defaults["mission"], str) or not defaults["mission"]:
            checker.fail("defaults.json", "mission must be a mission id")
        for field, minimum in (("seed", 0), ("expected_players", 1), ("peer_count", 1)):
            if not _is_int(defaults[field], minimum):
                checker.fail("defaults.json", f"{field} must be an integer >= {minimum}")

    for identifier in sorted(scenarios):
        name, document = scenarios[identifier]
        if not isinstance(document, dict):
            checker.fail(name, "must be an object")
            continue
        checker.scenario(identifier, name, document)

    for identifier in sorted(scenarios):
        try:
            chain(identifier, scenarios)
        except ValueError as error:
            checker.fail(scenarios[identifier][0], str(error))

    missing = sorted(contract.covered - set(scenarios))
    if missing:
        checker.fail("corpus", f"matrix cells without a scenario: {', '.join(missing)}")
    return checker.errors


def expanded(identifier, scenarios, defaults, contract):
    """One scenario with its parents' expectations composed first."""
    if identifier not in scenarios:
        raise ValueError(f"{identifier} is not a scenario in the corpus")
    order = chain(identifier, scenarios)
    document = scenarios[identifier][1]
    result = {
        "id": identifier,
        "layer": contract.layers[identifier],
        "chain": order,
        "defaults": {key: value for key, value in defaults.items() if key != "schema"},
        "peers": document.get("peers", []),
        "faults": document.get("faults", []),
        "expect": [item for step in order for item in scenarios[step][1].get("expect", [])],
    }
    if "requires" in document:
        result["requires"] = document["requires"]
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="validate the corpus (default)")
    parser.add_argument("--print", dest="show", metavar="ID", help="print one expanded scenario")
    parser.add_argument("--scenarios", type=Path, default=SCENARIO_DIRECTORY)
    parser.add_argument("--harness", type=Path, default=HARNESS)
    parser.add_argument("--game-operation", type=Path, default=GAME_OPERATION)
    arguments = parser.parse_args(argv)

    try:
        contract = load_contract(arguments.harness, arguments.game_operation)
        defaults, scenarios = load_corpus(arguments.scenarios)
    except ContractError as error:
        print(f"scenario corpus: {error}", file=sys.stderr)
        return 3

    errors = validate(defaults, scenarios, contract)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    if arguments.show:
        try:
            print(json.dumps(expanded(arguments.show, scenarios, defaults, contract), indent=2))
        except ValueError as error:
            print(f"scenario corpus: {error}", file=sys.stderr)
            return 1
        return 0
    print(f"scenario corpus: {len(scenarios)} scenarios cover {len(contract.covered)} matrix cells")
    return 0


if __name__ == "__main__":
    sys.exit(main())
