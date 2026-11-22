"""The corpus gate refuses anything the document does not admit."""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import scenario_corpus as corpus

CORE_TESTS = ["robin_engine::engine::coop::tests::ownership_rules_and_disconnect"]
TRANSPORT_TESTS = ["robin_rs::multiplayer::tests::state_reports_the_hosts_dial_address"]

CONTRACT = corpus.Contract(
    covered=frozenset({"H01", "H02"}),
    layers={"H01": "core", "H02": "core"},
    game_codes=frozenset({"quit", "level_succeeded", "level_failed"}),
    asserts={"H01": frozenset({"hash-equal(peers=0,1,every=25)"}),
             "H02": frozenset({"hash-equal(peers=0,1,every=25)"})},
)
# The same two rows as process rows, for the assertions only that layer carries.
PROCESS_CONTRACT = corpus.Contract(
    covered=CONTRACT.covered,
    layers={"H01": "process", "H02": "process"},
    game_codes=CONTRACT.game_codes,
    asserts=CONTRACT.asserts,
)
PROCESS_PROVES = {"layer": "process", "runner": corpus.PROCESS_RUNNER}
# One extra row, only for the fault-verb/layer test, so the shared fixture stays
# the smallest corpus the other cases need.
TRANSPORT_CONTRACT = corpus.Contract(
    covered=CONTRACT.covered,
    layers={**CONTRACT.layers, "H03": "transport"},
    game_codes=CONTRACT.game_codes,
    asserts={**CONTRACT.asserts, "H03": CONTRACT.asserts["H01"]},
)
DEFAULTS = {
    "schema": corpus.SCHEMA,
    "mission": "H01_Lin_VL",
    "seed": 0,
    "expected_players": 2,
    "peer_count": 2,
}
PEERS = [{"seat": 0, "role": "host"}, {"seat": 1, "role": "client"}]
PROVEN = [{"assert": "hash-equal", "peers": [0, 1], "every": 25}]
PROVES = {"layer": "core", "tests": CORE_TESTS}


def scenario(identifier, **fields):
    document = {"schema": corpus.SCHEMA, "id": identifier, "proves": PROVES, "peers": PEERS,
                "expect": PROVEN}
    document.update(fields)
    return document


def corpus_of(*documents):
    return {document["id"]: (f"{document['id']}.json", document) for document in documents}


class CorpusValidationTests(unittest.TestCase):
    def assert_refused(self, documents, message):
        errors = corpus.validate(DEFAULTS, corpus_of(*documents), CONTRACT)
        self.assertTrue(any(message in error for error in errors), errors)

    def test_valid_corpus_passes(self):
        self.assertEqual(corpus.validate(DEFAULTS, corpus_of(scenario("H01"), scenario("H02")), CONTRACT), [])

    def test_unknown_fault_verb_is_refused(self):
        broken = scenario("H01", faults=[{"at": {"frame": 5}, "peer": 1, "fault": "explode"}])
        self.assert_refused([broken, scenario("H02")], "unknown fault verb")

    def test_unknown_assertion_is_refused(self):
        broken = scenario("H01", expect=[{"assert": "hash-equalish", "peers": [0, 1], "every": 25}])
        self.assert_refused([broken, scenario("H02")], "unknown assertion")

    def test_unknown_field_is_refused(self):
        broken = scenario("H01", retries=2)
        self.assert_refused([broken, scenario("H02")], "unknown field")

    def test_a_cardinality_claim_is_an_assertion_not_log_wording(self):
        good = scenario(
            "H01",
            expect=PROVEN + [{"assert": "count", "subject": "barrier", "name": "begin-sim", "times": 1}],
        )
        self.assertEqual(corpus.validate(DEFAULTS, corpus_of(good, scenario("H02")), CONTRACT), [])
        broken = scenario(
            "H01",
            expect=[{"assert": "count", "subject": "vibes", "name": "begin-sim", "times": 1}],
        )
        self.assert_refused([broken, scenario("H02")], "unknown count subject")

    def process(self, identifier, **fields):
        fields.setdefault("proves", PROCESS_PROVES)
        return scenario(identifier, **fields)

    def test_a_log_line_is_counted_exactly_or_with_a_floor(self):
        floor = self.process("H01", expect=PROVEN + [
            {"assert": "log", "contains": "refused", "at_least": 1}])
        self.assertEqual(corpus.validate(DEFAULTS, corpus_of(floor, self.process("H02")),
                                         PROCESS_CONTRACT), [])
        both = self.process("H01", expect=[{"assert": "log", "contains": "x", "count": 1,
                                            "at_least": 1}])
        self.assert_refused([both, self.process("H02")],
                            "exactly one of count or at_least")
        neither = self.process("H01", expect=[{"assert": "log", "contains": "x"}])
        self.assert_refused([neither, self.process("H02")],
                            "exactly one of count or at_least")
        absent = self.process("H01", expect=[{"assert": "log", "absent": "x", "at_least": 1}])
        self.assert_refused([absent, self.process("H02")], "absent states a zero count")

    def test_a_campaign_exit_carries_the_code_two_terminal_rows_differ_in(self):
        good = scenario(
            "H01",
            expect=PROVEN + [{"assert": "transition",
                              "payload": {"CampaignExit": "level_failed"},
                              "acked": [0, 1], "committed": True}],
        )
        self.assertEqual(corpus.validate(DEFAULTS, corpus_of(good, scenario("H02")), CONTRACT), [])
        broken = scenario(
            "H01",
            expect=PROVEN + [{"assert": "transition",
                              "payload": {"CampaignExit": "LevelExploded"},
                              "acked": [0, 1], "committed": True}],
        )
        self.assert_refused([broken, scenario("H02")], "unknown GameCode")

    def test_restart_states_whether_the_seat_keeps_its_identity(self):
        good = scenario("H01", faults=[{"at": {"frame": 40}, "peer": 1,
                                       "fault": "restart", "identity": "fresh"}])
        self.assertEqual(corpus.validate(DEFAULTS, corpus_of(good, scenario("H02")), CONTRACT), [])
        broken = scenario("H01", faults=[{"at": {"frame": 40}, "peer": 1, "fault": "restart"}])
        self.assert_refused([broken, scenario("H02")], "identity same or fresh")

    def test_matrix_cell_without_a_scenario_is_refused(self):
        errors = corpus.validate(DEFAULTS, corpus_of(scenario("H01")), CONTRACT)
        self.assertEqual([error for error in errors if "matrix cells" in error],
                         ["corpus: matrix cells without a scenario: H02"])

    def process_scenario(self, identifier, **fields):
        return scenario(identifier, proves=PROCESS_PROVES, **fields)

    def process_refused(self, documents, message):
        errors = corpus.validate(DEFAULTS, corpus_of(*documents), PROCESS_CONTRACT)
        self.assertTrue(any(message in error for error in errors), errors)

    def test_a_mission_end_names_its_result_and_only_the_host(self):
        good = {"at": {"frame": 40}, "peer": 0, "fault": "mission-result", "result": "lost"}
        document = self.process_scenario(
            "H01", faults=[good],
            expect=PROVEN + [{"assert": "presentation", "peers": [0, 1], "state": "gone"}])
        self.assertEqual(corpus.validate(DEFAULTS,
                                         corpus_of(document, self.process_scenario("H02")),
                                         PROCESS_CONTRACT), [])
        broken = self.process_scenario("H01", faults=[{**good, "peer": 1}])
        self.process_refused([broken, self.process_scenario("H02")], "only the host")
        broken = self.process_scenario("H01", faults=[{**good, "result": "draw"}])
        self.process_refused([broken, self.process_scenario("H02")], "must name one of")

    def test_a_scripted_victory_is_a_host_only_process_verb(self):
        good = {"at": {"frame": 40}, "peer": 0, "fault": "script-win"}
        document = self.process_scenario("H01", faults=[good], expect=PROVEN)
        self.assertEqual(corpus.validate(DEFAULTS,
                                         corpus_of(document, self.process_scenario("H02")),
                                         PROCESS_CONTRACT), [])
        # Not expressible at core: only the process layer drives the game's own
        # victory command plus the player's leave command through HTTP.
        core = scenario("H01", faults=[good])
        self.assert_refused([core, scenario("H02")], "layer 'core' cannot express")

    def test_a_page_that_is_gone_cannot_name_a_kind(self):
        omitted = self.process_scenario("H01", expect=PROVEN + [
            {"assert": "presentation", "peers": [0, 1], "state": "gone"}])
        self.assertEqual(corpus.validate(DEFAULTS,
                                         corpus_of(omitted, self.process_scenario("H02")),
                                         PROCESS_CONTRACT), [])
        named = self.process_scenario("H01", expect=PROVEN + [
            {"assert": "presentation", "peers": [0], "state": "gone",
             "kind": "FinalDebriefing"}])
        self.process_refused([named, self.process_scenario("H02")], "no kind")

    def test_a_command_carries_the_player_command_it_posts(self):
        good = self.process_scenario(
            "H01",
            faults=[{"at": {"frame": 40}, "peer": 1, "fault": "command",
                     "command": {"ModalDismiss": {"kind": "X", "result": "Completed"}}}],
            expect=PROVEN)
        self.assertEqual(corpus.validate(DEFAULTS,
                                         corpus_of(good, self.process_scenario("H02")),
                                         PROCESS_CONTRACT), [])
        broken = self.process_scenario("H01", faults=[
            {"at": {"frame": 40}, "peer": 1, "fault": "command"}])
        self.process_refused([broken, self.process_scenario("H02")], "externally tagged")

    def test_resolve_belongs_to_the_process_layer(self):
        resolving = scenario("H01", resolve=[{"peer": 0, "result": "Completed"}])
        self.assert_refused([resolving, scenario("H02")],
                            "only the process layer reads a page")

    def test_extends_cycle_is_refused(self):
        errors = corpus.validate(
            DEFAULTS,
            corpus_of(scenario("H01", extends="H02"), scenario("H02", extends="H01")),
            CONTRACT,
        )
        self.assertTrue(any("extends cycle H01 -> H02 -> H01" in error for error in errors), errors)

    def test_fault_verb_its_layer_cannot_express_is_refused(self):
        broken = scenario(
            "H03",
            proves={"layer": "transport", "tests": TRANSPORT_TESTS},
            faults=[{"at": {"frame": 5}, "peer": 1, "fault": "diverge"}],
        )
        errors = corpus.validate(DEFAULTS, corpus_of(scenario("H01"), broken),
                                 TRANSPORT_CONTRACT)
        self.assertIn("H03.json: faults[0]: layer 'transport' cannot express fault verb 'diverge'",
                      errors)

    def test_extends_runs_the_parent_expectations_first(self):
        parent = scenario("H01", expect=PROVEN, faults=[{"at": {"frame": 5}, "peer": 1, "fault": "kill"}])
        child = scenario("H02", extends="H01", expect=[{"assert": "final-operation", "operation": "quit"}])
        found = corpus_of(parent, child)
        self.assertEqual(corpus.chain("H02", found), ["H01", "H02"])
        expanded = corpus.expanded("H02", found, DEFAULTS, CONTRACT)
        self.assertEqual(expanded["expect"], PROVEN + child["expect"])
        self.assertEqual(expanded["faults"], [])  # the parent's faults are the child's own window

    def test_budget_survives_expansion(self):
        bounded = scenario("H01", budget_s=180)
        expanded = corpus.expanded("H01", corpus_of(bounded), DEFAULTS, CONTRACT)
        self.assertEqual(expanded["budget_s"], 180)
        unbounded = corpus.expanded("H02", corpus_of(
            scenario("H02", expect=PROVEN)), DEFAULTS, CONTRACT)
        self.assertNotIn("budget_s", unbounded)

    def test_a_row_without_a_prover_is_refused(self):
        broken = scenario("H01")
        del broken["proves"]
        self.assert_refused([broken, scenario("H02")], "must name what proves it")

    def test_a_prover_that_disagrees_with_section_one_is_refused(self):
        broken = scenario("H01", proves={"layer": "transport", "tests": TRANSPORT_TESTS})
        self.assert_refused([broken, scenario("H02")],
                            "claims 'transport' but section 1 gives H01 'core'")

    def test_a_named_rust_test_that_does_not_exist_is_refused(self):
        broken = scenario("H01", proves={
            "layer": "core", "tests": ["robin_engine::engine::coop::tests::no_such_test"]})
        self.assert_refused([broken, scenario("H02")], "is not a `#[test]` in crates/robin_engine")

    def test_a_test_in_another_crate_than_the_layer_allows_is_refused(self):
        broken = scenario("H01", proves={
            "layer": "core", "tests": ["robin_util::tests::anything"]})
        self.assert_refused([broken, scenario("H02")],
                            "robin_util is not one of ['robin_engine', 'robin_rs']")

    def test_a_core_row_cannot_carry_a_process_only_assertion(self):
        broken = scenario("H01", expect=PROVEN + [
            {"assert": "log", "contains": "anything", "count": 1}])
        self.assert_refused([broken, scenario("H02")], "makes this a process row")

    def test_the_matrix_asserts_clause_is_enforced_against_the_row(self):
        # The fixture's matrix promises `hash-equal(peers=0,1,every=25)`.
        weak = scenario("H01", expect=[{"assert": "final-operation", "operation": "Quit"}])
        self.assert_refused([weak, scenario("H02")],
                            "section 1 lists ['hash-equal(peers=0,1,every=25)']")

    def test_a_row_without_a_parseable_asserts_clause_breaks_the_contract(self):
        with tempfile.TemporaryDirectory() as root:
            harness = Path(root) / "HARNESS.md"
            harness.write_text(
                "| Transition | BA |\n| --- | --- |\n| mission start | H01 |\n\n"
                "| id | layer | role | must assert | extends |\n| --- | --- | --- | --- | --- |\n"
                "| H01 | core | baseline | peers agree | pair |\n"
            )
            with self.assertRaises(corpus.ContractError):
                corpus.load_contract(harness, Path(root) / "GameCode.rs")

    def test_repository_corpus_matches_the_document(self):
        defaults, scenarios = corpus.load_corpus()
        contract = corpus.load_contract()
        self.assertEqual(corpus.validate(defaults, scenarios, contract), [])
        self.assertLessEqual(contract.covered, set(scenarios))
        # One scenario per corpus row; a row without a file, or a file without a
        # row, is the mismatch that matters — not the count on the day.
        self.assertEqual(len(scenarios), len(contract.layers))
        layers = [contract.layers[identifier] for identifier in scenarios]
        for layer in corpus.LAYERS:
            self.assertIn(layer, layers)
        for row in scenarios:
            self.assertTrue(contract.asserts[row], f"{row} has no machine-checked assertion")
            proves = scenarios[row][1]["proves"]
            self.assertTrue(proves.get("tests") or proves.get("runner"))


class RustTestResolutionTests(unittest.TestCase):
    """A test's name is the module path `cargo test` selects, not its file path."""

    def tree(self, root, files):
        for name, text in files.items():
            path = Path(root) / "crates/robin_rs/src" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)

    def test_a_path_mounted_module_names_its_children_under_the_mount(self):
        # `multiplayer.rs` mounts `enabled.rs` as `transport`, so the file at
        # `multiplayer/native/server/tests.rs` is `multiplayer::transport::…`
        # and a path built from the file layout names nothing cargo selects.
        files = {
            "lib.rs": "pub mod multiplayer;\n",
            "multiplayer.rs": (
                '#[cfg(feature = "multiplayer")]\n'
                '#[path = "multiplayer/enabled.rs"]\n'
                "mod transport;\n"
            ),
            "multiplayer/enabled.rs": "pub(super) mod native;\n",
            "multiplayer/native.rs": "mod server;\n",
            "multiplayer/native/server.rs": "#[cfg(test)]\nmod tests;\n",
            "multiplayer/native/server/tests.rs": "#[test]\nfn probe() {}\n",
        }
        with tempfile.TemporaryDirectory() as root:
            self.tree(root, files)
            found = corpus.rust_test("robin_rs", "multiplayer::transport::native::server::tests",
                                     "probe", root=Path(root))
            self.assertEqual(found, Path(root) / "crates/robin_rs/src/multiplayer/native/server/tests.rs")
            self.assertIsNone(
                corpus.rust_test("robin_rs", "multiplayer::native::server::tests",
                                 "probe", root=Path(root)))

    def test_an_inline_submodule_must_be_declared_by_the_file(self):
        files = {
            "lib.rs": "pub mod game;\n",
            "game.rs": "mod tests {\n    #[test]\n    fn probe() {}\n}\n",
        }
        with tempfile.TemporaryDirectory() as root:
            self.tree(root, files)
            self.assertIsNotNone(corpus.rust_test("robin_rs", "game::tests", "probe",
                                                 root=Path(root)))
            # `game::elsewhere::tests` is not a module the file declares.
            self.assertIsNone(corpus.rust_test("robin_rs", "game::elsewhere::tests", "probe",
                                               root=Path(root)))

    def test_an_ignored_test_is_named_as_ignored(self):
        files = {
            "lib.rs": "pub mod game;\n",
            "game.rs": ('#[test]\n#[ignore = "requires full game data"]\nfn probe() {}\n'),
        }
        with tempfile.TemporaryDirectory() as root:
            self.tree(root, files)
            self.assertTrue(corpus.ignored_test("robin_rs", "game", "probe", root=Path(root)))

    def test_an_ignored_prover_is_refused(self):
        # A `#[ignore]`d test exists but no suite runs it, so it proves nothing.
        broken = scenario("H01", proves={"layer": "core", "tests": CORE_TESTS})
        with mock.patch.object(corpus, "ignored_test", return_value=True):
            errors = corpus.validate(DEFAULTS, corpus_of(broken, scenario("H02")), CONTRACT)
        self.assertTrue(any("is `#[ignore]`d" in error for error in errors), errors)

if __name__ == "__main__":
    unittest.main()

