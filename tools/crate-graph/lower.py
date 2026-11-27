"""Modules whose transitive in-crate dependencies avoid the given hub modules.

usage: lower.py CRATE_DIR HUB...
Such a module, with its closure, can move into a crate below the rest.
"""
import pathlib, re, sys

root = pathlib.Path(sys.argv[1]) / 'src'
hubs = set(sys.argv[2:])
lib = (root / 'lib.rs').read_text()
mods = re.findall(r'^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([a-z_0-9]+)\s*;', lib, re.M)

def files_of(m):
    out = []
    f = root / f'{m}.rs'
    if f.exists():
        out.append(f)
    d = root / m
    if d.is_dir():
        out += sorted(d.rglob('*.rs'))
    return out

text = {m: '\n'.join(f.read_text() for f in files_of(m)) for m in mods}
size = {m: sum(f.stat().st_size for f in files_of(m)) for m in mods}

def mentions(src, other):
    if re.search(r'\bcrate::' + other + r'\b', src):
        return True
    for g in re.finditer(r'use\s+crate::\{([^;]*)\};', src, re.S):
        if re.search(r'(^|[\s,{])' + other + r'\b', g.group(1)):
            return True
    return False

uses = {m: {o for o in mods if o != m and mentions(text[m], o)} for m in mods}

def closure(m):
    seen = set(); stack = [m]
    while stack:
        x = stack.pop()
        for y in uses[x]:
            if y not in seen:
                seen.add(y); stack.append(y)
    return seen

lower = [m for m in mods if m not in hubs and not (closure(m) | {m}) & hubs]
total = sum(size[m] for m in lower)
print(f'lower set: {len(lower)} modules, {total/1e6:.2f} MB')
for m in sorted(lower, key=lambda m: -size[m]):
    deps = sorted(uses[m])
    print(f'  {m:28} {size[m]//1024:6} KB  uses {",".join(deps)[:80]}')
