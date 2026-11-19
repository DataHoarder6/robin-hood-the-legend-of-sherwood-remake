#!/usr/bin/env python3
"""Extract a mission's text into a Lithuanian translation skeleton.

Reads the mission's `.red` descriptor (format documented in
`crates/robin_assets/src/res_descr.rs`) and the `dump_res` JSON of `Level.res`,
then writes one `[[line]]` per string with the English text in `display` and an
empty `spoken`:

    dump_res 2047/data/Text/Level.res > /tmp/level.json
    python3 scripts/voiceover/extract.py --dump /tmp/level.json \
        --red DATA/Text/RHLevelHA.red --mission H01 --output scripts/voiceover/lines/h01.toml
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import descriptors


SPEAKERS = {
    "Robin": "robin",
    "Marian": "marian",
    "Tuck": "tuck",
    "Sheriff": "sheriff",
}


def voice_for(descriptor, dump: dict, table_id: int, index: int) -> str:
    """Read the speaker from the dialogue clip's filename, if this is dialogue."""
    for text_table, sound_table, _portraits in descriptor.dialogues:
        if text_table != table_id:
            continue
        table = dump.get(str(sound_table), {})
        paths = table.get("paths", [])
        if index >= len(paths):
            break
        speaker = paths[index].rsplit("_", 1)[-1].rsplit(".", 1)[0]
        return SPEAKERS.get(speaker, "villager")
    return "narrator"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dump", required=True, type=Path, help="dump_res JSON")
    parser.add_argument("--red", required=True, type=Path)
    parser.add_argument("--mission", required=True, help="mission id, e.g. H01")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)

    resources = json.loads(args.dump.read_text())
    descriptor = descriptors.read(args.red)
    lines: list[str] = []
    total = 0
    for table_id in descriptor.text_tables():
        table = resources.get(str(table_id))
        if not table or "strings" not in table:
            raise SystemExit(f"{args.mission}: text table {table_id} is absent from the dump")
        for index, text in enumerate(table["strings"]):
            if text == "NOTUSED":
                continue
            lines.append(
                "\n".join(
                    [
                        "[[line]]",
                        f"id = {table_id}",
                        f"index = {index}",
                        f"display = {json.dumps(text, ensure_ascii=False)}",
                        'spoken = ""',
                        f'voice = "{voice_for(descriptor, resources, table_id, index)}"',
                        'provenance = "machine-draft"',
                    ]
                )
            )
            total += len(text)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        f"# {args.mission}: {len(lines)} lines, {total} characters of English.\n"
        + "\n\n".join(lines)
        + "\n"
    )
    print(f"{args.output}: {len(lines)} lines, {total} characters", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
