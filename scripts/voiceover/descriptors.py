"""Reader for a mission's `.red` level descriptor.

The format is documented in `crates/robin_assets/src/res_descr.rs`: a flat
sequence of little-endian u32 words with no header.
"""

from __future__ import annotations

import struct
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class Descriptor:
    mission_text: int
    mission_picture: int
    dialogues: list[tuple[int, int, list[int]]] = field(default_factory=list)
    popup_text: int = 0
    popup_pictures: list[int] = field(default_factory=list)
    win_text: int = 0
    lose_text: int = 0
    briefing_text: int = 0

    def text_tables(self) -> list[int]:
        tables = [self.mission_text]
        tables.extend(text for text, _sound, _portraits in self.dialogues)
        tables.extend([self.popup_text, self.win_text, self.lose_text, self.briefing_text])
        return [table for table in tables if table]


def read(path: Path) -> Descriptor:
    raw = path.read_bytes()
    words = list(struct.unpack(f"<{len(raw) // 4}I", raw))
    cursor = 0

    def take(count: int = 1) -> list[int]:
        nonlocal cursor
        values = words[cursor : cursor + count]
        if len(values) != count:
            raise ValueError(f"{path}: truncated descriptor")
        cursor += count
        return values

    mission_text, mission_picture = take(2)
    descriptor = Descriptor(mission_text, mission_picture)

    (dialogue_count,) = take()
    for _ in range(dialogue_count):
        text_table, sound_table, portrait_count = take(3)
        descriptor.dialogues.append((text_table, sound_table, take(portrait_count)))

    picture_count, popup_text = take(2)
    descriptor.popup_text = popup_text
    descriptor.popup_pictures = take(picture_count)

    _win_count, win_text, _lose_count, lose_text = take(4)
    descriptor.win_text = win_text
    descriptor.lose_text = lose_text

    _briefing_count, briefing_text = take(2)
    descriptor.briefing_text = briefing_text
    return descriptor
