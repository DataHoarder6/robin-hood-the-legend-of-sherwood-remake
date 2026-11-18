"""Unit tests for the pure parts of the process-layer multiplayer suite.

The runner's process, signal and display work needs a provisioned Linux host
(section 5); everything here is a function of the contract text, a scenario
document and a recorded trace, so it runs without a display.
"""

import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import multiplayer_e2e as suite
import scenario_corpus


def load():
    contract = scenario_corpus.load_contract()
    defaults, scenarios = scenario_corpus.load_corpus()
    return contract, defaults, scenarios


class SelectionTest(unittest.TestCase):
    def setUp(self):
        self.contract, self.defaults, self.scenarios = load()
        self.run, self.skipped, self.refused = suite.select(self.scenarios)

    def test_log_and_exit_assertions_force_the_process_layer(self):
        process = set(self.run) | {item["id"] for item in self.refused}
        forced = {
            identifier for identifier, (_, document) in self.scenarios.items()
            if any(item["assert"] in suite.PROCESS_ASSERTIONS for item in document["expect"])
        }
        self.assertTrue(forced <= process)
        self.assertEqual({"H11", "H12", "H19"} <= process, True)

    def test_a_graphical_requirement_forces_the_process_layer(self):
        scenarios = {
            "H99": (".json", {
                "schema": 1, "id": "H99",
                "peers": [{"seat": 0, "role": "host"}, {"seat": 1, "role": "client"}],
                "requires": ["graphical"],
                "expect": [{"assert": "barrier", "barrier": "begin-sim", "state": "reached"}],
            })
        }
        run, skipped, refused = suite.select(scenarios)
        self.assertEqual(run, ["H99"])
        self.assertEqual((skipped, refused), ([], []))

    def test_every_unselected_scenario_carries_the_section_three_reason(self):
        for item in self.skipped:
            self.assertIn("section 3", item["reason"])
            self.assertIn(item["id"], self.scenarios)
        self.assertEqual(
            sorted([item["id"] for item in self.skipped] + self.run
                   + [item["id"] for item in self.refused]),
            sorted(self.scenarios),
        )

    def test_a_fault_the_process_layer_cannot_express_is_refused_by_name(self):
        scenarios = {
            "H98": (".json", {
                "schema": 1, "id": "H98",
                "peers": [{"seat": 0, "role": "host"}, {"seat": 1, "role": "client"}],
                "faults": [{"at": {"frame": 1}, "peer": 1, "fault": "drop",
                            "message": "Input"}],
                "expect": [{"assert": "log", "contains": "anything", "count": 1}],
            })
        }
        run, _, refused = suite.select(scenarios)
        self.assertEqual(run, [])
        self.assertEqual([item["id"] for item in refused], ["H98"])
        self.assertIn("netem drop", refused[0]["reason"])

    def test_a_declared_netem_requirement_makes_a_netem_fault_expressible(self):
        scenarios = {
            "H97": (".json", {
                "schema": 1, "id": "H97",
                "peers": [{"seat": 0, "role": "host"}, {"seat": 1, "role": "client"}],
                "requires": ["netem"],
                "faults": [{"at": {"frame": 1}, "peer": 1, "fault": "drop",
                            "message": "Input"}],
                "expect": [{"assert": "log", "contains": "anything", "count": 1}],
            })
        }
        run, _, refused = suite.select(scenarios)
        self.assertEqual((run, refused), (["H97"], []))

    def test_the_container_extension_problem_is_absent(self):
        """`extended` scenarios contribute their parent's faults and expectations."""
        contract, defaults, scenarios = load()
        parent = {
            "H90": (".json", {
                "schema": 1, "id": "H90",
                "peers": [{"seat": 0, "role": "host"}, {"seat": 1, "role": "client"}],
                "expect": [{"assert": "log", "contains": "parent wording", "count": 1}],
            })
        }
        child = {
            "H91": (".json", {
                "schema": 1, "id": "H91", "extends": "H90",
                "peers": [{"seat": 0, "role": "host"}, {"seat": 1, "role": "client"}],
                "expect": [{"assert": "barrier", "barrier": "begin-sim", "state": "reached"}],
            })
        }
        run, skipped, refused = suite.select({**parent, **child})
        self.assertEqual(run, ["H90", "H91"])
        self.assertEqual((skipped, refused), ([], []))


class ExitCodeTest(unittest.TestCase):
    def test_zero_only_when_every_selected_scenario_was_observed(self):
        self.assertEqual(suite.exit_code({"H11": "observed", "H12": "observed"}), 0)

    def test_a_single_failed_verdict_fails_the_suite(self):
        self.assertEqual(suite.exit_code({"H11": "observed", "H19": "failed"}), 1)

    def test_a_hang_is_not_a_pass(self):
        self.assertEqual(suite.exit_code({"H12": "hang"}), 1)

    def test_an_empty_selection_is_never_a_pass(self):
        self.assertEqual(suite.exit_code({}), 1)


class BarrierTest(unittest.TestCase):
    def sample(self, role, admission=None, begin_sim=None, pending=None):
        return ({"seat": 0 if role == "host" else 1, "role": role},
                {"frame": 10, "multiplayer": {
                    "local_seat": 0 if role == "host" else 1,
                    "admission": admission,
                    "begin_sim_frame": begin_sim,
                    "pending_transition": pending,
                    "last_compared_hash_frame": None,
                    "desync_count": 0,
                    "final_operation": "level_in_progress",
                }})

    def test_begin_sim_is_reached_when_the_frame_is_accepted(self):
        host, observed = self.sample("host", "waiting_for_start", begin_sim=133)
        self.assertTrue(suite.barrier_reached("begin-sim", host, observed))
        self.assertFalse(suite.barrier_released("begin-sim", host, observed))
        released = self.sample("host", "running", begin_sim=133)
        self.assertTrue(suite.barrier_released("begin-sim", *released))

    def test_a_client_only_barrier_is_not_reached_by_the_host(self):
        host, observed = self.sample("host", "host_waiting_for_begin")
        self.assertFalse(suite.barrier_reached("content-ready", host, observed))
        client, observed = self.sample("client", "peer_waiting_for_snapshot")
        self.assertTrue(suite.barrier_reached("content-ready", client, observed))
        self.assertFalse(suite.barrier_reached("snapshot-adopt", client, observed))
        adopted = self.sample("client", "peer_waiting_for_begin")
        self.assertTrue(suite.barrier_reached("snapshot-adopt", *adopted))

    def test_modal_ack_is_unprovable_rather_than_guessed(self):
        host, observed = self.sample("host", "running", begin_sim=1)
        with self.assertRaises(suite.Unprovable):
            suite.barrier_reached("modal-ack", host, observed)

    def test_absent_is_the_negation_of_reached(self):
        host, observed = self.sample("host", "host_waiting_for_begin")
        record = {"trace": [(host, observed)]}
        ok, detail, hits = suite.evaluate_barrier(
            {"assert": "barrier", "barrier": "begin-sim", "state": "absent"}, record)
        self.assertTrue(ok)
        self.assertEqual(hits, [])
        ok, detail, hits = suite.evaluate_barrier(
            {"assert": "barrier", "barrier": "begin-sim", "state": "reached"}, record)
        self.assertFalse(ok)
        self.assertIn("0 of 1", detail)

    def test_a_pending_transition_is_a_reached_commit_barrier(self):
        host, observed = self.sample("host", "running", begin_sim=1,
                                     pending={"id": 7, "payload": "Save", "awaiting": [1]})
        self.assertTrue(suite.barrier_reached("transition-commit", host, observed))
        self.assertFalse(suite.barrier_released("transition-commit", host, observed))
        released = self.sample("host", "running", begin_sim=1,
                               pending={"id": 7, "payload": "Save", "awaiting": []})
        self.assertTrue(suite.barrier_released("transition-commit", *released))


class AssertionTest(unittest.TestCase):
    def record(self, texts, statuses=None, latest=None, peers=None):
        return {
            "texts": texts,
            "peers": peers or [{"seat": 0, "role": "host"}, {"seat": 1, "role": "client"}],
            "trace": [],
            "statuses": statuses or {0: None, 1: None},
            "latest": latest or {},
        }

    def test_log_counts_every_peer_and_names_them(self):
        record = self.record({0: "handshake error\n", 1: "handshake error\n"})
        ok, detail, counts = suite.evaluate_expectation(
            {"assert": "log", "contains": "handshake error", "count": 2}, record)
        self.assertTrue(ok)
        self.assertEqual(counts, {0: 1, 1: 1})
        ok, _, _ = suite.evaluate_expectation(
            {"assert": "log", "contains": "handshake error", "count": 1}, record)
        self.assertFalse(ok)

    def trace(self, multiplayer):
        record = self.record({})
        record["trace"] = [({"seat": 0, "role": "host"}, {"multiplayer": multiplayer})]
        return record

    def test_an_absent_barrier_is_denied_by_one_reached_sample(self):
        reached = self.trace({"admission": "running", "begin_sim_frame": 0})
        ok, detail, hits = suite.evaluate_expectation(
            {"assert": "barrier", "barrier": "begin-sim", "state": "absent"}, reached)
        self.assertFalse(ok, detail)
        self.assertEqual(len(hits), 1)
        waiting = self.trace({"admission": "host_waiting_for_begin", "begin_sim_frame": None})
        ok, _, _ = suite.evaluate_expectation(
            {"assert": "barrier", "barrier": "begin-sim", "state": "absent"}, waiting)
        self.assertTrue(ok)

    def test_frame_zero_is_a_reached_begin_sim(self):
        record = self.trace({"admission": "waiting_for_start", "begin_sim_frame": 0})
        ok, _, _ = suite.evaluate_expectation(
            {"assert": "barrier", "barrier": "begin-sim", "state": "reached"}, record)
        self.assertTrue(ok)

    def test_absent_counts_zero_and_fails_on_one_hit(self):
        record = self.record({0: "received Welcome\n", 1: ""})
        ok, _, _ = suite.evaluate_expectation(
            {"assert": "log", "absent": "Welcome", "count": 0}, record)
        self.assertFalse(ok)
        ok, _, _ = suite.evaluate_expectation(
            {"assert": "log", "absent": "Reject", "count": 0}, record)
        self.assertTrue(ok)

    def test_at_least_is_a_floor_for_a_line_printed_per_retry(self):
        record = self.record({0: "refused\n" * 33, 1: ""})
        ok, _, counts = suite.evaluate_expectation(
            {"assert": "log", "contains": "refused", "at_least": 1}, record)
        self.assertTrue(ok)
        self.assertEqual(counts, {0: 33, 1: 0})
        ok, detail, _ = suite.evaluate_expectation(
            {"assert": "log", "contains": "refused", "at_least": 34}, record)
        self.assertFalse(ok)
        self.assertIn("at least 34", detail)
        ok, _, _ = suite.evaluate_expectation(
            {"assert": "log", "contains": "absent-line", "at_least": 1}, record)
        self.assertFalse(ok)

    def test_exit_status_must_be_nonzero_and_known(self):
        expectation = {"assert": "exit", "peers": [1], "status": "nonzero"}
        ok, _, _ = suite.evaluate_expectation(expectation, self.record({}, {0: 0, 1: 2}))
        self.assertTrue(ok)
        ok, _, _ = suite.evaluate_expectation(expectation, self.record({}, {0: 0, 1: 0}))
        self.assertFalse(ok)
        ok, _, _ = suite.evaluate_expectation(expectation, self.record({}, {0: 0, 1: None}))
        self.assertFalse(ok)

    def test_running_exit_states_the_peer_is_still_there(self):
        expectation = {"assert": "exit", "peers": [0], "status": "running"}
        ok, _, _ = suite.evaluate_expectation(expectation, self.record({}, {0: None, 1: 2}))
        self.assertTrue(ok)
        for status in (0, 1, -9):
            ok, _, _ = suite.evaluate_expectation(expectation,
                                                  self.record({}, {0: status, 1: 2}))
            self.assertFalse(ok)

    def test_final_operation_is_read_from_every_peer(self):
        latest = {
            0: {"multiplayer": {"final_operation": "quit"}},
            1: {"multiplayer": {"final_operation": "quit"}},
        }
        ok, _, observed = suite.evaluate_expectation(
            {"assert": "final-operation", "operation": "quit"}, self.record({}, latest=latest))
        self.assertTrue(ok)
        self.assertEqual(set(observed.values()), {"quit"})

    def test_an_unimplemented_assertion_is_unprovable(self):
        with self.assertRaises(suite.Unprovable):
            suite.evaluate_expectation(
                {"assert": "transition", "payload": "Save", "acked": [0, 1], "committed": True},
                self.record({}))


class EvidenceLayoutTest(unittest.TestCase):
    def test_each_run_gets_a_fresh_directory_below_its_scenario(self):
        with tempfile.TemporaryDirectory() as root:
            first = suite.run_directory(Path(root), "H11")
            second = suite.run_directory(Path(root), "H11")
            self.assertEqual(first, Path(root) / "H11" / "1")
            self.assertEqual(second, Path(root) / "H11" / "2")
            self.assertTrue(first.is_dir() and second.is_dir())
            (first / "summary.json").write_text("{}")
            third = suite.run_directory(Path(root), "H11")
            self.assertEqual(third.name, "3")

    def test_an_occupied_evidence_directory_is_refused(self):
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "summary.json").write_text("{}")
            with self.assertRaises(suite.PrerequisiteMissing):
                suite.require_empty(Path(root))

    def test_an_empty_or_absent_evidence_directory_is_accepted(self):
        with tempfile.TemporaryDirectory() as root:
            suite.require_empty(Path(root) / "absent")
            suite.require_empty(Path(root))

    def test_the_required_evidence_names_are_declared(self):
        self.assertEqual(suite.EVIDENCE_FILES,
                         ("scenario.json", "summary.json", "events.jsonl", "binary.sha256"))


class PrerequisiteTest(unittest.TestCase):
    def fake_binary(self, root):
        binary = Path(root) / "robin"
        binary.write_bytes(b"not a game")
        binary.chmod(0o755)
        return binary

    def provided(self, *missing):
        """A tool/host lookup where only the named executables are absent."""
        return lambda tool: None if tool in missing else f"/usr/bin/{tool}"

    def present(self):
        return lambda: None

    def test_missing_everything_is_named(self):
        with tempfile.TemporaryDirectory() as root:
            missing = suite.require_prerequisites(
                Path(root) / "absent", Path(root) / "absent",
                overlay=Path(root) / "absent", which=self.provided("unshare", "ip", "Xvfb"),
                xlib=self.present)
        joined = "\n".join(missing)
        self.assertIn("binary:", joined)
        self.assertIn("data root:", joined)
        self.assertIn("core overlay:", joined)
        self.assertIn("executable: unshare", joined)

    def test_a_missing_x_helper_is_a_prerequisite_not_a_skip(self):
        def absent():
            raise RuntimeError("python-xlib is not provisioned")
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "game" / "Data").mkdir(parents=True)
            missing = suite.require_prerequisites(self.fake_binary(root), Path(root) / "game",
                                                  overlay=root, which=self.provided(),
                                                  xlib=absent)
        self.assertEqual(missing, ["python-xlib: python-xlib is not provisioned"])

    def test_a_data_root_without_data_is_named(self):
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "empty").mkdir()
            missing = suite.require_prerequisites(self.fake_binary(root), Path(root) / "empty",
                                                  overlay=root, which=self.provided(),
                                                  xlib=self.present)
        self.assertEqual(missing, [f"data root: {Path(root) / 'empty'} contains no Data/ "
                                   "(case-insensitive)"])

    def test_a_lowercase_data_directory_is_accepted(self):
        """Shipping distributions ship Data/, DATA/ or data/ (lifecycle_gate)."""
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "game" / "data").mkdir(parents=True)
            missing = suite.require_prerequisites(self.fake_binary(root), Path(root) / "game",
                                                  overlay=root, which=self.provided(),
                                                  xlib=self.present)
        self.assertEqual(missing, [])

    def test_a_staged_overlay_and_binary_pass(self):
        with tempfile.TemporaryDirectory() as root:
            binary = self.fake_binary(root)
            (Path(root) / "game" / "Data").mkdir(parents=True)
            missing = suite.require_prerequisites(binary, Path(root) / "game", overlay=root,
                                                  which=self.provided(), xlib=self.present)
        self.assertEqual(missing, [])

    def test_the_install_root_is_inside_the_run_s_not_the_account(self):
        """The run owns `XDG_DATA_HOME`, so a stray `dirs::data_dir()` is caught."""
        self.assertEqual(suite.install_facts("/runs/H11/1/account-data"),
                         Path("/runs/H11/1/account-data/robin_hood"))

    def test_snapshot_facts_notices_a_change_outside_the_install_home(self):
        with tempfile.TemporaryDirectory() as root:
            before = suite.snapshot_facts(suite.install_facts(Path(root) / "account-data"))
            self.assertEqual(before["exists"], False)
            installed = Path(root) / "account-data" / "robin_hood"
            installed.mkdir(parents=True)
            (installed / "datadir.txt").write_text("/game")
            after = suite.snapshot_facts(suite.install_facts(Path(root) / "account-data"))
            self.assertNotEqual(before, after)


if __name__ == "__main__":
    unittest.main()
