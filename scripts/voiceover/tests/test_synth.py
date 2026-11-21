"""Offline tests for the voice-over pipeline. No network, no game data."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
SYNTH = HERE / "synth.py"

CAST = """
[narrator]
voice = "lt-LT-Chirp3-HD-Charon"
encoding = "OGG_OPUS"

[soldier]
voice = "lt-LT-Chirp3-HD-Schedar"
encoding = "OGG_OPUS"
"""

LINES = """
[[line]]
id = 1000105
index = 0
display = "Jaunas Robinas iš Loksli paliko savo kraštą."
spoken = ""
voice = "narrator"
provenance = "machine-draft"

[[line]]
id = 1000106
index = 0
display = "Sargybinis!"
spoken = "Sargybinis!"
voice = "soldier"
provenance = "machine-draft"

[[line]]
id = 1000349
index = 0
display = "Robinas paliko Lincolną."
spoken = ""
voice = "none"
provenance = "machine-draft"
"""


class Fixture(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.lines = self.root / "lines" / "h01.toml"
        self.lines.parent.mkdir(parents=True)
        self.lines.write_text(LINES)
        self.cast = self.root / "cast.toml"
        self.cast.write_text(CAST)
        self.manifest = self.root / "manifest.json"
        self.spent = self.root / "spent.json"

    def tearDown(self) -> None:
        self.temp.cleanup()

    def run_synth(self, *extra: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [
                sys.executable,
                str(SYNTH),
                "--lines",
                str(self.lines.parent),
                "--cast",
                str(self.cast),
                "--manifest",
                str(self.manifest),
                "--spent",
                str(self.spent),
                "--out",
                str(self.root / "audio"),
                *extra,
            ],
            capture_output=True,
            text=True,
        )


class DryRun(Fixture):
    def test_dry_run_writes_a_manifest_and_no_audio(self):
        result = self.run_synth("--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = json.loads(self.manifest.read_text())
        self.assertEqual(len(manifest["lines"]), 2)
        self.assertFalse((self.root / "audio").exists())

    def test_dry_run_refuses_a_budget_it_would_exceed(self):
        result = self.run_synth("--dry-run", "--budget", "10")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("budget", result.stderr)
        self.assertFalse(self.manifest.exists())

    def test_an_empty_spoken_string_falls_back_to_display(self):
        result = self.run_synth("--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = json.loads(self.manifest.read_text())
        first = manifest["lines"]["H01-1000105-0"]
        self.assertEqual(first["characters"], len("Jaunas Robinas iš Loksli paliko savo kraštą."))

    def test_a_dry_run_manifest_does_not_hide_real_work(self):
        first = self.run_synth("--dry-run")
        self.assertEqual(first.returncode, 0, first.stderr)
        second = self.run_synth("--dry-run")
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertIn("2 pending", second.stderr)
        self.assertNotIn("0 pending", second.stderr)

    def test_a_manifest_with_audio_on_disk_is_not_repeated(self):
        first = self.run_synth("--dry-run")
        self.assertEqual(first.returncode, 0, first.stderr)
        manifest = json.loads(self.manifest.read_text())
        for entry in manifest["lines"].values():
            path = self.root / "audio" / entry["path"]
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"ogg")
        second = self.run_synth("--dry-run")
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertIn("0 pending", second.stderr)


    def test_lines_without_a_playback_lane_are_not_synthesised(self):
        result = self.run_synth("--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = json.loads(self.manifest.read_text())
        self.assertEqual(len(manifest["lines"]), 2)
        self.assertNotIn("H01-1000349-0", manifest["lines"])
        self.assertIn("2 lines, 2 pending", result.stderr)


class LineTable(unittest.TestCase):
    def test_shipped_line_tables_carry_display_text_for_every_line(self):
        import tomllib

        for path in sorted((HERE / "lines").glob("*.toml")):
            document = tomllib.loads(path.read_text())
            self.assertTrue(document["line"], path)
            for line in document["line"]:
                self.assertTrue(line["display"].strip(), f"{path}: empty display")


if __name__ == "__main__":
    unittest.main()
