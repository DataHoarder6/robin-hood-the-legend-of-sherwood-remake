#!/usr/bin/env python3
"""Validate the multiplayer transition scenario corpus.

The contract is `docs/multiplayer/HARNESS.md`: section 1 owns the coverage
matrix and the corpus table (which id covers which cell, at which layer, which
assertions prove it), section 2 owns the scenario schema, the fault vocabulary
and the assertion vocabulary. This module reads that document rather than
restating it, so the matrix and the corpus cannot drift apart:

* every `Hnn` cell in the matrix must resolve to a file in the corpus
  directory;
* every row's `must assert` cell carries an `asserts:` clause whose canonical
  tokens must all appear in that row's expanded `expect` list, so a cell the
  matrix claims is a cell the corpus declares;
* every row's `proves` object must name the layer section 1 gives it and an
  executor that exists: a `module::test_name` that is a real `#[test]` in a
  workspace crate, or the process suite itself.

Run `python3 scripts/validation/scenario_corpus.py` (or `--check`) to validate,
`--print H05` to see one scenario expanded through its `extends` chain.

Exit status follows the harness convention in section 5: 0 the corpus is
consistent, 1 the corpus is broken, 3 the contract or its sources cannot be
read (a broken checkout must never read as a broken corpus).

Two places where the document is narrower than a validator would like, and what
this file does about each:

* A fault verb's meaning is per layer (`kill` drops channels at core, shuts the
  runtime at transport), so it cannot be checked without a layer. This validator
  takes the layer from section 1's corpus table — the document's own per-row
  label — and rejects a fault verb that column has no entry for. A row may only
  claim a `log` or `exit` assertion, or `requires: ["graphical"]`, when that
  column says `process`: those are the assertions no Rust test makes, which is
  why section 2 gives them to the suite alone.
* Section 2 fixes the shape of `hash-equal`, `transition` and
  `transition-id-equal` by example. `barrier`, `modal`, `log`, `final-operation`
  and `exit` are named with their values but not their keys, so the shapes
  below are the smallest ones the prose supports. `seat` states `selected`,
  `owns` or both: a refused claim has no selection to name.
* `transition.payload` is not checked against `SnapshotTransitionPayload`:
  the document's own example names `LoadRestart`, which is a terminal
  debriefing action, not a payload variant. The runner resolves payload names;
  the corpus only has to agree with the document.
"""

from __future__ import annotations

import argparse
import dataclasses
import functools
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCENARIO_DIRECTORY = ROOT / "scripts/validation/scenarios"
HARNESS = ROOT / "docs/multiplayer/HARNESS.md"
GAME_OPERATION = ROOT / "crates/robin_engine/src/game_operation.rs"

SCHEMA = 3

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
    # Section 1's terminal-choice rows. Ending a mission is a process decision
    # (`POST /mission-result`); a key is the only way to reach the save surface,
    # which is a window, so it is the process layer's alone.
    "mission-result": frozenset({"process"}),
    "key": frozenset({"process"}),
    "command": frozenset({"core", "process"}),
    "native": frozenset({"core", "process"}),
}
# The fields each verb carries beyond `at`, `peer` and `fault`. An extra the
# verb does not own is an unknown field, so a `command` cannot carry a `result`.
FAULT_EXTRAS = {
    "mission-result": frozenset({"result"}),
    "command": frozenset({"command"}),
    "native": frozenset({"op", "args"}),
    "key": frozenset({"key"}),
}
MISSION_RESULTS = ("won", "lost", "aborted")
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
    "log": (set(), {"contains", "absent", "count", "at_least"}),
    "final-operation": ({"operation"}, set()),
    "exit": ({"peers", "status"}, set()),
    "count": ({"subject", "name", "times"}, set()),
    "seat": ({"seat"}, {"selected", "owns"}),
    # Each named seat has been through `count` distinct missions in this run and
    # they are on the same one now: one host-owned campaign, not two saves.
    "mission": ({"peers", "count"}, set()),
    "presentation": ({"peers", "state"}, {"kind"}),
}
COUNT_SUBJECTS = ("barrier", "transition", "operation", "reconnect")

# `/state.presentation` is the terminal page a peer is showing, so a row can say
# it appeared (`shown`) or that the seat left it after it had appeared (`gone`).
PRESENTATION_STATES = ("shown", "gone")

REQUIRES = frozenset({"game-data", "graphical", "netem"})
EXIT_STATUS = ("zero", "nonzero", "running")

# Section 2: the process layer's own assertions. No Rust test in the tree reports
# stderr text or a process exit code, so a row carrying one is a process row.
PROCESS_ASSERTIONS = frozenset({"log", "exit", "presentation"})
PROVES_FIELDS = {"layer", "tests", "runner"}
RUST_CRATES = {"core": frozenset({"robin_engine", "robin_rs"}),
               "transport": frozenset({"robin_rs"})}
PROCESS_RUNNER = "scripts/validation/multiplayer_e2e.py"

DEFAULTS_FIELDS = {"schema", "mission", "seed", "expected_players", "peer_count"}
SCENARIO_FIELDS = {"schema", "id", "extends", "proves", "peers", "faults", "resolve", "expect",
                    "requires", "budget_s", "settle_deadline_s", "campaign"}
PEER_FIELDS = {"seat", "role", "start", "timeline"}
TIMELINE_FIELDS = {"at", "command"}
AT_FIELDS = {"frame"}
BARRIER_AT_FIELDS = {"barrier", "after"}
JOIN_FRAME_FIELDS = {"join_at_frame"}
JOIN_BARRIER_FIELDS = {"join_at_barrier"}
FAULT_FIELDS = {"at", "peer", "fault", "message", "ms", "bytes", "identity",
                 "result", "command", "op", "args", "key"}
RESOLVE_FIELDS = {"peer", "result"}
# `DialogResult` as it serializes: a unit variant by name, or the load selection.
RESOLVE_RESULTS = ("Completed", "Restart", "Aborted")

TABLE_ROW = re.compile(r"^\|.*\|\s*$")
SEPARATOR = re.compile(r"^\|[\s:|-]+\|\s*$")
SCENARIO_ID = re.compile(r"\bH\d{2}\b")


ASSERTS_CLAUSE = re.compile(r"asserts:\s*(.+)")


class ContractError(RuntimeError):
    """The document or the sources it points at cannot be read."""


@dataclasses.dataclass(frozen=True)
class Contract:
    """What the document promises: covered cells, layers, assertions and the
    GameCode spellings `/state` actually writes."""

    covered: frozenset
    layers: dict
    game_codes: frozenset
    asserts: dict


def seats(values):
    """A seat list as the canonical token writes it."""
    return ",".join(str(value) for value in values)


def payload_name(payload):
    if isinstance(payload, str):
        return payload
    return "CampaignExit(" + payload["CampaignExit"] + ")"


def canonical(expectation):
    """One expectation as the exact token section 1's `asserts:` clause carries.

    Both sides are written by this function's counterpart: the document quotes
    the token, the corpus derives it from the JSON, and the gate compares them
    modulo whitespace, so a verb's shape is defined once, here.
    """
    verb = expectation["assert"]
    if verb in ("hash-equal", "hash-diverged", "transition-id-equal"):
        tail = seats(expectation["peers"])
        if verb == "hash-equal":
            tail += ",every=" + str(expectation["every"])
        return f"{verb}(peers={tail})"
    if verb == "barrier":
        return f"barrier(name={expectation['barrier']},state={expectation['state']})"
    if verb == "transition":
        return (f"transition(payload={payload_name(expectation['payload'])},"
                f"acked={seats(expectation['acked'])},"
                f"committed={str(expectation['committed']).lower()})")
    if verb == "modal":
        return f"modal(count={expectation['count']})"
    if verb == "log":
        needle = "contains" if "contains" in expectation else "absent"
        cardinality = (f"count={expectation['count']}" if "count" in expectation
                       else f"at_least={expectation['at_least']}")
        return f"log({needle}={expectation[needle]},{cardinality})"
    if verb == "final-operation":
        return f"final-operation(operation={expectation['operation']})"
    if verb == "count":
        return (f"count(subject={expectation['subject']},name={expectation['name']},"
                f"times={expectation['times']})")
    if verb == "seat":
        fields = [f"seat={expectation['seat']}"]
        if "owns" in expectation:
            fields.append(f"owns={str(expectation['owns']).lower()}")
        if "selected" in expectation:
            fields.append("selected=" + "|".join(expectation["selected"]))
        return "seat(" + ",".join(fields) + ")"
    if verb == "exit":
        return f"exit(peers={seats(expectation['peers'])},status={expectation['status']})"
    if verb == "mission":
        return (f"mission(peers={seats(expectation['peers'])},"
                f"count={expectation['count']})")
    if verb == "presentation":
        fields = [f"peers={seats(expectation['peers'])}", f"state={expectation['state']}"]
        if "kind" in expectation:
            fields.append(f"kind={expectation['kind']}")
        return "presentation(" + ",".join(fields) + ")"
    raise ContractError(f"no canonical token for assertion {verb!r}")


def normalise(token):
    return re.sub(r"\s+", "", token)


MODULE_DECLARATION = re.compile(
    r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*(;|\{)\s*$")
PATH_ATTRIBUTE = re.compile(r'^\s*#\[path\s*=\s*"([^"]+)"\s*\]\s*$')
ATTRIBUTE_OR_COMMENT = re.compile(r"^\s*(?:#\[|//)")
MAX_MODULE_DEPTH = 32


def _path_attribute(lines, number):
    """The `#[path = …]` in the attribute block above `lines[number]`, if any."""
    for earlier in reversed(lines[max(0, number - MAX_MODULE_DEPTH):number]):
        match = PATH_ATTRIBUTE.match(earlier)
        if match is not None:
            return match.group(1)
        if not earlier.strip() or ATTRIBUTE_OR_COMMENT.match(earlier):
            continue
        break
    return None


def _module_tree(src):
    """`(module path, file)` for every file the crate's own `mod` tree mounts.

    Rust, not the filesystem, names a module: `multiplayer.rs` mounts
    `multiplayer/enabled.rs` as `mod transport`, so the tests in
    `multiplayer/native/server/tests.rs` are `multiplayer::transport::native::…`
    and a path built from the file layout names nothing `cargo test` selects.
    Children of a file mounted by name live in the directory named after it;
    children of a `#[path]`-mounted file live next to it, which is how
    `enabled.rs` keeps its submodules in `multiplayer/`.
    """
    mounted, stack, seen = [], [], set()
    for name in ("lib.rs", "main.rs"):
        root = src / name
        if root.is_file():
            stack.append(((), root, src))
    while stack:
        module, path, child_dir = stack.pop()
        if (module, path) in seen or len(module) > MAX_MODULE_DEPTH:
            continue
        seen.add((module, path))
        mounted.append((module, path))
        lines = path.read_text(errors="replace").splitlines()
        for number, line in enumerate(lines):
            match = MODULE_DECLARATION.match(line)
            if match is None or match.group(2) != ";":
                continue
            name = match.group(1)
            attribute = _path_attribute(lines, number)
            if attribute is not None:
                target = path.parent / attribute
                target_dir = target.parent
            else:
                target = child_dir / f"{name}.rs"
                if not target.is_file():
                    target = child_dir / name / "mod.rs"
                target_dir = target.parent if target.name == "mod.rs" else child_dir / name
            if target.is_file():
                stack.append((module + (name,), target, target_dir))
    return mounted


@functools.cache
def _rust_index(root=ROOT):
    """`crate` -> {(module path, test name): file} for every `#[test]` fn.

    The module path comes from [`_module_tree`], so it is the path `cargo test`
    selects, `#[path]` mounting included. An inline `mod tests { … }` adds a
    segment the tree walk does not enter; [`rust_test`] accepts it only when the
    file itself declares that submodule.
    """
    index = {}
    for crate in RUST_CRATES["core"]:
        src = root / "crates" / crate / "src"
        if not src.is_dir():
            continue
        tests = {}
        for module, path in _module_tree(src):
            lines = path.read_text(errors="replace").splitlines()
            for number, line in enumerate(lines):
                match = re.match(r"\s*fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(", line)
                if match is None:
                    continue
                window = lines[max(0, number - 6):number]
                if any(re.match(r"\s*#\[test\]\s*$", attribute) for attribute in window):
                    ignored = any(re.match(r"\s*#\[ignore\b", attribute)
                                  for attribute in window)
                    tests[(module, match.group(1))] = (path, ignored)
        index[crate] = tests
    return index


def _declares_submodule(path, name):
    return re.search(rf"\bmod\s+{re.escape(name)}\b", path.read_text(errors="replace")) is not None


def _resolve(crate, module_path, name, root=ROOT):
    """`(file, ignored)` for the named test, or `None` when the tree has none."""
    tests = _rust_index(root).get(crate, {})
    if not tests:
        raise ContractError(f"crates/{crate}/src has no Rust tests to resolve")
    module = tuple(module_path.split("::")) if module_path else ()
    for candidate in (module, module[:-1] if module else ()):
        found = tests.get((candidate, name))
        if found is None:
            continue
        if candidate != module and not _declares_submodule(found[0], module[-1]):
            continue
        return found


def rust_test(crate, module_path, name, root=ROOT):
    """The file holding `#[test] fn name` under `crate::module_path`.

    A module segment that is not a file of its own must be an inline `mod` the
    file declares, so a path cannot borrow a test from an unrelated neighbour.
    """
    found = _resolve(crate, module_path, name, root)
    return None if found is None else found[0]


def ignored_test(crate, module_path, name, root=ROOT):
    """Whether the named test is `#[ignore]`d, so no suite runs it by default."""
    found = _resolve(crate, module_path, name, root)
    return found is not None and found[1]


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
    corpus_header = None
    for header, rows in _tables(text):
        if header[:1] == ["Transition"]:
            matrix = rows
        elif header[:2] == ["id", "layer"]:
            corpus, corpus_header = rows, header
    if matrix is None:
        raise ContractError(f"{harness}: no coverage matrix table")
    if corpus is None:
        raise ContractError(f"{harness}: no corpus table")
    if "must assert" not in corpus_header:
        raise ContractError(f"{harness}: the corpus table has no 'must assert' column")

    covered = frozenset(
        scenario for row in matrix for cell in row[1:] for scenario in SCENARIO_ID.findall(cell)
    )
    if not covered:
        raise ContractError(f"{harness}: the coverage matrix names no scenario")

    layers, asserts = {}, {}
    must_assert = corpus_header.index("must assert")
    for row in corpus:
        identifier, layer = row[0], row[1]
        if layer not in LAYERS:
            raise ContractError(f"{harness}: {identifier} has unknown layer {layer!r}")
        layers[identifier] = layer
        clause = ASSERTS_CLAUSE.search(row[must_assert])
        if clause is None:
            raise ContractError(
                f"{harness}: {identifier} has no `asserts:` clause, so its claim is unreadable")
        tokens = [token.strip().strip("`").strip()
                  for token in clause.group(1).split(";")]
        tokens = [token for token in tokens if token]
        if not tokens or any("(" not in token or not token.endswith(")") for token in tokens):
            raise ContractError(f"{harness}: {identifier} `asserts:` is not a token list")
        asserts[identifier] = frozenset(tokens)

    missing = sorted(covered - set(layers))
    if missing:
        raise ContractError(f"{harness}: matrix cells {missing} have no corpus row")

    match = re.search(r"pub enum GameCode \{(.*?)\n\}", source, re.S)
    if match is None:
        raise ContractError(f"{game_operation}: no GameCode enum")
    variants = frozenset(re.findall(r"^\s{4}([A-Za-z_][A-Za-z0-9_]*)", match.group(1), re.M))
    if not variants:
        raise ContractError(f"{game_operation}: GameCode has no variants")
    # `/state` spells a code through `wire_name`, never `Debug`; the corpus
    # asserts that spelling, so read the same match arms the binary compiles.
    names = re.search(r"pub fn wire_name\(self\) -> &'static str \{(.*?)\n    \}", source, re.S)
    if names is None:
        raise ContractError(f"{game_operation}: GameCode has no wire_name")
    arms = dict(re.findall(r"Self::([A-Za-z_][A-Za-z0-9_]*) => \"([a-z_]+)\"", names.group(1)))
    if set(arms) != variants:
        missing = sorted(variants ^ set(arms))
        raise ContractError(f"{game_operation}: wire names do not cover {missing}")
    return Contract(covered, layers, frozenset(arms.values()), asserts)


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
        if "campaign" in document and not isinstance(document["campaign"], bool):
            self.fail(where, "campaign must be a boolean")
        if "settle_deadline_s" in document and not _is_int(document["settle_deadline_s"], 1):
            self.fail(where, "settle_deadline_s must be a positive integer")
        if "requires" in document:
            required = document["requires"]
            if not isinstance(required, list) or any(item not in REQUIRES for item in required):
                self.fail(where, f"requires must be a list of {sorted(REQUIRES)}")
            elif len(set(required)) != len(required):
                self.fail(where, "requires repeats an entry")
        self.proves(where, identifier, document)
        seats = self.peers(where, document)
        if seats is None:
            return
        self.faults(where, identifier, document, seats)
        self.resolve(where, document, seats)
        self.expect(where, document, seats)

    def proves(self, where, identifier, document):
        """Section 1/2: the row names the layer section 1 gives it and its prover."""
        if "proves" not in document:
            return self.fail(where, "must name what proves it in `proves`")
        at = f"{where}: proves"
        proves = document["proves"]
        if not self.fields(at, proves, PROVES_FIELDS, {"layer"}):
            return
        layer = proves["layer"]
        if layer not in LAYERS:
            return self.fail(at, f"unknown layer {layer!r}")
        claimed = self.contract.layers.get(identifier)
        if claimed is not None and layer != claimed:
            self.fail(at, f"claims {layer!r} but section 1 gives {identifier} {claimed!r}")
        if layer == "process":
            if "tests" in proves:
                self.fail(at, "a process row names a runner, not Rust tests")
            if not isinstance(proves.get("runner"), str):
                return self.fail(at, f"a process row must name `runner`: {PROCESS_RUNNER}")
            if proves["runner"] != PROCESS_RUNNER:
                self.fail(at, f"runner must be {PROCESS_RUNNER}")
            elif not (ROOT / PROCESS_RUNNER).is_file():
                raise ContractError(f"{PROCESS_RUNNER} is missing from this checkout")
            return
        if "runner" in proves:
            self.fail(at, "a core or transport row names Rust tests, not a process runner")
        tests = proves.get("tests")
        if not isinstance(tests, list) or not tests:
            return self.fail(at, "must name at least one Rust test")
        allowed = RUST_CRATES[layer]
        for index, name in enumerate(tests):
            here = f"{at}.tests[{index}]"
            if not isinstance(name, str) or name.count("::") < 2:
                self.fail(here, "must be `crate::module::test_name`")
                continue
            crate = name.split("::")[0]
            module_path, test_name = "::".join(name.split("::")[1:-1]), name.rsplit("::", 1)[-1]
            if crate not in allowed:
                self.fail(here, f"{crate} is not one of {sorted(allowed)} for a {layer} row")
                continue
            found = rust_test(crate, module_path, test_name)
            if found is None:
                self.fail(here, f"{name} is not a `#[test]` in crates/{crate}")
            elif ignored_test(crate, module_path, test_name):
                self.fail(here, f"{name} is `#[ignore]`d, so no suite runs it by default")

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
            for extra in ("result", "command", "op", "args", "key"):
                if extra in fault and extra not in FAULT_EXTRAS.get(verb, frozenset()):
                    self.fail(at, f"{verb!r} does not take {extra}")
            if verb == "mission-result":
                if fault["peer"] != 0:
                    self.fail(at, "only the host (seat 0) may end the session's mission")
                if fault.get("result") not in MISSION_RESULTS:
                    self.fail(at, f"'mission-result' must name one of {list(MISSION_RESULTS)}")
            elif verb == "command":
                self.player_command(f"{at}.command", fault.get("command"))
            elif verb == "native":
                if not isinstance(fault.get("op"), str) or not fault["op"]:
                    self.fail(at, "'native' must name the native in op")
                args = fault.get("args")
                if not isinstance(args, list) or not all(_is_int(arg, -(2**31)) for arg in args):
                    self.fail(at, "'native' must carry an integer args list")
            elif verb == "key":
                if not isinstance(fault.get("key"), str) or not fault["key"]:
                    self.fail(at, "'key' must name an X keysym such as F1")

    def player_command(self, where, value):
        """A `PlayerCommand` as the `/command` endpoint takes it: one tagged variant."""
        if isinstance(value, str):
            if not value:
                self.fail(where, "command must name a PlayerCommand variant")
            return
        if not isinstance(value, dict) or len(value) != 1:
            self.fail(where, "command must be one externally tagged PlayerCommand object")

    def resolve(self, where, document, seats):
        """Section 3: pressing a seat's own debriefing page, read from `/state`."""
        if "resolve" not in document:
            return
        layer = self.contract.layers.get(document["id"])
        if layer is not None and layer != "process":
            self.fail(f"{where}: resolve",
                      f"only the process layer reads a page and presses it, not {layer!r}")
        steps = document["resolve"]
        if not isinstance(steps, list) or not steps:
            return self.fail(f"{where}: resolve", "must be a non-empty list")
        for index, step in enumerate(steps):
            at = f"{where}: resolve[{index}]"
            if not self.fields(at, step, RESOLVE_FIELDS, RESOLVE_FIELDS):
                continue
            if step["peer"] not in seats:
                self.fail(at, f"peer {step['peer']!r} is not a declared seat")
            result = step["result"]
            if isinstance(result, str):
                if result not in RESOLVE_RESULTS:
                    self.fail(at, f"result must be one of {list(RESOLVE_RESULTS)}")
            elif (isinstance(result, dict) and set(result) == {"Load"}
                  and isinstance(result["Load"], dict) and set(result["Load"]) == {"slot"}
                  and _is_int(result["Load"]["slot"], 0)):
                continue
            else:
                self.fail(at, 'result must be "Completed", "Restart", "Aborted" or '
                              '{"Load": {"slot": n}}')

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
            elif verb == "mission":
                self.seats(f"{at}.peers", expectation["peers"], seats)
                if not _is_int(expectation["count"], 1):
                    self.fail(at, "count must be a positive integer")
            elif verb == "presentation":
                self.seats(f"{at}.peers", expectation["peers"], seats)
                state = expectation["state"]
                if state not in PRESENTATION_STATES:
                    self.fail(at, f"unknown presentation state {state!r}")
                if "kind" in expectation:
                    if state == "gone":
                        self.fail(at, "a page that is gone has no kind")
                    if not isinstance(expectation["kind"], str) or not expectation["kind"]:
                        self.fail(at, "kind must name a ModalKind variant")
            elif verb == "seat":
                self.seats(f"{at}.seat", [expectation["seat"]], seats)
                if not ({"selected", "owns"} & set(expectation)):
                    self.fail(at, "seat must state selected, owns, or both")
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
        cardinals = [key for key in ("count", "at_least") if key in expectation]
        if len(cardinals) != 1:
            return self.fail(where, "log asserts exactly one of count or at_least")
        if "count" in expectation:
            if not _is_int(expectation["count"], 0):
                self.fail(where, "count must be a non-negative integer")
            elif choices[0] == "absent" and expectation["count"] != 0:
                self.fail(where, "absent cannot require a non-zero count")
        else:
            if not _is_int(expectation["at_least"], 1):
                self.fail(where, "at_least must be a positive integer")
            elif choices[0] == "absent":
                self.fail(where, "absent states a zero count, not at_least")


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


def _canonical(expectation):
    """`canonical`, or `None` when the expectation is too broken to tokenise."""
    try:
        return canonical(expectation)
    except (ContractError, KeyError, TypeError):
        return None


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
            order = chain(identifier, scenarios)
        except ValueError as error:
            checker.fail(scenarios[identifier][0], str(error))
            continue
        composed = [item for step in order for item in scenarios[step][1].get("expect", [])]
        requires = {item for step in order for item in scenarios[step][1].get("requires", [])}
        layer = contract.layers.get(identifier)
        verbs = {item["assert"] for item in composed}
        if layer is not None and layer != "process" and (
                verbs & PROCESS_ASSERTIONS or "graphical" in requires
                or "resolve" in scenarios[identifier][1]):
            checker.fail(
                scenarios[identifier][0],
                f"a log/exit assertion or `requires: [\"graphical\"]` makes this a process row, "
                f"but section 1 gives it {layer!r}",
            )
        have = {
            normalise(canonical(item)) for item in composed
            if item.get("assert") in EXPECT_FIELDS and _canonical(item) is not None
        }
        missing = sorted(token for token in contract.asserts.get(identifier, frozenset())
                         if normalise(token) not in have)
        if missing:
            checker.fail(
                scenarios[identifier][0],
                f"section 1 lists {missing} in `must assert`, which this row does not declare",
            )

    missing = sorted(set(contract.layers) - set(scenarios))
    if missing:
        checker.fail("corpus", f"corpus rows without a scenario: {', '.join(missing)}")
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
        "resolve": document.get("resolve", []),
        "expect": [item for step in order for item in scenarios[step][1].get("expect", [])],
    }
    if document.get("campaign"):
        result["campaign"] = True
    if "proves" in document:
        result["proves"] = document["proves"]
    if "requires" in document:
        result["requires"] = document["requires"]
    if "budget_s" in document:
        # Section 5's per-scenario bound: the run is an alarm for the caller,
        # so it survives expansion instead of being read from the defaults.
        result["budget_s"] = document["budget_s"]
    if "settle_deadline_s" in document:
        # A promised state the game reaches on its own schedule (the modal
        # acknowledgement budget is minutes) states how long this row may wait.
        result["settle_deadline_s"] = document["settle_deadline_s"]
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
    by_layer = {}
    for identifier in scenarios:
        by_layer[contract.layers[identifier]] = by_layer.get(contract.layers[identifier], 0) + 1
    split = ", ".join(f"{count} {layer}" for layer, count in sorted(by_layer.items()))
    tokens = sum(len(tokens) for tokens in contract.asserts.values())
    print(f"scenario corpus: {len(scenarios)} scenarios ({split}) prove "
          f"{len(contract.covered)} matrix cells with {tokens} machine-checked assertions")
    return 0


if __name__ == "__main__":
    sys.exit(main())
