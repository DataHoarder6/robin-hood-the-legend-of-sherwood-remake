#!/usr/bin/env python3
"""Assemble a `1063/` Lithuanian language pack from the line tables and audio.

    python3 scripts/voiceover/pack.py \
        --retail ~/Games/robin-hood-data --dump /tmp/level.json \
        --tool target/debug/translate_level_res --out build/pack

The pack is structurally complete and partially translated: `H01`/`H02`
strings and clips are Lithuanian, every other id and dialogue clip is the
retail English copy, so the pack validates and no mission can fail to find its
text.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

import descriptors

LOCALE_ROOT = "1063"
ENGLISH_LOCALE = "2047"


def resolve(root: Path, *parts: str) -> Path:
    """Walk `parts` case-insensitively; distributions spell Data/data/DATA."""
    current = root
    for part in parts:
        match = next(
            (child for child in current.iterdir() if child.name.lower() == part.lower()),
            None,
        )
        if match is None:
            raise SystemExit(f"{current}: no {part}")
        current = match
    return current


def load_lines(directory: Path) -> dict[str, dict]:
    missions: dict[str, dict] = {}
    for path in sorted(directory.glob("*.toml")):
        document = tomllib.loads(path.read_text())
        missions[path.stem.upper()] = document
    return missions


def wave_paths(descriptor: descriptors.Descriptor, dump: dict) -> dict[tuple[int, int], str]:
    """Map `(dialogue text table, index)` to the retail clip path it names."""
    mapping: dict[tuple[int, int], str] = {}
    for text_table, sound_table, _portraits in descriptor.dialogues:
        table = dump.get(str(sound_table))
        if not table or "paths" not in table:
            raise SystemExit(f"sound table {sound_table} is absent from the dump")
        for index, path in enumerate(table["paths"]):
            mapping[(text_table, index)] = path.replace("\\", "/")
    return mapping


def write_translations(missions: dict[str, dict], path: Path) -> None:
    blocks = []
    for document in missions.values():
        for line in document["line"]:
            blocks.append(
                "[[line]]\n"
                f"id = {line['id']}\n"
                f"index = {line['index']}\n"
                f"text = {json.dumps(line['display'], ensure_ascii=False)}"
            )
    path.write_text("\n".join(blocks) + "\n")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--retail", required=True, type=Path)
    parser.add_argument("--dump", required=True, type=Path)
    parser.add_argument("--lines", type=Path, default=Path("scripts/voiceover/lines"))
    parser.add_argument("--manifest", type=Path, default=Path("scripts/voiceover/manifest.json"))
    parser.add_argument("--audio", type=Path, default=Path("build/voiceover"))
    parser.add_argument("--tool", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)

    dump = json.loads(args.dump.read_text())
    missions = load_lines(args.lines)
    pack = args.out / LOCALE_ROOT / "Data"

    translations = args.out / "translations.toml"
    translations.parent.mkdir(parents=True, exist_ok=True)
    write_translations(missions, translations)

    retail_text = resolve(args.retail / ENGLISH_LOCALE, "data", "Text")
    retail_level = retail_text / "Level.res"
    subprocess.run(
        [
            str(args.tool),
            "--input",
            str(retail_level),
            "--translations",
            str(translations),
            "--output",
            str(pack / "Text"),
        ],
        check=True,
    )
    print(f"{pack / 'Text' / 'Level.res'}: translated", file=sys.stderr)

    dialogue_dir = pack / "Text" / "Dialogues"
    dialogue_dir.mkdir(parents=True, exist_ok=True)
    copied = 0
    for clip in sorted((retail_text / "Dialogues").glob("*.wav")):
        shutil.copy2(clip, dialogue_dir / clip.name)
        copied += 1
    print(f"{dialogue_dir}: {copied} retail clips", file=sys.stderr)

    manifest = json.loads(args.manifest.read_text()) if args.manifest.exists() else {"lines": {}}
    dubbed = 0
    for mission, document in missions.items():
        level_code = document["meta"]["level_code"]
        red_dir = resolve(args.retail, "DATA", "Text")
        descriptor = descriptors.read(red_dir / f"RHLevel{level_code}.red")
        mapping = wave_paths(descriptor, dump)
        for line in document["line"]:
            key = f"{mission}-{line['id']}-{line['index']}"
            entry = manifest["lines"].get(key)
            if not entry:
                continue
            source = args.audio / entry["path"]
            if not source.exists():
                print(f"{key}: no audio at {source}, skipping", file=sys.stderr)
                continue
            if line["voice"] == "narrator":
                target = pack.parent / entry["path"]
            else:
                target = pack / "Text" / mapping[(int(line["id"]), int(line["index"]))]
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
            dubbed += 1
    print(f"{dubbed} Lithuanian clips placed", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
