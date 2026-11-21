#!/usr/bin/env python3
"""Synthesise the Lithuanian line tables with Google Chirp 3 HD.

    python3 scripts/voiceover/synth.py --dry-run          # counts only, no network
    GOOGLE_TTS_API_KEY=... python3 scripts/voiceover/synth.py

`--dry-run` writes the manifest and stops before the first request: it is what
the tests exercise and what turns "how much would this cost" into a number.
The real run resumes from the manifest, so an interrupted batch does not pay
for its finished lines twice.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import subprocess
import sys
import time
import tomllib
import urllib.request
from datetime import date
from pathlib import Path

ENDPOINT = "https://texttospeech.googleapis.com/v1beta1/text:synthesize"


def load_lines(directory: Path) -> list[dict]:
    entries: list[dict] = []
    for path in sorted(directory.glob("*.toml")):
        mission = path.stem.upper()
        document = tomllib.loads(path.read_text())
        if "line" not in document:
            raise SystemExit(f"{path}: no [[line]] entries")
        for index, line in enumerate(document["line"]):
            for key in ("id", "index", "display", "voice"):
                if key not in line:
                    raise SystemExit(f"{path}: line {index} has no {key}")
            if not line["display"].strip():
                raise SystemExit(f"{path}: line {index} has an empty display text")
            if line["voice"] == "none":
                # No playback lane exists for this text (mission description,
                # short briefings, debrief): do not synthesise an unplayable clip.
                continue
            entries.append(
                {
                    "mission": mission,
                    "table": int(line["id"]),
                    "index": int(line["index"]),
                    "display": line["display"],
                    "spoken": line.get("spoken") or line["display"],
                    "voice": line["voice"],
                    "provenance": line.get("provenance", "machine-draft"),
                }
            )
    return entries


def key_of(entry: dict) -> str:
    return f"{entry['mission']}-{entry['table']}-{entry['index']}"


def output_path(entry: dict) -> str:
    if entry["voice"] == "narrator":
        return f"Data/Text/Narration/{entry['table']}_{entry['index']}.ogg"
    return f"Data/Text/Dialogues/{entry['mission']}_{entry['index']}.ogg"


def load_spent(path: Path) -> dict:
    if not path.exists():
        return {"month": date.today().strftime("%Y-%m"), "characters": 0}
    return json.loads(path.read_text())


def load_manifest(path: Path) -> dict:
    if not path.exists():
        return {"characters": 0, "lines": {}}
    return json.loads(path.read_text())


def resolve_voice(entry: dict, cast: dict) -> str:
    character = entry["voice"]
    if character not in cast:
        raise SystemExit(f"cast.toml has no voice for '{character}'")
    return cast[character]["voice"]


def synthesise(text: str, voice: str, audio_encoding: str, api_key: str) -> bytes:
    body = json.dumps(
        {
            "input": {"text": text},
            "voice": {"languageCode": "lt-LT", "name": voice},
            "audioConfig": {"audioEncoding": audio_encoding, "sampleRateHertz": 48000},
        }
    ).encode()
    request = urllib.request.Request(
        f"{ENDPOINT}?key={api_key}",
        data=body,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=120) as response:
        payload = json.load(response)
    return base64.b64decode(payload["audioContent"])


def write_ogg(raw: bytes, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    wav = path.with_suffix(".tmp.wav")
    wav.write_bytes(raw)
    subprocess.run(
        [
            "ffmpeg",
            "-y",
            "-loglevel",
            "error",
            "-i",
            str(wav),
            "-ac",
            "1",
            "-ar",
            "44100",
            "-c:a",
            "libvorbis",
            "-q:a",
            "5",
            str(path),
        ],
        check=True,
    )
    wav.unlink()


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lines", type=Path, default=Path("scripts/voiceover/lines"))
    parser.add_argument("--cast", type=Path, default=Path("scripts/voiceover/cast.toml"))
    parser.add_argument("--manifest", type=Path, default=Path("scripts/voiceover/manifest.json"))
    parser.add_argument("--spent", type=Path, default=Path("scripts/voiceover/spent.json"))
    parser.add_argument("--out", type=Path, default=Path("build/voiceover"))
    parser.add_argument("--budget", type=int, default=900_000)
    parser.add_argument(
        "--min-interval",
        type=float,
        default=12.0,
        help="seconds between requests; the project's Chirp 3 HD quota is 5/min",
    )
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)

    entries = load_lines(args.lines)
    cast = tomllib.loads(args.cast.read_text())
    manifest = load_manifest(args.manifest)
    spent = load_spent(args.spent)
    month = date.today().strftime("%Y-%m")
    if spent.get("month") != month:
        spent = {"month": month, "characters": 0}

    # A manifest entry alone does not mean the line was synthesised: a dry run
    # records entries without writing audio. The file on disk is the proof.
    pending = [
        entry
        for entry in entries
        if key_of(entry) not in manifest["lines"]
        or not (args.out / output_path(entry)).is_file()
    ]
    characters = sum(len(entry["spoken"]) for entry in pending)
    if spent["characters"] + characters > args.budget:
        raise SystemExit(
            f"refusing: {characters} characters would pass the {args.budget} budget "
            f"({spent['characters']} already spent this month)"
        )

    print(
        f"{len(entries)} lines, {len(pending)} pending, {characters} characters, "
        f"{args.min_interval:.1f}s interval",
        file=sys.stderr,
    )
    if args.dry_run:
        for entry in pending:
            manifest["lines"][key_of(entry)] = {
                "path": output_path(entry),
                "characters": len(entry["spoken"]),
                "voice": resolve_voice(entry, cast),
                "provenance": entry["provenance"],
            }
        manifest["characters"] = sum(
            line["characters"] for line in manifest["lines"].values()
        )
        args.manifest.parent.mkdir(parents=True, exist_ok=True)
        args.manifest.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
        print(f"dry run: wrote {args.manifest}", file=sys.stderr)
        return 0

    api_key = os.environ.get("GOOGLE_TTS_API_KEY")
    if not api_key:
        raise SystemExit("GOOGLE_TTS_API_KEY is not set")

    for index, entry in enumerate(pending):
        if index:
            time.sleep(args.min_interval)
        key = key_of(entry)
        voice = resolve_voice(entry, cast)
        raw = synthesise(entry["spoken"], voice, cast[entry["voice"]].get("encoding", "OGG_OPUS"), api_key)
        path = args.out / output_path(entry)
        write_ogg(raw, path)
        manifest["lines"][key] = {
            "path": output_path(entry),
            "characters": len(entry["spoken"]),
            "voice": voice,
            "provenance": entry["provenance"],
        }
        manifest["characters"] = sum(
            line["characters"] for line in manifest["lines"].values()
        )
        spent["characters"] += len(entry["spoken"])
        args.manifest.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
        args.spent.write_text(json.dumps(spent, indent=2) + "\n")
        print(f"{key}: {len(entry['spoken'])} chars -> {path}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
