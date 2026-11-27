"""Share of robin_engine non-test code inside `impl EngineInner|Engine` blocks
versus everything else (free functions, other types' impls, type definitions)."""
import pathlib, re, sys
root = pathlib.Path(sys.argv[1]) / 'crates/robin_engine/src'
TEST = re.compile(r'(^|/)tests?(/|\.rs$)|_tests?\.rs$|test_helpers')
inside = total = 0
per = {}
for f in root.rglob('*.rs'):
    rel = str(f.relative_to(root))
    if TEST.search(rel): continue
    s = f.read_text(); total += len(s); n = 0
    for m in re.finditer(r'\bimpl(?:<[^>]*>)?\s+(EngineInner|Engine)\b[^{]*\{', s):
        i = s.index('{', m.start()); d = 0; j = i
        while j < len(s):
            d += (s[j] == '{') - (s[j] == '}')
            if d == 0: break
            j += 1
        n += j - i
    inside += n
    top = rel.split('/')[0].removesuffix('.rs')
    a = per.setdefault(top, [0, 0]); a[0] += n; a[1] += len(s)
print(f'non-test {total/1e6:.2f} MB; inside impl EngineInner/Engine {inside/1e6:.2f} MB ({100*inside/total:.0f}%)')
for k, (n, t) in sorted(per.items(), key=lambda kv: -kv[1][1])[:14]:
    print(f'  {k:22} {t/1e6:5.2f} MB, {100*n/t:3.0f}% EngineInner methods')
