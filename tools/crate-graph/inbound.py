"""List the items of the given modules that the rest of the crate references.

usage: inbound.py CRATE_DIR mod1 [mod2 ...]
"""
import pathlib, re, sys
from collections import defaultdict

root = pathlib.Path(sys.argv[1]) / 'src'
targets = set(sys.argv[2:])
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

hits = defaultdict(lambda: defaultdict(set))
for m in mods:
    if m in targets:
        continue
    for f in files_of(m):
        src = f.read_text()
        rel = str(f.relative_to(root))
        is_test = bool(re.search(r'(^|/)tests?(/|\.rs$)|_tests?\.rs$', rel))
        for t in targets:
            for mm in re.finditer(r'\bcrate::' + t + r'((?:::[A-Za-z_0-9]+)*)', src):
                hits[t][(mm.group(1) or '::*', is_test)].add(rel)
            for g in re.finditer(r'use\s+crate::\{([^;]*)\};', src, re.S):
                for e in re.finditer(r'(?:^|[\s,{])' + t + r'((?:::[A-Za-z_0-9{}, ]+)?)', g.group(1)):
                    hits[t][((e.group(1) or '::*').strip(), is_test)].add(rel)
for t in sorted(hits):
    print(f'== {t}')
    for (item, is_test), files in sorted(hits[t].items()):
        tag = 'TEST' if is_test else '    '
        print(f'  {tag} {item[:60]:60} {len(files):3} files  {sorted(files)[0]}')
