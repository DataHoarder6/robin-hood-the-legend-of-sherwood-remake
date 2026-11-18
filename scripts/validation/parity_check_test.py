"""Unit tests for the Linux×Windows replay-hash parity gate; no wine required."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import parity_check as gate

HASH_LOG = """\
2026-11-18T07:43:12.1Z DEBUG robin_rs::other: unrelated debug line
2026-11-18T07:43:12.2Z DEBUG robin_rs::game_session::runtime: Replay hash OK @ frame 0: dbf9793b78f50fc1
2026-11-18T07:43:12.3Z DEBUG robin_rs::game_session::runtime: Replay hash OK @ frame 25: 15c8fe6db6007645
2026-11-18T07:43:12.4Z DEBUG robin_rs::game_session::runtime: Replay hash OK @ frame 50: 4a97b9eeb886fff1
2026-11-18T07:43:12.5Z  INFO robin_rs::game_session::headless: headless replay finished
"""

DESYNC_LOG = """\
2026-11-18T07:43:12.2Z ERROR robin_rs::game_session::runtime: Replay desync at frame 125: expected 7cf5a111542ea8e5, got 0000000000000001
replay desync: 1 recorded modal dismissal(s) were unused in their host frame
"""


def chain(*samples):
    return gate.Chain(tuple(samples))


class ParseTests(unittest.TestCase):
    def test_extracts_existing_hash_lines_in_frame_order(self):
        parsed = gate.parse_hash_chain(HASH_LOG)
        self.assertEqual(
            parsed.samples,
            ((0, "dbf9793b78f50fc1"), (25, "15c8fe6db6007645"), (50, "4a97b9eeb886fff1")),
        )

    def test_ignores_lines_that_only_look_like_hashes(self):
        self.assertEqual(
            gate.parse_hash_chain("Replay hash OK @ frame 25: not-a-hash\n").samples, ()
        )

    def test_rejects_two_values_for_one_frame(self):
        with self.assertRaises(ValueError):
            gate.parse_hash_chain(
                "Replay hash OK @ frame 25: 15c8fe6db6007645\n"
                "Replay hash OK @ frame 25: 15c8fe6db6007646\n"
            )

    def test_reports_both_desync_forms(self):
        desyncs = gate.parse_desync_lines(DESYNC_LOG)
        self.assertEqual(len(desyncs), 2)
        self.assertIn("Replay desync at frame 125", desyncs[0])
        self.assertIn("modal dismissal", desyncs[1])

    def test_accepts_a_clean_log(self):
        self.assertEqual(gate.parse_desync_lines(HASH_LOG), ())

    def test_reads_the_commit_carrying_version_banner(self):
        self.assertEqual(
            gate.parse_version("robin 0.1.0+5e1d2b17e5f5\n"), "robin 0.1.0+5e1d2b17e5f5"
        )

    def test_version_banner_is_required(self):
        self.assertEqual(gate.parse_version("wine: created the configuration\n"), "")


class CompareTests(unittest.TestCase):
    def test_equal_chains_are_identical(self):
        same = chain((0, "aa"), (25, "bb"))
        self.assertIsNone(gate.compare(same, chain((0, "aa"), (25, "bb"))))

    def test_first_mismatch_names_frame_and_both_values(self):
        divergence = gate.compare(
            chain((0, "aa"), (25, "bb"), (50, "cc")),
            chain((0, "aa"), (25, "dd"), (50, "cc")),
        )
        self.assertEqual(divergence.frame, 25)
        self.assertEqual(divergence.linux, "bb")
        self.assertEqual(divergence.windows, "dd")
        self.assertEqual(divergence.reason, "state hash mismatch")
        reported = gate.describe_divergence(divergence)
        self.assertIn("DIVERGED at frame 25", reported)
        self.assertIn("linux   = bb", reported)
        self.assertIn("windows = dd", reported)

    def test_context_shows_the_surrounding_chain(self):
        divergence = gate.compare(
            chain((0, "aa"), (25, "bb"), (50, "cc")),
            chain((0, "aa"), (25, "dd"), (50, "cc")),
        )
        frames = [row[0] for row in divergence.context]
        self.assertEqual(frames, [0, 25, 50])
        self.assertIn("-> frame     25", gate.describe_divergence(divergence))

    def test_a_chain_that_stops_early_diverges(self):
        divergence = gate.compare(chain((0, "aa"), (25, "bb")), chain((0, "aa")))
        self.assertEqual(divergence.frame, 25)
        self.assertEqual(divergence.reason, "frame sampled by only one binary")
        self.assertIsNone(divergence.windows)
        self.assertIn("<not sampled>", gate.describe_divergence(divergence))

    def test_a_shared_prefix_with_different_length_is_not_identical(self):
        self.assertIsNotNone(
            gate.compare(chain((0, "aa"), (25, "bb")), chain((0, "aa"), (25, "bb"), (50, "cc")))
        )


class ExitCodeTests(unittest.TestCase):
    def test_the_three_documented_outcomes(self):
        self.assertEqual(gate.verdict_exit("identical"), 0)
        self.assertEqual(gate.verdict_exit("not-identical"), 1)
        self.assertEqual(gate.verdict_exit("prerequisite"), 3)

    def test_an_unknown_verdict_is_an_error_not_a_pass(self):
        with self.assertRaises(KeyError):
            gate.verdict_exit("skipped")


class PrerequisiteTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(self.root))
        self.linux_dir = self.root / "linux"
        self.windows_dir = self.root / "windows"
        for directory in (self.linux_dir, self.windows_dir):
            (directory / "assets" / "core-datadir").mkdir(parents=True)
        self.linux = self.linux_dir / "robin"
        self.windows = self.windows_dir / "robin.exe"
        self.linux.write_text("")
        self.linux.chmod(0o755)
        self.windows.write_text("")
        self.replay = self.root / "recording.rhrec.jsonl"
        self.replay.write_text("")
        self.data = self.root / "game"
        (self.data / "DATA" / "Configuration").mkdir(parents=True)

    def problems(self):
        return gate.directory_problems(
            self.linux, self.windows, self.replay, self.data,
            self.linux_dir, self.windows_dir,
        )

    def test_a_complete_layout_has_no_problems(self):
        self.assertEqual(self.problems(), [])

    def test_original_distribution_casing_is_accepted(self):
        self.assertEqual(self.problems(), [])

    def test_each_missing_input_is_named(self):
        self.windows.unlink()
        self.replay.unlink()
        problems = self.problems()
        self.assertTrue(any("windows executable" in item for item in problems))
        self.assertTrue(any("replay" in item for item in problems))

    def test_a_non_executable_linux_binary_is_rejected(self):
        self.linux.chmod(0o644)
        self.assertTrue(any("not executable" in item for item in self.problems()))

    def test_a_data_root_without_configuration_is_rejected(self):
        (self.data / "DATA" / "Configuration").rmdir()
        self.assertTrue(any("Data/Configuration" in item for item in self.problems()))

    def test_an_install_without_the_core_overlay_is_rejected(self):
        __import__("shutil").rmtree(self.windows_dir / "assets")
        self.assertTrue(any("core-datadir" in item for item in self.problems()))


class WineTests(unittest.TestCase):
    def test_missing_wine_is_a_prerequisite(self):
        with mock.patch.object(gate.shutil, "which", return_value=None):
            self.assertIn("wine not found", gate.wine_problem("wine", "winepath"))

    def test_a_failing_wine_is_a_prerequisite(self):
        failed = subprocess.CompletedProcess([], 1, "", "cannot load")
        with mock.patch.object(gate.shutil, "which", side_effect=lambda name: name):
            self.assertIn(
                "wine --version failed",
                gate.wine_problem("wine", "winepath", version_runner=lambda *a, **k: failed),
            )

    def test_a_missing_winepath_is_a_prerequisite(self):
        found = subprocess.CompletedProcess([], 0, "wine-11.21\n", "")
        with mock.patch.object(
            gate.shutil, "which",
            side_effect=lambda name: "/usr/bin/wine" if name == "wine" else None,
        ):
            self.assertIn(
                "winepath not found",
                gate.wine_problem("wine", "winepath", version_runner=lambda *a, **k: found),
            )

    def test_a_working_wine_passes(self):
        found = subprocess.CompletedProcess([], 0, "wine-11.21\n", "")
        with mock.patch.object(gate.shutil, "which", side_effect=lambda name: f"/usr/bin/{name}"):
            self.assertIsNone(
                gate.wine_problem("wine", "winepath", version_runner=lambda *a, **k: found)
            )


class RunTests(unittest.TestCase):
    def healthy(self):
        return gate.Run(
            label="linux",
            version="robin 0.1.0+5e1d2b17e5f5",
            chain=chain((0, "aa")),
            desyncs=(),
            returncode=0,
            log=Path("/dev/null"),
        )

    def test_a_complete_run_is_comparable(self):
        self.assertIsNone(gate.run_problem(self.healthy()))

    def test_a_nonzero_exit_is_not_comparable(self):
        self.assertIn("exited 1", gate.run_problem(gate.Run(
            **{**self.healthy().__dict__, "returncode": 1})))

    def test_an_empty_chain_is_not_a_pass(self):
        self.assertIn("no state-hash samples", gate.run_problem(gate.Run(
            **{**self.healthy().__dict__, "chain": chain()})))

    def test_a_desync_against_the_recording_is_not_comparable(self):
        self.assertIn("desync", gate.run_problem(gate.Run(
            **{**self.healthy().__dict__, "desyncs": ("replay desync: modal",)})))


if __name__ == "__main__":
    unittest.main()
