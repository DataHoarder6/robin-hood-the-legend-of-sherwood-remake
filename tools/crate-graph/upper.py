"""Largest set of top-level modules that nothing outside the set references.

usage: upper.py CRATE_DIR CORE_MODULE...
Modules in the result can move to a crate that depends on the rest.
"""
import pathlib, re, sys
from collections import defaultdict

root = pathlib.Path(sys.argv[1]) / 'src'
core = set(sys.argv[2:])
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

upper = set(mods) - core
changed = True
while changed:
    changed = False
    for m in sorted(upper):
        users = [o for o in mods if o not in upper and m in uses[o]]
        if users:
            upper.discard(m)
            changed = True

print(f'upper set ({sum(size[m] for m in upper)/1e6:.2f} MB):')
for m in sorted(upper, key=lambda m: -size[m]):
    print(f'  {m:28} {size[m]//1024:6} KB')
