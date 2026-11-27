"""Method-call graph between robin_engine's engine/* submodules (non-test code).

A unit is a top-level file or directory under crates/robin_engine/src/engine/.
An edge U -> V means U's non-test code calls a method that only V defines in an
`impl EngineInner|Engine|...` block. Prints unit sizes, SCCs and the layering.
usage: engine_graph.py REPO
"""
import pathlib, re, sys, collections
root = pathlib.Path(sys.argv[1]) / 'crates/robin_engine/src/engine'
TEST = re.compile(r'(^|/)tests?(/|\.rs$)|_tests?\.rs$|test_helpers')
def unit_of(f):
    rel = f.relative_to(root)
    return rel.parts[0].removesuffix('.rs')
def strip_test_mods(src):
    out=[]; skip=0; depth=0; pending=False
    for line in src.split('\n'):
        if skip:
            depth += line.count('{') - line.count('}')
            if depth <= 0: skip=0
            continue
        if pending and re.match(r'\s*(pub\s+)?mod\s+\w+\s*\{', line):
            skip=1; depth=line.count('{')-line.count('}'); pending=False
            if depth<=0: skip=0
            continue
        pending = line.strip()=='#[cfg(test)]'
        out.append(line)
    return '\n'.join(out)
files=[f for f in root.rglob('*.rs') if not TEST.search(str(f.relative_to(root)))]
text=collections.defaultdict(str); size=collections.Counter()
for f in files:
    s=strip_test_mods(f.read_text()); u=unit_of(f); text[u]+=s+'\n'; size[u]+=len(s)
defs=collections.defaultdict(set)
for u,s in text.items():
    for m in re.finditer(r'\bimpl(?:<[^>]*>)?\s+(\w+)(?:<[^>]*>)?\s*\{', s):
        if m.group(1) not in ('EngineInner','Engine','SimulationHost','WorldState','EngineControl','MissionDomain'): continue
        # collect fn names until the matching close brace
        i=s.index('{',m.start()); d=0; j=i
        while j < len(s):
            if s[j]=='{': d+=1
            elif s[j]=='}':
                d-=1
                if d==0: break
            j+=1
        for fm in re.finditer(r'\bfn\s+(\w+)', s[i:j]): defs[u].add(fm.group(1))
owner=collections.defaultdict(set)
for u,ns in defs.items():
    for n in ns: owner[n].add(u)
unique={n:next(iter(us)) for n,us in owner.items() if len(us)==1}
edges=collections.defaultdict(set)
for u,s in text.items():
    for m in re.finditer(r'\.(\w+)\s*(?:::<[^>]*>)?\(', s):
        v=unique.get(m.group(1))
        if v and v!=u: edges[u].add(v)
# Tarjan SCC
idx={}; low={}; st=[]; on=set(); sccs=[]; c=[0]
sys.setrecursionlimit(10000)
def sc(v):
    idx[v]=low[v]=c[0]; c[0]+=1; st.append(v); on.add(v)
    for w in edges[v]:
        if w not in idx: sc(w); low[v]=min(low[v],low[w])
        elif w in on: low[v]=min(low[v],idx[w])
    if low[v]==idx[v]:
        comp=[]
        while True:
            w=st.pop(); on.discard(w); comp.append(w)
            if w==v: break
        sccs.append(comp)
for u in list(text): 
    if u not in idx: sc(u)
total=sum(size.values())
print(f'engine/* non-test: {total/1e6:.2f} MB in {len(text)} units, {sum(len(v) for v in defs.values())} methods')
big=sorted(sccs,key=lambda c:-sum(size[u] for u in c))
for comp in big[:6]:
    print(f'SCC {sum(size[u] for u in comp)/1e6:.2f} MB, {len(comp)} units: {", ".join(sorted(comp, key=lambda u:-size[u])[:25])}')
singles=[c[0] for c in sccs if len(c)==1]
print(f'acyclic units: {len(singles)}, {sum(size[u] for u in singles)/1e6:.2f} MB')
for u in sorted(singles,key=lambda u:-size[u])[:25]:
    print(f'   {u:30} {size[u]//1024:5} KB  out:{len(edges[u]):3} in:{sum(1 for x in edges if u in edges[x]):3}')
