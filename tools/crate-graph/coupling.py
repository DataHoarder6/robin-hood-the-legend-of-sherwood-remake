"""Rank a crate's top-level modules by size and coupling to the rest of the crate."""
import pathlib, re, sys
from collections import defaultdict

root = pathlib.Path(sys.argv[1]) / 'src'
lib = (root / 'lib.rs').read_text()
mods = re.findall(r'^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([a-z_0-9]+)\s*;', lib, re.M)

def files_of(m):
    out = []
    f = root / f'{m}.rs'
    if f.exists():
        out.append(f)
    d = root / m
    if d.is_dir():
        out += list(d.rglob('*.rs'))
    return out

size = {}
text = {}
for m in mods:
    fs = files_of(m)
    text[m] = '\n'.join(f.read_text() for f in fs)
    size[m] = sum(f.stat().st_size for f in fs)

uses = defaultdict(set)   # m -> other top-level modules it references
for m in mods:
    for other in mods:
        if other == m:
            continue
        if re.search(r'\bcrate::' + other + r'\b', text[m]) or re.search(r'\bsuper::' + other + r'\b', text[m]):
            uses[m].add(other)
lib_uses = {o for o in mods if re.search(r'\bcrate::' + o + r'\b', lib)}
used_by = defaultdict(set)
for m, os_ in uses.items():
    for o in os_:
        used_by[o].add(m)

rows = sorted(mods, key=lambda m: -size[m])
print(f'{"module":28} {"KB":>6} {"uses":>4} {"used_by":>7}  uses-list')
for m in rows:
    print(f'{m:28} {size[m]//1024:6} {len(uses[m]):4} {len(used_by[m]):7}  {",".join(sorted(uses[m]))[:90]}')
