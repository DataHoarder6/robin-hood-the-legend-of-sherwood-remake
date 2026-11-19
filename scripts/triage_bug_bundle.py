#!/usr/bin/env python3
"""Turn a bug-report bundle into a row for docs/DEFECTS.md.

    scripts/triage_bug_bundle.py <bundle directory>
    scripts/triage_bug_bundle.py --check <bundle directory>

A bundle is written by the pause menu, `/BUG` or `POST /bug-report`; its shape
is documented in `docs/BUG_REPORTS.md`. This reads the fields a row needs and
prints the `--replay` command that reproduces the defect, so the row is written
from evidence instead of from memory.

The class and the check are printed as `?`: neither can be inferred from a
bundle. Choosing them is the diagnosis, and a script that guessed would be the
"probably fixed" row this ledger exists to prevent.

`--check` exits 1 when the bundle is not fit to file a row (no `bundle.json`,
no replay, no description) and prints why.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

CLASSES = (
    "wait",
    "authority",
    "desync",
    "generation",
    "identity",
    "presentation",
    "release",
    "data",
)
SYMPTOM_LIMIT = 100


def fail(message: str) -> int:
    print(f"triage: {message}", file=sys.stderr)
    return 2


def one_line(text: object, limit: int = SYMPTOM_LIMIT) -> str:
    line = " ".join(str(text).split())
    if len(line) > limit:
        line = line[: limit - 1] + "…"
    return line


def cell(text: str) -> str:
    """A Markdown table cell: no pipe can split it, no newline can end the row."""
    return one_line(text, 10_000).replace("|", "\\|")


def field(document: dict, *path: str, default=None):
    node: object = document
    for key in path:
        if not isinstance(node, dict) or key not in node:
            return default
        node = node[key]
    return node


def replay_files(bundle: Path) -> list[Path]:
    directory = bundle / "replay"
    if not directory.is_dir():
        return []
    return sorted(path for path in directory.iterdir() if path.is_file())


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Print the docs/DEFECTS.md row a bug-report bundle supports."
    )
    parser.add_argument("bundle", type=Path, help="the bug-reports/<id> directory")
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit 1 unless the bundle is fit to file a row",
    )
    arguments = parser.parse_args(argv)

    bundle = arguments.bundle
    if not bundle.is_dir():
        return fail(f"{bundle} is not a directory")
    document_path = bundle / "bundle.json"
    try:
        document = json.loads(document_path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return fail(f"{document_path} is missing")
    except (OSError, json.JSONDecodeError) as error:
        return fail(f"{document_path} is not readable JSON: {error}")
    if not isinstance(document, dict):
        return fail(f"{document_path} is not a JSON object")

    fingerprint = str(document.get("fingerprint") or "")
    description = one_line(document.get("description") or "")
    replays = replay_files(bundle)
    problems: list[str] = []
    if not description:
        problems.append("no description, so the row would have no symptom")
    if not replays:
        problems.append("no replay under replay/, so the row would have no reproduction")
    if len(fingerprint) != 16 or any(character not in "0123456789abcdef" for character in fingerprint):
        problems.append(f"fingerprint {fingerprint!r} is not 16 lowercase hex characters")

    identity = f"D-{fingerprint[:8]}" if len(fingerprint) >= 8 else "D-?"
    mission = field(document, "session", "transport", "mission_id", default="?")
    tick = field(document, "session", "tick", "simulation_tick", default="?")
    kind = field(document, "session", "kind", default="?")
    build = field(document, "build", "version", default="?")
    revision = field(document, "build", "git_rev", default=None)
    desync = field(document, "session", "desync", default=None)
    notes = document.get("notes") or []

    print(f"bundle   {bundle}")
    print(f"id       {identity}")
    print(f"class    ?  ({'|'.join(CLASSES)})")
    print(f"symptom  {description or '(none)'}")
    print(f"session  {kind}, mission {mission}, tick {tick}")
    print(f"build    {build}" + (f" ({revision})" if revision else ""))
    if isinstance(desync, dict):
        print(
            "desync   "
            f"hash {desync.get('current_state_hash', '?')}, "
            f"last compared frame {desync.get('last_compared_hash_frame', '?')}, "
            f"mismatches {desync.get('desync_count', '?')}"
        )
    if replays:
        for replay in replays:
            print(f"replay   {replay}")
        print(f"repro    robin --replay {replays[0]}")
    else:
        print("replay   (none)")
    if notes:
        print("notes    " + one_line("; ".join(str(note) for note in notes), 300))

    reproduction = (
        f"`robin --replay {replays[0]}`" if replays else "no reproduction captured"
    )
    print()
    print(
        f"| {identity} | ? | {cell(description) or '?'} | {reproduction} | ? | open |"
    )

    if problems:
        for problem in problems:
            print(f"triage: {problem}", file=sys.stderr)
        if arguments.check:
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
