"""Unit tests for OS-supervised harness children.

No process is started here: the point is to prove that a child cannot be
launched without a unit name, a hard runtime budget and a bounded stop.
"""

from __future__ import annotations

from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

import supervised


class UnitNameTest(unittest.TestCase):
    def test_names_are_unique_per_scope_and_systemd_safe(self):
        first = supervised.unit_name("H35/host/1")
        second = supervised.unit_name("H35/host/2")
        self.assertNotEqual(first, second)
        self.assertRegex(first, r"^rh-e2e-[A-Za-z0-9_.-]+$")
        self.assertEqual(first, supervised.unit_name("H35/host/1"))


class CommandTest(unittest.TestCase):
    def test_command_carries_a_ttl_and_a_control_group_kill(self):
        argv = supervised.command(["/bin/robin", "--server"], unit="rh-e2e-x",
                                  budget_s=130, env={"ROBIN_X": "1"})
        self.assertEqual(argv[:2], ["systemd-run", "--user"])
        self.assertIn("--unit", argv)
        self.assertIn("rh-e2e-x", argv)
        self.assertIn("--property=RuntimeMaxSec=130s", argv)
        self.assertIn("--property=KillMode=control-group", argv)
        self.assertIn("--property=TimeoutStopSec=10s", argv)
        self.assertIn("--setenv=ROBIN_X=1", argv)
        self.assertEqual(argv[-3:], ["--", "/bin/robin", "--server"])

    def test_zero_budget_is_refused(self):
        with self.assertRaises(ValueError):
            supervised.command(["/bin/true"], unit="rh-e2e-x", budget_s=0)

    def test_missing_systemd_is_a_prerequisite_failure(self):
        with patch.object(supervised.shutil, "which", return_value=None):
            with self.assertRaises(supervised.SupervisionUnavailable):
                supervised.require_supervisor()


class StopTest(unittest.TestCase):
    def test_stop_is_bounded_and_reports_failures(self):
        def fail(argv, **_kwargs):
            class R:
                returncode = 1
                stderr = "Unit not found"
            return R()
        with patch.object(supervised.subprocess, "run", side_effect=fail) as run:
            failures = supervised.stop("rh-e2e-x")
        self.assertEqual(len(failures), 2)  # stop, then SIGKILL
        self.assertEqual(run.call_count, 2)
        for call in run.call_args_list:
            self.assertEqual(call.kwargs["timeout"], 10)

    def test_successful_stop_does_not_escalate(self):
        class R:
            returncode = 0
            stderr = ""
        with patch.object(supervised.subprocess, "run", return_value=R()) as run:
            self.assertEqual(supervised.stop("rh-e2e-x"), [])
        run.assert_called_once()


if __name__ == "__main__":
    unittest.main()
