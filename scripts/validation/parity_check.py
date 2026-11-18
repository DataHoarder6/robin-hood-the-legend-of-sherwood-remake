#!/usr/bin/env python3
"""Cross-platform replay-hash parity gate.

Replay one recording with two binaries built from the same commit — the native
Linux client and the Windows release client under Wine — and diff the
per-25-frame state-hash chain the replay runtime already prints and verifies.

The hash surface is the shipping one: `robin_engine::replay::state_hash`, sampled
every `STATE_HASH_INTERVAL` (25) frames while recording and re-checked while
playing back (`crates/robin_rs/src/game_session/runtime.rs`, "Replay hash OK @
frame N: <hash>" at DEBUG). This script adds no second replay format and no
second hash function; it extracts those existing lines and compares them.

Exit status: 0 identical, 1 not identical (diverged, an unusable run, or
different build identities), 3 a prerequisite is missing. Nothing here can exit
0 without two complete, non-empty, equal chains.

Wine is not Windows. This proves the toolchain/FP/locale class of divergence
for the simulation; it does not exercise MSVC's runtime, WinDivert or Known
Folder paths, or a Windows scheduler. See docs/TESTING.md.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import os
import re
import shutil
import subprocess
import sys
import tempfile

EXIT_IDENTICAL = 0
EXIT_NOT_IDENTICAL = 1
EXIT_PREREQUISITE = 3

HASH_LINE = re.compile(r"Replay hash OK @ frame (\d+): ([0-9a-f]{16})\b")
DESYNC_LINE = re.compile(
    r"(?:Replay desync at frame \d+: expected [0-9a-f]{16}, got [0-9a-f]{16}"
    r"|replay desync:.*)"
)
VERSION_LINE = re.compile(r"^robin \S+$")

CONTEXT_SAMPLES = 2


@dataclass(frozen=True)
class Chain:
    """One binary's sampled state hashes, in frame order."""

    samples: tuple[tuple[int, str], ...]

    def values(self) -> dict[int, str]:
        return dict(self.samples)

    def __len__(self) -> int:
        return len(self.samples)


@dataclass(frozen=True)
class Divergence:
    frame: int
    linux: str | None
    windows: str | None
    reason: str
    context: tuple[tuple[int, str | None, str | None], ...]


@dataclass(frozen=True)
class Run:
    label: str
    version: str
    chain: Chain
    desyncs: tuple[str, ...]
    returncode: int
    log: Path
    problem: str | None = None


def parse_hash_chain(text: str) -> Chain:
    """Extract the existing per-25-frame hash lines, first sample per frame wins."""
    samples: dict[int, str] = {}
    for match in HASH_LINE.finditer(text):
        frame = int(match.group(1))
        value = match.group(2)
        if frame in samples and samples[frame] != value:
            raise ValueError(
                f"frame {frame} was sampled twice with {samples[frame]} and {value}"
            )
        samples[frame] = value
    return Chain(tuple(sorted(samples.items())))


def parse_desync_lines(text: str) -> tuple[str, ...]:
    """Recorded-hash or modal-policy desyncs: the replayed run is not trustworthy."""
    return tuple(
        line.strip() for line in text.splitlines() if DESYNC_LINE.search(line)
    )


def parse_version(text: str) -> str:
    """The `--version` banner, which carries the build's commit suffix."""
    for line in text.splitlines():
        if VERSION_LINE.match(line.strip()):
            return line.strip()
    return ""


def compare(linux: Chain, windows: Chain) -> Divergence | None:
    """First frame where the two chains differ, or None when they are identical."""
    left = linux.values()
    right = windows.values()
    frames = sorted(set(left) | set(right))
    for frame in frames:
        if left.get(frame) == right.get(frame):
            continue
        reason = (
            "state hash mismatch"
            if frame in left and frame in right
            else "frame sampled by only one binary"
        )
        position = frames.index(frame)
        window = frames[max(0, position - CONTEXT_SAMPLES):position + CONTEXT_SAMPLES + 1]
        context = tuple((item, left.get(item), right.get(item)) for item in window)
        return Divergence(frame, left.get(frame), right.get(frame), reason, context)
    return None


def describe_divergence(divergence: Divergence) -> str:
    lines = [
        f"parity: DIVERGED at frame {divergence.frame}: {divergence.reason}",
        f"parity:   linux   = {divergence.linux or '<not sampled>'}",
        f"parity:   windows = {divergence.windows or '<not sampled>'}",
        "parity: surrounding chain:",
    ]
    for frame, linux, windows in divergence.context:
        marker = "->" if frame == divergence.frame else "  "
        lines.append(
            f"parity: {marker} frame {frame:>6} "
            f"linux={linux or '<not sampled>':>16} "
            f"windows={windows or '<not sampled>':>16}"
        )
    return "\n".join(lines)


def verdict_exit(verdict: str) -> int:
    """The gate's three outcomes; every other verdict is a failed comparison."""
    return {
        "identical": EXIT_IDENTICAL,
        "not-identical": EXIT_NOT_IDENTICAL,
        "prerequisite": EXIT_PREREQUISITE,
    }[verdict]


def directory_problems(
    linux: Path, windows: Path, replay: Path, data_dir: Path,
    linux_dir: Path, windows_dir: Path,
) -> list[str]:
    """Everything the run needs before it starts; one line per missing input."""
    problems = []
    for label, path in (("linux binary", linux), ("windows executable", windows)):
        if not path.is_file():
            problems.append(f"{label} not found: {path}")
        elif label == "linux binary" and not (path.stat().st_mode & 0o111):
            problems.append(f"linux binary is not executable: {path}")
    if not replay.is_file():
        problems.append(f"replay not found: {replay}")
    if not data_dir.is_dir():
        problems.append(f"data root not found: {data_dir}")
    else:
        data = find_child(data_dir, "data")
        configuration = find_child(data, "configuration") if data else None
        if configuration is None:
            problems.append(
                f"data root has no Data/Configuration (original casing supported): {data_dir}"
            )
    for label, directory in (("linux", linux_dir), ("windows", windows_dir)):
        if not (directory / "assets" / "core-datadir").is_dir():
            problems.append(
                f"{label} install directory has no assets/core-datadir: {directory}"
            )
    return problems


def find_child(directory: Path, name: str) -> Path | None:
    """Case-insensitive single-level lookup for the original distribution casing."""
    if not directory.is_dir():
        return None
    for child in directory.iterdir():
        if child.name.lower() == name.lower():
            return child
    return None


def wine_problem(wine: str, winepath: str, version_runner=subprocess.run) -> str | None:
    """Wine and its path mapper must answer before any replay is attempted."""
    executable = shutil.which(wine)
    if executable is None:
        return f"wine not found: {wine}"
    try:
        result = version_runner(
            [executable, "--version"], capture_output=True, text=True, timeout=120
        )
    except OSError as error:
        return f"wine could not run: {error}"
    except subprocess.TimeoutExpired:
        return "wine --version timed out"
    if result.returncode != 0:
        return f"wine --version failed ({result.returncode}): {result.stderr.strip()}"
    if not result.stdout.startswith("wine-"):
        return f"unexpected wine version output: {result.stdout.strip()!r}"
    if shutil.which(winepath) is None:
        return f"{winepath} not found; it ships with wine and maps the replay path"
    return None


def run_problem(run: Run) -> str | None:
    """Why a finished process cannot be compared; None when it can."""
    if run.problem:
        return run.problem
    if run.desyncs:
        return f"reported a desync against the recording: {run.desyncs[0]}"
    if run.returncode != 0:
        return f"exited {run.returncode}"
    if not len(run.chain):
        return "printed no state-hash samples"
    return None


def launch(
    label: str, command: list[str], cwd: Path, environment: dict[str, str],
    timeout: float, log: Path,
) -> tuple[int, str]:
    try:
        result = subprocess.run(
            command,
            cwd=cwd,
            env=environment,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as error:
        output = (error.stderr or "") + (error.stdout or "")
        log.write_text(output)
        return -1, output
    except OSError as error:
        log.write_text(str(error) + "\n")
        return -2, str(error)
    output = result.stdout + result.stderr
    log.write_text(output)
    return result.returncode, output


def replay_run(
    label: str, launcher: list[str], install: Path, environment: dict[str, str],
    replay_argument: str, timeout: float, log_dir: Path,
) -> Run:
    log = log_dir / f"{label}.log"
    version_status, version_output = launch(
        label, [*launcher, "--version"], install, environment, timeout,
        log_dir / f"{label}.version.log",
    )
    version = parse_version(version_output) if version_status == 0 else ""
    returncode, output = launch(
        label,
        [*launcher, "--headless", "--no-sound", "--http-server", "0",
         "--replay", replay_argument],
        install,
        environment,
        timeout,
        log,
    )
    problem = None
    if version_status != 0 or not version:
        problem = f"--version failed ({version_status}): {version_output.strip()[:200]}"
    elif returncode == -1:
        problem = f"did not finish within {timeout:.0f}s"
    elif returncode == -2:
        problem = f"could not be started: {output.strip()[:200]}"
    try:
        chain = parse_hash_chain(output)
    except ValueError as error:
        chain = Chain(())
        problem = problem or f"ambiguous hash chain: {error}"
    return Run(
        label=label,
        version=version,
        chain=chain,
        desyncs=parse_desync_lines(output),
        returncode=returncode,
        log=log,
        problem=problem,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--linux", required=True, type=Path,
                        help="native Linux robin built from the target commit")
    parser.add_argument("--windows", required=True, type=Path,
                        help="Windows robin.exe built from the same commit")
    parser.add_argument("--replay", required=True, type=Path,
                        help="recording made by that commit (any replay the client accepts)")
    parser.add_argument("--data-dir", required=True, type=Path,
                        help="game data root containing Data/ (original casing supported)")
    parser.add_argument("--linux-dir", type=Path,
                        help="install directory run as the Linux working directory "
                             "(default: the binary's parent)")
    parser.add_argument("--windows-dir", type=Path,
                        help="install directory run as the Windows working directory "
                             "(default: the executable's parent)")
    parser.add_argument("--wine", default="wine", help="wine executable (default: wine)")
    parser.add_argument("--winepath", default="winepath",
                        help="winepath executable (default: winepath)")
    parser.add_argument("--timeout", type=float, default=900.0,
                        help="seconds per replay process (default: 900)")
    parser.add_argument("--log-dir", type=Path,
                        help="keep each run's full output here (default: a new temp dir)")
    arguments = parser.parse_args(argv)

    linux = arguments.linux.resolve()
    windows = arguments.windows.resolve()
    replay = arguments.replay.resolve()
    data_dir = arguments.data_dir.resolve()
    linux_dir = (arguments.linux_dir or linux.parent).resolve()
    windows_dir = (arguments.windows_dir or windows.parent).resolve()

    problems = directory_problems(
        linux, windows, replay, data_dir, linux_dir, windows_dir
    )
    if not problems:
        problem = wine_problem(arguments.wine, arguments.winepath)
        if problem:
            problems.append(problem)
    if problems:
        for problem in problems:
            print(f"parity: missing prerequisite: {problem}", file=sys.stderr)
        return verdict_exit("prerequisite")

    log_dir = arguments.log_dir or Path(tempfile.mkdtemp(prefix="parity-check-"))
    log_dir.mkdir(parents=True, exist_ok=True)

    environment = dict(
        os.environ,
        RUST_LOG="debug",
        ROBINHOOD_DATA_DIR=str(data_dir),
    )
    windows_environment = dict(environment, WINEDEBUG="-all")

    try:
        converted = subprocess.run(
            [arguments.winepath, "-w", str(replay)],
            env=windows_environment, capture_output=True, text=True, timeout=120,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        print(
            f"parity: missing prerequisite: winepath could not run: {error}",
            file=sys.stderr,
        )
        return verdict_exit("prerequisite")
    if converted.returncode != 0 or not converted.stdout.strip():
        print(
            "parity: missing prerequisite: winepath could not map the replay path: "
            f"{converted.stderr.strip()}",
            file=sys.stderr,
        )
        return verdict_exit("prerequisite")
    windows_replay = converted.stdout.strip()

    native = replay_run(
        "linux", [str(linux)], linux_dir, environment, str(replay),
        arguments.timeout, log_dir,
    )
    under_wine = replay_run(
        "windows", [shutil.which(arguments.wine) or arguments.wine, str(windows)],
        windows_dir, windows_environment, windows_replay,
        arguments.timeout, log_dir,
    )

    print(f"parity: linux   {native.version or '<no version>'}  {linux}")
    print(f"parity: windows {under_wine.version or '<no version>'}  {windows} (wine)")
    print(f"parity: replay  {replay}")
    print(f"parity: data    {data_dir}", flush=True)

    failures = []
    if native.version and under_wine.version and native.version != under_wine.version:
        failures.append(
            "binaries are not the same build: "
            f"linux={native.version!r} windows={under_wine.version!r}"
        )
    for run in (native, under_wine):
        problem = run_problem(run)
        if problem:
            failures.append(f"{run.label} replay {problem}: {run.log}")
    if failures:
        for failure in failures:
            print(f"parity: FAIL {failure}", file=sys.stderr)
        return verdict_exit("not-identical")

    divergence = compare(native.chain, under_wine.chain)
    if divergence:
        print(describe_divergence(divergence), file=sys.stderr)
        print(f"parity: logs kept in {log_dir}", file=sys.stderr)
        return verdict_exit("not-identical")

    first, last = native.chain.samples[0][0], native.chain.samples[-1][0]
    print(
        f"parity: identical: {len(native.chain)} sampled frames match "
        f"(frames {first}..{last})"
    )
    if arguments.log_dir is None:
        shutil.rmtree(log_dir, ignore_errors=True)
    else:
        print(f"parity: logs kept in {log_dir}")
    return verdict_exit("identical")


if __name__ == "__main__":
    sys.exit(main())
