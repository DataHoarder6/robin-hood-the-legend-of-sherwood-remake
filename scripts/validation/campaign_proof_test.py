"""RED/GREEN contract tests for the fixture-driven campaign proof.

These are pure tests: no binary, browser, two seats or build host. They prove
that the checker rejects a broken observation at the exact place a criterion
would break, not that the game itself behaved. Real observations come from a
two-seat run; the same checker reads them unchanged.
"""
from __future__ import annotations

import copy
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import campaign_proof as proof


def claim(criterion, **overrides):
    values = {
        "schema": proof.SCHEMA,
        "id": f"campaign/{criterion}",
        "criterion": criterion,
    }
    values.update({
        "rejoin": {"min_mission_index": 2, "seat": 1, "guest_saves": 0,
                   "require_same_identity": True},
        "hash-chain": {"every": 25, "include_transitions": True, "seats": [0, 1]},
        "persistence": {"sessions": 2, "same_slot": True, "same_assignments": True,
                        "guest_saves": 0, "resume_mission_index": 2},
        "terminal": {"mission_indices": [1, 2],
                     "actions": ["continue", "restart", "next", "abort"],
                     "seats": [0, 1], "budget_s": 30, "pages": "gone"},
    }[criterion])
    values.update(overrides)
    return values


def hashes(tick, value=None):
    value = value or f"h{tick}"
    return {"tick": tick, "mission_index": 1 + tick // 100,
            "hashes": {"0": value, "1": value}}


def observation(**overrides):
    """One conforming Linux-Linux observation, expressed as scriptable data."""
    values = {
        "schema": proof.SCHEMA,
        "id": "fixture/linux-linux-pass",
        "campaign": {"slot": "thursday", "mission_count": 3},
        "hash_samples": [
            hashes(0), hashes(25), hashes(50), hashes(75),
            {"tick": 100, "mission_index": 2, "hashes": {"0": "h100", "1": "h100"}},
            {"tick": 125, "mission_index": 2, "hashes": {"0": "h125", "1": "h125"}},
            hashes(150) | {"mission_index": 2},
            {"tick": 175, "mission_index": 3, "hashes": {"0": "h175", "1": "h175"}},
        ],
        "transition_samples": [
            {"tick": 75, "from": 1, "to": 2, "hashes": {"0": "h75", "1": "h75"}},
            {"tick": 150, "from": 2, "to": 3, "hashes": {"0": "h150", "1": "h150"}},
        ],
        "rejoins": [
            {"tick": 112, "seat": 1, "drop_mission_index": 2,
             "resume_mission_index": 2, "host_mission_index": 2,
             "host_progress_preserved": True, "guest_save_writes": 0,
             "seat_identity": "same"},
        ],
        "sessions": [
            {"id": "evening-1", "slot": "thursday", "mission_index": 1,
             "assignments": {"0": ["Robin"], "1": ["John"]}, "guest_save_writes": 0},
            {"id": "evening-2", "slot": "thursday", "mission_index": 2,
             "assignments": {"0": ["Robin"], "1": ["John"]}, "guest_save_writes": 0},
        ],
        "terminals": [
            {"mission_index": mission, "action": action, "pressed_by": seat,
             "settle_s": 4.0, "budget_s": 30,
             "pages": {"0": "gone", "1": "gone"}}
            for mission in (1, 2)
            for action in ("continue", "restart", "next", "abort")
            for seat in (0, 1)
        ],
    }
    values.update(overrides)
    return values


class CampaignProofTests(unittest.TestCase):
    def assert_fails(self, criterion, observed, reason):
        with self.assertRaises(proof.ProofError) as caught:
            proof.evaluate(claim(criterion), observed)
        self.assertIn(reason, str(caught.exception), caught.exception)

    # Rejoin: mission index > 1, host progress, no guest save, equal hashes.
    def test_rejoin_at_mission_two_passes(self):
        observed = proof.evaluate(claim("rejoin"), observation())
        self.assertEqual(observed["mission_index"], 2)
        self.assertEqual(observed["seat"], 1)
        self.assertLess(observed["hash_samples_before"], observed["tick"])
        self.assertGreater(observed["hash_samples_after"], observed["tick"])

    def test_rejoin_at_mission_one_is_not_evidence(self):
        observed = observation()
        observed["rejoins"][0].update(drop_mission_index=1, resume_mission_index=1,
                                      host_mission_index=1)
        self.assert_fails("rejoin", observed, "mission index")

    def test_rejoin_forked_from_host_progress_fails(self):
        observed = observation()
        observed["rejoins"][0]["host_mission_index"] = 3
        self.assert_fails("rejoin", observed, "fork")

    def test_rejoin_with_a_guest_save_fails(self):
        observed = observation()
        observed["rejoins"][0]["guest_save_writes"] = 1
        self.assert_fails("rejoin", observed, "guest save")

    def test_rejoin_with_a_new_seat_identity_fails(self):
        observed = observation()
        observed["rejoins"][0]["seat_identity"] = "fresh"
        self.assert_fails("rejoin", observed, "identity")

    def test_rejoin_without_hash_samples_on_both_sides_fails(self):
        observed = observation()
        observed["hash_samples"] = [hashes(0), hashes(25)]
        self.assert_fails("rejoin", observed, "before and after")

    def test_rejoin_desync_after_return_names_the_tick(self):
        observed = observation()
        observed["hash_samples"][-1] = {
            "tick": 175, "mission_index": 3, "hashes": {"0": "a", "1": "b"},
        }
        self.assert_fails("rejoin", observed, "tick 175")

    # Hash chain: one continuous chain over the whole run, transitions included.
    def test_continuous_chain_passes(self):
        observed = proof.evaluate(claim("hash-chain"), observation())
        self.assertEqual(observed["ticks"][0], 0)
        self.assertEqual(observed["ticks"][-1], 175)
        self.assertEqual(observed["transitions"], 2)

    def test_a_gap_in_the_chain_fails_with_the_tick(self):
        observed = observation()
        observed["hash_samples"] = [sample for sample in observed["hash_samples"]
                                    if sample["tick"] != 125]
        self.assert_fails("hash-chain", observed, "gap at tick 125")

    def test_a_mismatched_hash_names_the_tick(self):
        observed = observation()
        observed["hash_samples"][2]["hashes"]["1"] = "diverged"
        self.assert_fails("hash-chain", observed, "desync at tick 50")

    def test_a_transition_snapshot_is_part_of_the_chain(self):
        observed = observation()
        observed["transition_samples"] = []
        self.assert_fails("hash-chain", observed, "transition")

    def test_a_transition_hash_mismatch_names_the_tick(self):
        observed = observation()
        observed["transition_samples"][0]["hashes"]["1"] = "wrong"
        self.assert_fails("hash-chain", observed, "tick 75")

    def test_a_mission_index_that_goes_backwards_fails(self):
        observed = observation()
        observed["hash_samples"][4]["mission_index"] = 1
        self.assert_fails("hash-chain", observed, "mission index")

    # Persistence: the slot survives two sessions with the same assignments.
    def test_two_session_slot_resume_passes(self):
        observed = proof.evaluate(claim("persistence"), observation())
        self.assertEqual(observed["sessions"], 2)
        self.assertEqual(observed["mission_indices"], [1, 2])

    def test_a_second_session_with_a_different_slot_fails(self):
        observed = observation()
        observed["sessions"][1]["slot"] = "another"
        self.assert_fails("persistence", observed, "slot")

    def test_changed_player_assignments_fail(self):
        observed = observation()
        observed["sessions"][1]["assignments"]["1"] = ["Robin"]
        self.assert_fails("persistence", observed, "assignments")

    def test_a_guest_save_breaks_host_ownership(self):
        observed = observation()
        observed["sessions"][1]["guest_save_writes"] = 1
        self.assert_fails("persistence", observed, "guest save")

    def test_a_resume_at_the_wrong_mission_fails(self):
        observed = observation()
        observed["sessions"][1]["mission_index"] = 1
        self.assert_fails("persistence", observed, "resume mission")

    def test_missing_session_fails_closed(self):
        observed = observation()
        observed["sessions"] = observed["sessions"][:1]
        self.assert_fails("persistence", observed, "session")

    # Terminal matrix: every action on every seat settles on a shared page.
    def test_full_terminal_matrix_passes(self):
        observed = proof.evaluate(claim("terminal"), observation())
        self.assertEqual(observed["combinations"], 16)
        self.assertEqual(observed["slowest_settle_s"], 4.0)

    def test_a_missing_action_seat_pair_fails(self):
        observed = observation()
        observed["terminals"] = [item for item in observed["terminals"]
                                 if not (item["action"] == "abort" and item["pressed_by"] == 1)]
        self.assert_fails("terminal", observed, "mission 1 abort seat 1")

    def test_an_unbounded_settle_fails(self):
        observed = observation()
        observed["terminals"][0]["settle_s"] = 45
        self.assert_fails("terminal", observed, "settle")

    def test_a_seat_left_on_a_page_the_other_left_fails(self):
        observed = observation()
        observed["terminals"][0]["pages"]["1"] = "shown"
        self.assert_fails("terminal", observed, "page")

    def test_a_mission_without_terminal_rows_fails(self):
        observed = observation()
        observed["terminals"] = [item for item in observed["terminals"]
                                 if item["mission_index"] == 1]
        self.assert_fails("terminal", observed, "mission 2")

    # The CLI and the fixture contract itself.
    def test_unknown_criterion_is_refused(self):
        broken = claim("rejoin")
        broken["criterion"] = "vibes"
        with self.assertRaises(proof.ProofError):
            proof.evaluate(broken, observation())

    def test_missing_observation_field_fails_closed(self):
        observed = observation()
        del observed["rejoins"]
        self.assert_fails("rejoin", observed, "missing")

    def test_wrong_schema_is_refused(self):
        observed = observation()
        observed["schema"] = 99
        with self.assertRaises(proof.ProofError) as caught:
            proof.evaluate(claim("rejoin"), observed)
        self.assertIn("schema", str(caught.exception))

    def test_cli_binds_an_observation_to_an_exact_commit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            claim_path = root / "claim.json"
            observation_path = root / "observation.json"
            claim_path.write_text(json.dumps(claim("rejoin")))
            observed = observation()
            observed["commit"] = "6a86371f3930"
            observation_path.write_text(json.dumps(observed))
            argv = [sys.executable, str(Path(proof.__file__).resolve()),
                    "--claim", str(claim_path), "--observation", str(observation_path)]
            bound = subprocess.run(argv + ["--commit", "6a86371f3930"],
                                   capture_output=True, text=True)
            wrong = subprocess.run(argv + ["--commit", "000000000000"],
                                   capture_output=True, text=True)
            unbound = subprocess.run(argv, capture_output=True, text=True)
        self.assertEqual(bound.returncode, 0, bound.stderr)
        self.assertIn('"commit": "6a86371f3930"', bound.stdout)
        self.assertEqual(wrong.returncode, 3, wrong.stderr)
        self.assertIn("commit", wrong.stdout + wrong.stderr)
        self.assertEqual(unbound.returncode, 0, unbound.stderr)

    def test_cli_refuses_to_bind_an_unattributed_observation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            claim_path = root / "claim.json"
            observation_path = root / "observation.json"
            claim_path.write_text(json.dumps(claim("rejoin")))
            observation_path.write_text(json.dumps(observation()))
            argv = [sys.executable, str(Path(proof.__file__).resolve()),
                    "--claim", str(claim_path), "--observation", str(observation_path),
                    "--commit", "6a86371f3930"]
            result = subprocess.run(argv, capture_output=True, text=True)
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertIn("commit", result.stdout + result.stderr)

    def test_cli_green_and_red_exit_codes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            claim_path = root / "claim.json"
            good_path = root / "good.json"
            bad_path = root / "bad.json"
            claim_path.write_text(json.dumps(claim("rejoin")))
            good_path.write_text(json.dumps(observation()))
            bad = copy.deepcopy(observation())
            bad["rejoins"][0]["resume_mission_index"] = 1
            bad_path.write_text(json.dumps(bad))
            script = Path(proof.__file__).resolve()
            argv = [sys.executable, str(script), "--claim", str(claim_path)]
            green = subprocess.run(argv + ["--observation", str(good_path)],
                                   capture_output=True, text=True)
            red = subprocess.run(argv + ["--observation", str(bad_path)],
                                 capture_output=True, text=True)
            missing = subprocess.run(argv + ["--observation", str(root / "absent.json")],
                                     capture_output=True, text=True)
        self.assertEqual(green.returncode, 0, green.stderr)
        self.assertIn('"verdict": "observed"', green.stdout)
        self.assertEqual(red.returncode, 1, red.stderr)
        self.assertIn("mission index", red.stdout + red.stderr)
        self.assertEqual(missing.returncode, 3, missing.stderr)


if __name__ == "__main__":
    unittest.main()
