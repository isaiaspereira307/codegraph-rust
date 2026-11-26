# ABOUTME: Counts File->File IMPORTS_FROM pairs and the pairs inside import cycles (CRG find_import_cycles).
# ABOUTME: Usage: python3 scripts/parity/cycle_pairs.py <label> <graph.db> [<label> <graph.db> ...]
import json, sqlite3, sys
from collections import defaultdict

def load(db):
    c = sqlite3.connect(db)
    files = {r[0] for r in c.execute(
        "SELECT file_path FROM nodes WHERE extra IS NULL OR json_valid(extra)=0 "
        "OR COALESCE(json_extract(extra,'$.virtual'),0) != 1 UNION SELECT file_path FROM edges")}
    raw, kept, ctx = set(), set(), defaultdict(int)
    for s, t, extra in c.execute(
            "SELECT source_qualified, target_qualified, extra FROM edges WHERE kind='IMPORTS_FROM'"):
        if s == t or s not in files or t not in files:
            continue
        raw.add((s, t))
        try:
            context = (json.loads(extra or "{}") or {}).get("import_context")
        except Exception:
            context = None
        ctx[context] += 1
        if context in ("type_only", "parent_package"):
            continue
        kept.add((s, t))
    return raw, kept, dict(ctx)

def scc(pairs):
    adj = defaultdict(list)
    for s, t in pairs:
        adj[s].append(t)
    index, low, stack, on, comp, n = {}, {}, [], set(), {}, [0]
    sys.setrecursionlimit(100000)
    def visit(v):
        index[v] = low[v] = n[0]; n[0] += 1; stack.append(v); on.add(v)
        for w in adj[v]:
            if w not in index:
                visit(w); low[v] = min(low[v], low[w])
            elif w in on:
                low[v] = min(low[v], index[w])
        if low[v] == index[v]:
            members = []
            while True:
                w = stack.pop(); on.discard(w); members.append(w)
                if w == v: break
            for m in members: comp[m] = (v, len(members))
    for v in list(adj):
        if v not in index: visit(v)
    return comp

for label, db in zip(sys.argv[1::2], sys.argv[2::2]):
    raw, kept, ctx = load(db)
    comp = scc(kept)
    in_cycle = [p for p in kept if p[0] in comp and p[1] in comp and comp[p[0]] == comp[p[1]] and comp[p[0]][1] > 1]
    groups = {comp[p[0]][0] for p in in_cycle}
    print(f"{label}: file->file pairs={len(raw)} after context filter={len(kept)} "
          f"pairs in cycles={len(in_cycle)} cycle groups={len(groups)} contexts={ctx}")
