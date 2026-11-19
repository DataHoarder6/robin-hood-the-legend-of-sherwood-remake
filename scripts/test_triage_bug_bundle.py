#!/usr/bin/env python3
"""Fixture tests for scripts/triage_bug_bundle.py.

A bundle is a directory: `bundle.json` plus `replay/`. The fixtures are
synthetic and small; the point is which fields the row is built from and when
the tool refuses to pretend a bundle is enough.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TRIAGE = ROOT / "scripts/triage_bug_bundle.py"

BUNDLE = {
    "description": "Mission launch failed: reconnect joined incompatible seat PlayerId(1)",
    "fingerprint": "0123456789abcdef",
    "build": {"version": "robin 0.1.0+abc123def456", "git_rev": "abc123def456"},
    "session": {
        "kind": "client",
        "session_id": "s-1",
        "transport": {"mission_id": "S01_Not_VL"},
        "tick": {"timeline_frame": 90, "simulation_tick": 133, "engine_frame_counter": 140},
        "desync": {
            "current_state_hash": "ffffffffffffffff",
            "last_compared_hash_frame": 88,
            "desync_count": 3,
        },
    },
    "notes": [],
}


class TriageTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="robin-triage-")
        self.addCleanup(self.temporary.cleanup)
        self.bundle = Path(self.temporary.name) / "2026-11-19T20-00-00Z-ingame-menu-1"
        (self.bundle / "replay").mkdir(parents=True)
        (self.bundle / "replay" / "session.rhrec.jsonl").write_text("{}\n", encoding="utf-8")
        self.write(BUNDLE)

    def write(self, document: object) -> None:
        (self.bundle / "bundle.json").write_text(json.dumps(document), encoding="utf-8")

    def triage(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(TRIAGE), str(self.bundle), *arguments],
            capture_output=True,
            text=True,
        )

    def test_row_names_the_evidence(self) -> None:
        result = self.triage()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("D-01234567", result.stdout)
        self.assertIn("mission S01_Not_VL", result.stdout)
        self.assertIn("tick 133", result.stdout)
        self.assertIn("mismatches 3", result.stdout)
        self.assertIn("robin --replay", result.stdout)
        row = [line for line in result.stdout.splitlines() if line.startswith("| D-")]
        self.assertEqual(len(row), 1)
        self.assertEqual(row[0].split(" | ")[1], "?")
        self.assertTrue(row[0].endswith("| open |"), row[0])

    def test_the_class_is_not_guessed(self) -> None:
        result = self.triage()
        self.assertIn("class    ?", result.stdout)
        self.assertIn("wait|authority|desync", result.stdout)

    def test_a_pipe_in_the_description_cannot_split_the_row(self) -> None:
        self.write({**BUNDLE, "description": "client says | host says"})
        result = self.triage()
        self.assertEqual(result.returncode, 0, result.stderr)
        row = [line for line in result.stdout.splitlines() if line.startswith("| D-")][0]
        self.assertIn("client says \\| host says", row)

    def test_a_bundle_without_json_is_refused(self) -> None:
        (self.bundle / "bundle.json").unlink()
        result = self.triage()
        self.assertEqual(result.returncode, 2)
        self.assertIn("bundle.json is missing", result.stderr)

    def test_a_bundle_without_a_replay_is_not_fit_to_file(self) -> None:
        for path in (self.bundle / "replay").iterdir():
            path.unlink()
        result = self.triage("--check")
        self.assertEqual(result.returncode, 1)
        self.assertIn("no replay", result.stderr)
        self.assertIn("no reproduction captured", result.stdout)

    def test_a_fingerprint_that_is_not_hex_is_not_fit_to_file(self) -> None:
        self.write({**BUNDLE, "fingerprint": "not-a-fingerprint"})
        result = self.triage("--check")
        self.assertEqual(result.returncode, 1)
        self.assertIn("16 lowercase hex", result.stderr)


if __name__ == "__main__":
    unittest.main()
