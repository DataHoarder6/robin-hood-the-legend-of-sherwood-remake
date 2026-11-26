#!/usr/bin/env python3
"""Scriptable evidence for the Android APK boot contract.

This checker never claims a boot it did not observe. It has three entry
points:

* ``--check-apk APK`` inspects a packaged APK on the host (overlay inventory,
  size and SHA-256, native library, manifest schema). No device is needed.
* ``--boot APK`` drives a clean install and boot through adb, requires log
  evidence for each stage, and writes a JSON evidence file.
* ``--self-test`` runs the deterministic fixture tests next to this file; they
  inject a fake adb, a synthetic APK and a synthetic source tree.

A stage's log needle is only trusted when the tree declares the source file
that emits it, and that file is present under ``--source-root``. No touch
producer exists in this tree, so a real ``--boot`` is BLOCKED before install
until one is added — a native-library check is not a substitute for a device
run. BLOCKED exits 3, a failed observation exits 1, PASS exits 0.

Provenance: adapted from the unlanded v070-android fixture; markers are
grounded in the source tree instead of invented log strings.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import unittest
import zipfile
from dataclasses import dataclass
from pathlib import Path

BLOCKED_EXIT = 3
FAIL_EXIT = 1

DEVICE_CAPABILITY = (
    "an Android device or emulator reachable through adb and a signed APK "
    "built with the operator's signing key"
)
APK_CAPABILITY = (
    "a packaged, signed APK (operator signing key) containing the core overlay"
)
LOG_CAPABILITY = (
    "an observable log channel for the packaged Android build: a tracing "
    "producer under the declared source path and a confirmed adb logcat "
    "bridge (init_tracing installs no Android log layer)"
)

NATIVE_LIB = "lib/arm64-v8a/librobin_rs.so"
OVERLAY_MANIFEST = "assets/core-overlay-manifest.json"
OVERLAY_PREFIX = "assets/"

LAUNCH_COMPONENT = (
    "io.github.phiresky.robinhood/io.github.phiresky.robinhood.RobinHoodActivity"
)


@dataclass(frozen=True)
class Marker:
    """A stage's log needle and the source file that emits it."""

    needle: str
    producer: str | None


# Strings verified against the tree on 2026-11-26:
#   crates/robin_rs/src/android.rs:215 - "Mounted validated Android core
#     overlay ahead of shipping content"
#   crates/robin_rs/src/observability.rs:250 - "mission started: {mission}"
# Touch has no producer: the input layer emits typed commands, not a log line.
# A stage with producer=None is reported BLOCKED, never assumed to pass.
STAGES: dict[str, Marker] = {
    "overlay": Marker(
        "Mounted validated Android core overlay ahead of shipping content",
        "crates/robin_rs/src/android.rs",
    ),
    "mission": Marker(
        "mission started: ",
        "crates/robin_rs/src/observability.rs",
    ),
    "touch": Marker("", None),
}


def default_source_root() -> Path:
    """The worktree root, from android/scripts/."""
    return Path(__file__).resolve().parents[2]


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


@dataclass
class AdbResult:
    returncode: int
    stdout: str
    stderr: str = ""


class Adb:
    """Thin adb adapter; the only place that shells out to a real device."""

    def __init__(self, binary: str = "adb"):
        self.binary = binary

    def run(self, *args: str) -> AdbResult:
        try:
            proc = subprocess.run(
                [self.binary, *args],
                capture_output=True,
                text=True,
                timeout=60,
                check=False,
            )
        except FileNotFoundError as exc:
            return AdbResult(127, "", f"adb binary not found: {exc}")
        except subprocess.TimeoutExpired as exc:
            return AdbResult(124, "", f"adb command timed out: {exc}")
        return AdbResult(proc.returncode, proc.stdout, proc.stderr)


class FakeAdb(Adb):
    """Deterministic adb fixture used by the host-runnable contract tests."""

    def __init__(
        self,
        devices: list[str],
        boot_completed: str = "1",
        log_lines: list[str] | None = None,
        failures: set[str] | None = None,
    ):
        super().__init__(binary="fake-adb")
        self.devices = devices
        self.boot_completed = boot_completed
        self.log_lines = log_lines or []
        self.failures = failures or set()
        self.calls: list[str] = []

    def run(self, *args: str) -> AdbResult:
        call = " ".join(args)
        self.calls.append(call)
        if any(failure in call for failure in self.failures):
            return AdbResult(1, "", f"injected failure: {call}")
        if args == ("devices",):
            lines = ["List of devices attached"]
            lines.extend(f"{serial}\tdevice" for serial in self.devices)
            lines.append("")
            return AdbResult(0, "\n".join(lines))
        if args == ("shell", "getprop", "sys.boot_completed"):
            return AdbResult(0, self.boot_completed + "\n")
        if len(args) >= 2 and args[0] == "shell" and args[1] == "logcat":
            return AdbResult(0, "\n".join(self.log_lines))
        return AdbResult(0, "")


def detect_device(adb: Adb) -> str | None:
    result = adb.run("devices")
    if result.returncode != 0:
        return None
    for line in result.stdout.splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[1] == "device":
            return parts[0]
    return None


def marker_producers(
    source_root: Path, stages: dict[str, Marker] | None = None
) -> dict:
    """Report which stage needles are emitted by a declared source file."""
    stages = stages or STAGES
    result: dict[str, dict] = {}
    for stage, marker in stages.items():
        if marker.producer is None or not marker.needle:
            result[stage] = {
                "declared": False,
                "producer": None,
                "needle": None,
                "present": False,
            }
            continue
        try:
            text = (source_root / marker.producer).read_text()
        except OSError:
            present = False
        else:
            present = marker.needle in text
        result[stage] = {
            "declared": True,
            "producer": marker.producer,
            "needle": marker.needle,
            "present": present,
        }
    return result


def missing_log_capability(producers: dict) -> str:
    missing = [stage for stage, entry in producers.items() if not entry["present"]]
    return f"{LOG_CAPABILITY}; stages without a producer: {', '.join(missing)}"


def verify_apk(path: Path) -> dict:
    """Inspect a packaged APK without a device."""
    failures: list[str] = []
    checked_files = 0
    if not path.is_file():
        return {
            "status": "BLOCKED",
            "missing_capability": APK_CAPABILITY,
            "failures": [],
            "checked_files": 0,
        }
    try:
        with zipfile.ZipFile(path) as apk:
            names = set(apk.namelist())
            if NATIVE_LIB not in names:
                failures.append(f"missing native library {NATIVE_LIB}")
            if OVERLAY_MANIFEST not in names:
                failures.append(f"missing core overlay manifest {OVERLAY_MANIFEST}")
                return {
                    "status": "FAIL",
                    "failures": failures,
                    "checked_files": 0,
                }
            try:
                manifest = json.loads(apk.read(OVERLAY_MANIFEST))
            except (ValueError, KeyError) as exc:
                failures.append(f"unreadable core overlay manifest: {exc}")
                return {
                    "status": "FAIL",
                    "failures": failures,
                    "checked_files": 0,
                }
            if manifest.get("schema") != 1:
                failures.append(
                    f"unsupported core overlay schema {manifest.get('schema')}; expected 1"
                )
            entries = manifest.get("files")
            if not isinstance(entries, list) or not entries:
                failures.append("core overlay manifest has no files")
                entries = []
            paths = [entry.get("path") for entry in entries]
            if paths != sorted(paths) or len(paths) != len(set(paths)):
                failures.append("core overlay manifest paths must be unique and sorted")
            for entry in entries:
                relative = entry.get("path", "")
                if (
                    not isinstance(relative, str)
                    or not relative.startswith("Data/")
                    or ".." in relative
                ):
                    failures.append(f"invalid core overlay path: {relative}")
                    continue
                asset = OVERLAY_PREFIX + relative
                if asset not in names:
                    failures.append(f"missing core overlay asset {relative}")
                    continue
                data = apk.read(asset)
                if len(data) != entry.get("size"):
                    failures.append(
                        f"core overlay asset {relative} has {len(data)} bytes; "
                        f"manifest requires {entry.get('size')}"
                    )
                digest = sha256(data)
                if digest != entry.get("sha256"):
                    failures.append(
                        f"core overlay asset {relative} failed SHA-256 validation"
                    )
                checked_files += 1
    except (zipfile.BadZipFile, OSError) as exc:
        failures.append(f"unreadable APK: {exc}")
    return {
        "status": "FAIL" if failures else "PASS",
        "failures": failures,
        "checked_files": checked_files,
    }


def _write_evidence(evidence_path: Path | None, result: dict) -> None:
    if evidence_path is None:
        return
    evidence_path.parent.mkdir(parents=True, exist_ok=True)
    evidence_path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")


def boot_check(
    adb: Adb,
    apk: Path | None,
    evidence_path: Path | None = None,
    source_root: Path | None = None,
    stages: dict[str, Marker] | None = None,
) -> dict:
    """Drive and record one APK boot check; never fabricate a pass."""
    stages = stages or STAGES
    if apk is None or not apk.is_file():
        return {"status": "BLOCKED", "missing_capability": APK_CAPABILITY}

    device = detect_device(adb)
    if device is None:
        return {"status": "BLOCKED", "missing_capability": DEVICE_CAPABILITY}

    overlay = verify_apk(apk)
    if overlay["status"] == "BLOCKED":
        return overlay
    if overlay["status"] == "FAIL":
        result = {
            "status": "FAIL",
            "device": device,
            "overlay": overlay,
            "failures": overlay["failures"],
        }
        _write_evidence(evidence_path, result)
        return result

    producers = marker_producers(source_root or default_source_root(), stages)
    if any(not entry["present"] for entry in producers.values()):
        result = {
            "status": "BLOCKED",
            "missing_capability": missing_log_capability(producers),
            "device": device,
            "apk": {"path": str(apk), "sha256": sha256(apk.read_bytes())},
            "overlay": overlay,
            "markers": producers,
            "failures": [],
        }
        _write_evidence(evidence_path, result)
        return result

    failures: list[str] = []
    install = adb.run("install", "-r", str(apk))
    if install.returncode != 0:
        failures.append(f"clean install failed: {install.stderr.strip()}")
    launch = adb.run("shell", "am", "start", "-n", LAUNCH_COMPONENT)
    if launch.returncode != 0:
        failures.append(f"launch failed: {launch.stderr.strip()}")

    boot = adb.run("shell", "getprop", "sys.boot_completed")
    if boot.stdout.strip() != "1":
        failures.append("device did not report sys.boot_completed=1")

    log = adb.run("shell", "logcat", "-d", "-t", "500").stdout
    seen = {name: marker.needle in log for name, marker in stages.items()}
    for name, observed in seen.items():
        if not observed:
            failures.append(f"missing boot evidence marker: {name}")

    result = {
        "status": "FAIL" if failures else "PASS",
        "device": device,
        "apk": {"path": str(apk), "sha256": sha256(apk.read_bytes())},
        "overlay": overlay,
        "markers": seen,
        "failures": failures,
        "install_variant": "debug-or-release-signed-by-operator",
    }
    _write_evidence(evidence_path, result)
    return result


def exit_code(result: dict) -> int:
    return {"PASS": 0, "FAIL": FAIL_EXIT, "BLOCKED": BLOCKED_EXIT}[result["status"]]


def self_test_suite() -> unittest.TestSuite:
    """Discover the fixture tests by directory, not by dotted module name.

    ``loadTestsFromNames`` splits on dots: a worktree under ``.worktrees/``
    turns the path into a module name and fails before any test runs.
    """
    directory = Path(__file__).resolve().parent
    return unittest.defaultTestLoader.discover(
        str(directory), pattern="test_apk_boot_check.py"
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check-apk", type=Path, help="inspect a packaged APK on the host")
    parser.add_argument("--boot", type=Path, help="run the device boot check with adb")
    parser.add_argument("--adb", default="adb", help="adb binary for --boot")
    parser.add_argument("--evidence", type=Path, help="write JSON evidence for --boot")
    parser.add_argument(
        "--source-root",
        type=Path,
        default=None,
        help="worktree root whose source declares each stage's log producer",
    )
    parser.add_argument(
        "--self-test", action="store_true", help="run the deterministic fixture tests"
    )
    args = parser.parse_args(argv)

    if args.self_test:
        result = unittest.TextTestRunner(verbosity=2).run(self_test_suite())
        return 0 if result.wasSuccessful() else 1

    if args.check_apk is not None:
        result = verify_apk(args.check_apk)
    elif args.boot is not None:
        result = boot_check(Adb(args.adb), args.boot, args.evidence, args.source_root)
    else:
        parser.error("pass one of --check-apk, --boot or --self-test")
        return 2

    print(json.dumps(result, indent=2, sort_keys=True))
    return exit_code(result)


if __name__ == "__main__":
    sys.exit(main())
