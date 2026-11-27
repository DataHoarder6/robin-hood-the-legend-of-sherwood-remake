"""For a set of top-level modules, list every crate item they reference outside the set.

usage: cluster.py CRATE_DIR mod1 mod2 ...
Prints module -> referenced items outside the cluster, and who outside uses the cluster.
"""
import pathlib, re, sys
from collections import defaultdict

root = pathlib.Path(sys.argv[1]) / 'src'
cluster = set(sys.argv[2:])
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

def strip_tests(text):
    # drop `#[cfg(test)] mod x { ... }` blocks and test files roughly: keep simple,
    # count test files separately by name
    return text

def refs(text):
    found = set()
    for m in re.finditer(r'\bcrate::([a-z_0-9]+)(?:::([A-Za-z_0-9]+))?', text):
        found.add((m.group(1), m.group(2) or '*'))
    # grouped: use crate::{a::B, c::{D, E}, f}
    for m in re.finditer(r'use\s+crate::\{', text):
        i = m.end(); depth = 1; j = i
        while depth and j < len(text):
            if text[j] == '{': depth += 1
            elif text[j] == '}': depth -= 1
            j += 1
        body = text[i:j-1]
        # top-level entries
        depth = 0; cur = ''; entries = []
        for ch in body:
            if ch == '{': depth += 1
            if ch == '}': depth -= 1
            if ch == ',' and depth == 0:
                entries.append(cur); cur = ''
            else:
                cur += ch
        entries.append(cur)
        for e in entries:
            e = ' '.join(e.split())
            if not e: continue
            mm = re.match(r'([a-z_0-9]+)(?:::(\{?)([A-Za-z_0-9]+)?)?', e)
            if not mm: continue
            mod = mm.group(1)
            if mm.group(2) == '{':
                for item in re.findall(r'[A-Za-z_][A-Za-z_0-9]*', e[len(mod)+2:]):
                    if item not in ('self', 'as'):
                        found.add((mod, item))
            else:
                found.add((mod, mm.group(3) or '*'))
    return found

out_refs = defaultdict(lambda: defaultdict(set))
test_only = defaultdict(lambda: defaultdict(set))
for m in sorted(cluster):
    for f in files_of(m):
        rel = f.relative_to(root)
        is_test = 'test' in f.name or '/tests/' in str(rel)
        for mod, item in refs(f.read_text()):
            if mod in cluster or mod not in mods:
                continue
            (test_only if is_test else out_refs)[mod][item].add(str(rel))

print('== non-test references from the cluster to the rest of the crate')
for mod in sorted(out_refs):
    items = out_refs[mod]
    print(f'{mod}:')
    for item in sorted(items):
        print(f'    {item:32} {len(items[item])} files  e.g. {sorted(items[item])[0]}')
print('== test-only references')
for mod in sorted(test_only):
    print(f'{mod}: {", ".join(sorted(test_only[mod]))[:200]}')

size = sum(f.stat().st_size for m in cluster for f in files_of(m))
print(f'== cluster size {size/1e6:.2f} MB')
