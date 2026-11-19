"""Offline tests for the language-pack assembler. No game data or network."""

from __future__ import annotations

import json
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
PACK = HERE / "pack.py"

LINES = """
[meta]
mission = "H01"
level_code = "HA"

[[line]]
id = 1000105
index = 0
display = "Jaunas Robinas grįžo."
spoken = ""
voice = "narrator"
provenance = "machine-draft"
"""


class PackAssembly(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.lines = self.root / "lines" / "h01.toml"
        self.lines.parent.mkdir(parents=True)
        self.lines.write_text(LINES)

        retail = self.root / "retail"
        text = retail / "2047" / "data" / "Text"
        (text / "Dialogues").mkdir(parents=True)
        (text / "Level.res").write_bytes(b"SRES-test")
        (text / "Dialogues" / "DLG_HB_D00_S00_Robin.wav").write_bytes(b"english")
        red = retail / "DATA" / "Text"
        red.mkdir(parents=True)
        # mission text, picture, dialogue_count=0, popup count/table,
        # debrief 4 words, briefing 2 words
        descriptor = [
            1_000_017,
            1_000_007,
            0,
            23,
            1_000_105,
            *[0] * 23,
            1,
            1_000_349,
            1,
            1_000_350,
            6,
            1_000_283,
        ]
        (red / "RHLevelHA.red").write_bytes(struct.pack(f"<{len(descriptor)}I", *descriptor))

        self.dump = self.root / "level.json"
        self.dump.write_text("{}")

        self.tool = self.root / "translate_level_res"
        self.tool.write_text(
            "#!/usr/bin/env python3\n"
            "import argparse, pathlib\n"
            "p = argparse.ArgumentParser()\n"
            "p.add_argument('--input'); p.add_argument('--translations'); p.add_argument('--output')\n"
            "a = p.parse_args()\n"
            "d = pathlib.Path(a.output); d.mkdir(parents=True, exist_ok=True)\n"
            "(d / 'Level.res').write_bytes(b'translated')\n"
        )
        self.tool.chmod(0o755)

        self.manifest = self.root / "manifest.json"
        self.manifest.write_text(
            json.dumps(
                {
                    "characters": 22,
                    "lines": {
                        "H01-1000105-0": {
                            "path": "Data/Text/Narration/1000105_0.ogg",
                            "characters": 22,
                            "voice": "lt-LT-Chirp3-HD-Charon",
                            "provenance": "machine-draft",
                        }
                    },
                }
            )
        )
        narration = self.root / "audio" / "Data" / "Text" / "Narration"
        narration.mkdir(parents=True)
        (narration / "1000105_0.ogg").write_bytes(b"lithuanian")

    def run_pack(self) -> subprocess.CompletedProcess:
        return subprocess.run(
            [
                sys.executable,
                str(PACK),
                "--retail",
                str(self.root / "retail"),
                "--dump",
                str(self.dump),
                "--lines",
                str(self.lines.parent),
                "--manifest",
                str(self.manifest),
                "--audio",
                str(self.root / "audio"),
                "--tool",
                str(self.tool),
                "--out",
                str(self.root / "out"),
            ],
            capture_output=True,
            text=True,
        )

    def test_narration_lands_where_the_game_probes_for_it(self):
        result = self.run_pack()
        self.assertEqual(result.returncode, 0, result.stderr)
        data = self.root / "out" / "1063" / "Data"
        self.assertTrue((data / "Text" / "Narration" / "1000105_0.ogg").is_file())
        self.assertFalse((data / "Data").exists(), "no doubled Data/ segment")

    def test_descriptor_name_uses_the_level_code(self):
        result = self.run_pack()
        self.assertEqual(result.returncode, 0, result.stderr)
        data = self.root / "out" / "1063" / "Data"
        self.assertTrue((data / "Text" / "Level.res").is_file())
        self.assertTrue(
            (data / "Text" / "Dialogues" / "DLG_HB_D00_S00_Robin.wav").is_file()
        )


if __name__ == "__main__":
    unittest.main()
