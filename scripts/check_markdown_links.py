#!/usr/bin/env python3
"""Validate relative links in tracked Markdown files (stdlib only).

Run from anywhere; files come from `git ls-files`, so only tracked Markdown is
checked. A link is local when it has no URI scheme and no query string; its
path (with the fragment removed) must exist relative to the linking file, or
at the repository root when it starts with `/`. External links, mail links,
same-file anchors and query endpoints are not this checker's business.
"""
from __future__ import annotations

import posixpath
import re
import subprocess
import sys
import urllib.parse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Inline links/images: [text](target "title") and ![alt](<target with spaces>).
INLINE = re.compile(r"!?\[[^\]]*\]\(\s*(<[^>]*>|[^)\s]+)(?:\s+[\"'(][^)]*)?\)")
# `[^1]:` is a footnote, not a reference link definition.
REFERENCE = re.compile(r"^ {0,3}\[([^\^\]]+)\]:\s*(<[^>]*>|\S+)(?:\s+[\"'(].*)?$")
FENCE = re.compile(r"^\s*(`{3,}|~{3,})")
SCHEME = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")
INLINE_CODE = re.compile(r"`[^`]*`")


def tracked_markdown() -> list[str]:
    output = subprocess.check_output(
        ["git", "ls-files", "--", "*.md"], cwd=ROOT, text=True
    )
    return sorted(line for line in output.splitlines() if line)


def local_target(raw: str) -> str | None:
    target = raw.strip()
    if target.startswith("<") and target.endswith(">"):
        target = target[1:-1]
    target = urllib.parse.unquote(target)
    if not target or SCHEME.match(target) or target.startswith("//"):
        return None  # external, mail, or protocol-relative
    if "?" in target:
        return None  # a query endpoint, not a repository file
    path = target.split("#", 1)[0]
    return path or None  # a same-file anchor needs no file check


def check_file(relative: str) -> list[str]:
    text = (ROOT / relative).read_text(encoding="utf-8", errors="replace")
    errors: list[str] = []
    references: list[tuple[int, str]] = []
    fence: str | None = None

    for lineno, line in enumerate(text.splitlines(), 1):
        marker = FENCE.match(line)
        if marker:
            char = marker.group(1)[0]
            if fence is None:
                fence = char
            elif fence == char:
                fence = None
            continue
        if fence is not None:
            continue
        code_free = INLINE_CODE.sub("", line)

        definition = REFERENCE.match(code_free)
        if definition:
            references.append((lineno, definition.group(2)))
        for match in INLINE.finditer(code_free):
            errors.extend(check_target(relative, lineno, match.group(1)))

    for lineno, target in references:
        errors.extend(check_target(relative, lineno, target))
    return errors


def check_target(relative: str, lineno: int, raw: str) -> list[str]:
    path = local_target(raw)
    if path is None:
        return []
    if path.startswith("/"):
        resolved = posixpath.normpath(path.lstrip("/"))
    else:
        resolved = posixpath.normpath(
            posixpath.join(posixpath.dirname(relative), path)
        )
    if resolved == ".." or resolved.startswith("../"):
        return []  # points outside the checkout; not a link to a tracked file
    if not (ROOT / resolved).exists():
        return [f"{relative}:{lineno}: missing link target {raw.strip()!r}"]
    return []


def main(argv: list[str] | None = None) -> int:
    # Paths are supplied by the landing gate: it checks the Markdown files a
    # change touches, not the retained historical/vendor docs that already
    # carry known rot. With no arguments this audits the whole tree.
    files = list(argv) if argv else tracked_markdown()
    errors: list[str] = []
    for relative in files:
        if not (ROOT / relative).is_file():
            errors.append(f"{relative}: not a file in this checkout")
            continue
        errors.extend(check_file(relative))
    if errors:
        for error in errors:
            print(error)
        print(f"markdown links: {len(errors)} broken relative target(s) in {len(files)} file(s)")
        return 1
    print(f"markdown links: {len(files)} file(s), all relative targets exist")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
