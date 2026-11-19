#!/usr/bin/env python3
"""Read-only checks on docs/DEFECTS.md.

The ledger is only worth having if every row is a real row: one class from the
vocabulary the triage script prints, one status from the four, an id nobody
else uses, and a check that exists. A row that points at `H20` while no
scenario file carries that id is the ledger lying about what is proved.
"""

from __future__ import annotations

import re
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LEDGER = ROOT / "docs/DEFECTS.md"
SCENARIOS = ROOT / "scripts/validation/scenarios"
sys.path.insert(0, str(ROOT / "scripts"))

from triage_bug_bundle import CLASSES  # noqa: E402

STATUSES = {"open", "fixed", "deferred", "out-of-scope"}
ID_PATTERN = re.compile(r"^D(?:\d{2}|-[0-9a-f]{8})$")
SCENARIO_PATTERN = re.compile(r"^H\d+$")
TEST_NAME_PATTERN = re.compile(r"^[a-z][a-z0-9_]{8,}$")


def rows() -> list[list[str]]:
    table: list[list[str]] = []
    for line in LEDGER.read_text(encoding="utf-8").splitlines():
        if not line.startswith("| D"):
            continue
        cells = [cell.strip() for cell in re.split(r"(?<!\\)\|", line.strip())]
        table.append(cells[1:-1])
    return table


class DefectLedgerTests(unittest.TestCase):
    def test_the_ledger_has_rows(self) -> None:
        self.assertGreater(len(rows()), 0)

    def test_every_row_has_six_cells(self) -> None:
        for row in rows():
            self.assertEqual(len(row), 6, row)

    def test_ids_are_unique_and_well_formed(self) -> None:
        identifiers = [row[0] for row in rows()]
        self.assertEqual(len(identifiers), len(set(identifiers)))
        for identifier in identifiers:
            self.assertRegex(identifier, ID_PATTERN)

    def test_classes_come_from_the_one_vocabulary(self) -> None:
        for row in rows():
            self.assertIn(row[1], CLASSES, row)

    def test_statuses_come_from_the_four(self) -> None:
        for row in rows():
            self.assertIn(row[5], STATUSES, row)

    def test_a_fixed_row_names_a_check_that_exists(self) -> None:
        """A row's check is a scenario file or a test function, not a wish."""
        for row in rows():
            identifier, _, _, _, check, status = row
            if status != "fixed":
                continue
            for token in re.findall(r"`([^`]+)`", check):
                if SCENARIO_PATTERN.match(token):
                    self.assertTrue(
                        (SCENARIOS / f"{token}.json").is_file(),
                        f"{identifier} claims {token}, which has no scenario file",
                    )
                elif TEST_NAME_PATTERN.match(token):
                    found = subprocess.run(
                        ["git", "-C", str(ROOT), "grep", "-q", f"fn {token}", "--", "crates"],
                        capture_output=True,
                    )
                    self.assertEqual(
                        found.returncode,
                        0,
                        f"{identifier} names {token}, which no crate defines",
                    )


if __name__ == "__main__":
    unittest.main()
