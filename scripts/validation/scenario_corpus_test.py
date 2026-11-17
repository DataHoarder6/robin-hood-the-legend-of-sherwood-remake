"""The corpus gate refuses anything the document does not admit."""
import unittest

import scenario_corpus as corpus

CONTRACT = corpus.Contract(
    covered=frozenset({"H01", "H02"}),
    layers={"H01": "core", "H02": "core", "H03": "transport"},
    game_codes=frozenset({"Quit", "LevelSucceeded", "LevelFailed"}),
)
DEFAULTS = {
    "schema": 1,
    "mission": "H01_Lin_VL",
    "seed": 0,
    "expected_players": 2,
    "peer_count": 2,
}
PEERS = [{"seat": 0, "role": "host"}, {"seat": 1, "role": "client"}]
PROVEN = [{"assert": "hash-equal", "peers": [0, 1], "every": 25}]


def scenario(identifier, **fields):
    document = {"schema": 1, "id": identifier, "peers": PEERS, "expect": PROVEN}
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

    def test_matrix_cell_without_a_scenario_is_refused(self):
        errors = corpus.validate(DEFAULTS, corpus_of(scenario("H01")), CONTRACT)
        self.assertEqual([error for error in errors if "matrix cells" in error],
                         ["corpus: matrix cells without a scenario: H02"])

    def test_extends_cycle_is_refused(self):
        errors = corpus.validate(
            DEFAULTS,
            corpus_of(scenario("H01", extends="H02"), scenario("H02", extends="H01")),
            CONTRACT,
        )
        self.assertTrue(any("extends cycle H01 -> H02 -> H01" in error for error in errors), errors)

    def test_fault_verb_its_layer_cannot_express_is_refused(self):
        broken = scenario("H03", faults=[{"at": {"frame": 5}, "peer": 1, "fault": "diverge"}])
        errors = corpus.validate(DEFAULTS, corpus_of(scenario("H01"), broken), CONTRACT)
        self.assertIn("H03.json: faults[0]: layer 'transport' cannot express fault verb 'diverge'",
                      errors)

    def test_extends_runs_the_parent_expectations_first(self):
        parent = scenario("H01", expect=PROVEN, faults=[{"at": {"frame": 5}, "peer": 1, "fault": "kill"}])
        child = scenario("H02", extends="H01", expect=[{"assert": "final-operation", "operation": "Quit"}])
        found = corpus_of(parent, child)
        self.assertEqual(corpus.chain("H02", found), ["H01", "H02"])
        expanded = corpus.expanded("H02", found, DEFAULTS, CONTRACT)
        self.assertEqual(expanded["expect"], PROVEN + child["expect"])
        self.assertEqual(expanded["faults"], [])  # the parent's faults are the child's own window

    def test_repository_corpus_matches_the_document(self):
        defaults, scenarios = corpus.load_corpus()
        contract = corpus.load_contract()
        self.assertEqual(corpus.validate(defaults, scenarios, contract), [])
        self.assertLessEqual(contract.covered, set(scenarios))


if __name__ == "__main__":
    unittest.main()
