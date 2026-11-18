"""The corpus gate refuses anything the document does not admit."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import scenario_corpus as corpus

CONTRACT = corpus.Contract(
    covered=frozenset({"H01", "H02"}),
    layers={"H01": "core", "H02": "core", "H03": "transport"},
    game_codes=frozenset({"quit", "level_succeeded", "level_failed"}),
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

    def test_a_log_line_is_counted_exactly_or_with_a_floor(self):
        floor = scenario("H01", expect=PROVEN + [
            {"assert": "log", "contains": "refused", "at_least": 1}])
        self.assertEqual(corpus.validate(DEFAULTS, corpus_of(floor, scenario("H02")), CONTRACT), [])
        both = scenario("H01", expect=[{"assert": "log", "contains": "x", "count": 1,
                                        "at_least": 1}])
        self.assert_refused([both, scenario("H02")], "exactly one of count or at_least")
        neither = scenario("H01", expect=[{"assert": "log", "contains": "x"}])
        self.assert_refused([neither, scenario("H02")], "exactly one of count or at_least")
        absent = scenario("H01", expect=[{"assert": "log", "absent": "x", "at_least": 1}])
        self.assert_refused([absent, scenario("H02")], "absent states a zero count")

    def test_a_campaign_exit_carries_the_code_two_terminal_rows_differ_in(self):
        good = scenario(
            "H01",
            expect=[{"assert": "transition", "payload": {"CampaignExit": "level_failed"},
                     "acked": [0, 1], "committed": True}],
        )
        self.assertEqual(corpus.validate(DEFAULTS, corpus_of(good, scenario("H02")), CONTRACT), [])
        broken = scenario(
            "H01",
            expect=[{"assert": "transition", "payload": {"CampaignExit": "LevelExploded"},
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

    def test_repository_corpus_matches_the_document(self):
        defaults, scenarios = corpus.load_corpus()
        contract = corpus.load_contract()
        self.assertEqual(corpus.validate(defaults, scenarios, contract), [])
        self.assertLessEqual(contract.covered, set(scenarios))


if __name__ == "__main__":
    unittest.main()
