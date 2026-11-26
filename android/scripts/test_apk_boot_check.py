#!/usr/bin/env python3
"""Fixture tests for the Android APK boot contract checker.

These tests never touch a device and never build an APK: they inject a
deterministic fake adb, a synthetic APK and a synthetic source tree. A real
device/boot run stays a separate, capability-gated step; no fixture reads as a
boot that happened.
"""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import apk_boot_check as boot  # noqa: E402


def make_apk(path: Path, *, corrupt: bool = False, omit_lib: bool = False) -> Path:
    """Build a minimal APK carrying a valid core overlay manifest."""
    payload = b"core-overlay-fixture-bytes"
    digest = boot.sha256(payload)
    manifest = {
        "schema": 1,
        "files": [
            {
                "path": "Data/core/fixture.bin",
                "size": len(payload),
                "sha256": "0" * 64 if corrupt else digest,
            }
        ],
    }
    with zipfile.ZipFile(path, "w") as apk:
        if not omit_lib:
            apk.writestr("lib/arm64-v8a/librobin_rs.so", b"ELF fixture")
        apk.writestr("assets/core-overlay-manifest.json", json.dumps(manifest))
        apk.writestr("assets/Data/core/fixture.bin", payload)
    return path


def all_stages() -> dict[str, boot.Marker]:
    """A tree-gounded stage map with a synthetic touch producer for tests."""
    stages = dict(boot.STAGES)
    stages["touch"] = boot.Marker("touch gesture applied", "crates/robin_rs/src/touch_probe.rs")
    return stages


def fake_sources(root: Path, stages: dict[str, boot.Marker]) -> Path:
    """A synthetic tree declaring all stages, so the PASS path is testable."""
    for marker in stages.values():
        if marker.producer is None:
            raise AssertionError("a fake source cannot declare an absent producer")
        path = root / marker.producer
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f'tracing::info!("{marker.needle}");\n')
    return root


class VerifyApkTests(unittest.TestCase):
    def test_valid_apk_verifies_every_overlay_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            apk = make_apk(Path(tmp) / "app.apk")
            result = boot.verify_apk(apk)
            self.assertEqual(result["status"], "PASS", result)
            self.assertEqual(result["checked_files"], 1)

    def test_corrupt_overlay_fails_with_named_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            apk = make_apk(Path(tmp) / "app.apk", corrupt=True)
            result = boot.verify_apk(apk)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(
                any("Data/core/fixture.bin" in f for f in result["failures"]),
                result["failures"],
            )

    def test_missing_native_library_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            apk = make_apk(Path(tmp) / "app.apk", omit_lib=True)
            result = boot.verify_apk(apk)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("librobin_rs.so" in f for f in result["failures"]))


class MarkerGroundingTests(unittest.TestCase):
    def test_declared_markers_name_a_producer_in_this_tree(self):
        """No marker may name a string nobody in the repo emits."""
        producers = boot.marker_producers(boot.default_source_root())
        for stage in ("overlay", "mission"):
            self.assertTrue(producers[stage]["present"], producers[stage])
        # No touch producer exists in the tree; the harness must say so rather
        # than expect a log line that cannot be emitted.
        self.assertFalse(producers["touch"]["declared"], producers["touch"])

    def test_partial_producer_is_not_a_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            stages = {
                "overlay": boot.Marker("overlay-up", "crates/robin_rs/src/android.rs"),
                "mission": boot.Marker("mission-up", "crates/robin_rs/src/observability.rs"),
                "touch": boot.Marker("touch-up", "crates/robin_rs/src/input.rs"),
            }
            fake_sources(root, stages)
            (root / "crates/robin_rs/src/input.rs").write_text("// no producer\n")
            producers = boot.marker_producers(root, stages)
            self.assertFalse(producers["touch"]["present"])
            self.assertIn("touch", boot.missing_log_capability(producers))


class BootCheckTests(unittest.TestCase):
    def test_no_device_is_blocked_with_named_capability(self):
        fake = boot.FakeAdb(devices=[])
        with tempfile.TemporaryDirectory() as tmp:
            apk = make_apk(Path(tmp) / "app.apk")
            result = boot.boot_check(fake, apk)
        self.assertEqual(result["status"], "BLOCKED")
        self.assertIn("adb", result["missing_capability"])
        self.assertIn("signing key", result["missing_capability"])
        self.assertFalse(
            [c for c in fake.calls if "install" in c or "am start" in c],
            "a blocked run must not install or launch anything",
        )

    def test_missing_apk_is_blocked_not_fabricated(self):
        fake = boot.FakeAdb(devices=["emulator-5554"])
        with tempfile.TemporaryDirectory() as tmp:
            result = boot.boot_check(fake, Path(tmp) / "absent.apk")
        self.assertEqual(result["status"], "BLOCKED")
        self.assertIn("APK", result["missing_capability"])
        self.assertEqual(fake.calls, [], "without an APK no adb call is justified")

    def test_no_producer_blocks_before_installing(self):
        fake = boot.FakeAdb(
            devices=["device-1"],
            boot_completed="1",
            log_lines=[
                boot.STAGES["overlay"].needle,
                boot.STAGES["mission"].needle,
            ],
        )
        with tempfile.TemporaryDirectory() as tmp:
            apk = make_apk(Path(tmp) / "app.apk")
            evidence = Path(tmp) / "evidence.json"
            result = boot.boot_check(fake, apk, evidence)
            self.assertEqual(result["status"], "BLOCKED", result)
            self.assertIn("touch", result["missing_capability"])
            self.assertIn("log channel", result["missing_capability"])
            self.assertEqual(
                fake.calls, ["devices"], "a missing producer must not trigger a boot"
            )
            saved = json.loads(evidence.read_text())
            self.assertEqual(saved["status"], "BLOCKED")
            self.assertFalse(saved["markers"]["touch"]["present"])

    def test_green_run_installs_boots_and_records_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            stages = all_stages()
            root = fake_sources(Path(tmp) / "src", stages)
            fake = boot.FakeAdb(
                devices=["emulator-5554"],
                boot_completed="1",
                log_lines=[marker.needle for marker in stages.values()],
            )
            apk = make_apk(Path(tmp) / "app.apk")
            evidence = Path(tmp) / "evidence.json"
            result = boot.boot_check(fake, apk, evidence, source_root=root, stages=stages)
            self.assertEqual(result["status"], "PASS", result)
            self.assertTrue(evidence.is_file())
            saved = json.loads(evidence.read_text())
            self.assertEqual(saved["status"], "PASS")
            self.assertEqual(saved["device"], "emulator-5554")
            self.assertEqual(saved["overlay"]["checked_files"], 1)
            self.assertTrue(any("install" in c for c in fake.calls))
            self.assertTrue(any("am start" in c for c in fake.calls))

    def test_boot_without_mission_marker_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            stages = all_stages()
            root = fake_sources(Path(tmp) / "src", stages)
            fake = boot.FakeAdb(
                devices=["device-1"],
                boot_completed="1",
                log_lines=[
                    stages["overlay"].needle,
                    stages["touch"].needle,
                ],
            )
            apk = make_apk(Path(tmp) / "app.apk")
            result = boot.boot_check(fake, apk, source_root=root, stages=stages)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("mission" in f for f in result["failures"]), result)

    def test_launch_failure_is_a_failed_boot(self):
        with tempfile.TemporaryDirectory() as tmp:
            stages = all_stages()
            root = fake_sources(Path(tmp) / "src", stages)
            fake = boot.FakeAdb(
                devices=["device-1"],
                boot_completed="1",
                log_lines=[marker.needle for marker in stages.values()],
                failures={"am start"},
            )
            apk = make_apk(Path(tmp) / "app.apk")
            result = boot.boot_check(fake, apk, source_root=root, stages=stages)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("am start" in f for f in result["failures"]), result)


class ExitCodeTests(unittest.TestCase):
    def test_blocked_has_distinct_exit_code(self):
        self.assertEqual(boot.exit_code({"status": "BLOCKED"}), boot.BLOCKED_EXIT)
        self.assertEqual(boot.exit_code({"status": "FAIL"}), 1)
        self.assertEqual(boot.exit_code({"status": "PASS"}), 0)

    def test_self_test_discovers_the_tests_under_a_dotted_worktree_path(self):
        self.assertGreater(boot.self_test_suite().countTestCases(), 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
