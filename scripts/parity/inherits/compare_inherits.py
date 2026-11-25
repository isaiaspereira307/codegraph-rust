# ABOUTME: Compares Rust inheritors_of and file impact radius (depth 1, 2) with a crg_inherits.py dump.
# ABOUTME: Args: crg.json rust_root crg_root codegraph; prints per-language Jaccards and every divergence.
import json
import os
import sqlite3
import subprocess
import sys
from collections import defaultdict

crg_json, rust_dir, crg_dir, cg = sys.argv[1:5]
# Resolve before run() switches cwd to rust_dir: a relative cg would be looked up from there,
# and node paths in both DBs are absolute, so the roots must be too for strip().
rust_dir, crg_dir, cg = (os.path.abspath(p) for p in (rust_dir, crg_dir, cg))
crg = json.load(open(crg_json))


def run(tool, params):
    out = subprocess.run([cg, "tool", "run", tool, "--params", json.dumps(params)],
                         cwd=rust_dir, capture_output=True, text=True, check=True).stdout
    return json.loads(out)["result"]


db = sqlite3.connect(f"{rust_dir}/.code-review-graph/graph.db")
qn_by_loc = {}
for qn, fp, ls, kind in db.execute("SELECT qualified_name, file_path, line_start, kind FROM nodes"):
    qn_by_loc.setdefault((fp, ls, kind), qn)


def strip(s, root):
    return s.replace(root + "/", "")


def rqn(n):
    loc = n["location"]
    qn = qn_by_loc.get((loc["file_path"], loc["start_line"], n["kind"]))
    return strip(qn or f"?{loc['file_path']}:{loc['start_line']}:{n['name']}", rust_dir)


def lang(rel):
    return rel.split("/", 1)[0]


# Per language and set kind: [sum |A∩B|, sum |A∪B|, sets compared, identical sets].
totals = defaultdict(lambda: [0, 0, 0, 0])
diffs = []


def record(kind, key, a, b, note=""):
    a, b = set(a), set(b)
    t = totals[(lang(key), kind)]
    t[0] += len(a & b)
    t[1] += len(a | b)
    t[2] += 1
    t[3] += a == b
    if a != b:
        diffs.append((kind, key, sorted(a - b), sorted(b - a), note))


for qn, results in sorted(crg["inheritors"].items()):
    key = strip(qn, crg_dir)
    r = run("query_graph", {"pattern": "inheritors_of", "node_id": f"{rust_dir}/{key}",
                            "limit": 100})
    crg_set = [strip(x["qualified_name"], crg_dir) for x in results]
    tags = sorted({x["inferred_by"] for x in results if x["inferred_by"]})
    note = f"CRG inferred_by={','.join(tags)}; " if tags else ""
    record("inheritors_of", key, crg_set, [rqn(n) for n in r["results"]],
           f"{note}Rust unresolved={r['unresolved']}")

for k, impacted in sorted(crg["impact"].items()):
    depth, f = k.split(":", 1)
    key = strip(f, crg_dir)
    r = run("get_impact_radius", {"files": [key], "depth": int(depth), "max_nodes": 1000})
    record(f"impact d{depth}", key, [strip(q, crg_dir) for q in impacted],
           [rqn(n) for n in r["impacted"]])

print("| language | set | compared | identical | Σ∩ | Σ∪ | Jaccard |")
print("|---|---|---|---|---|---|---|")
for (lg, kind), (inter, union, n, same) in sorted(totals.items()):
    j = inter / union if union else 1.0
    print(f"| {lg} | {kind} | {n} | {same} | {inter} | {union} | {j:.3f} |")
print("\n### Divergences (CRG-only / Rust-only)\n")
for kind, key, only_crg, only_rust, note in diffs:
    print(f"- {kind} `{key}`: CRG-only {only_crg}, Rust-only {only_rust}"
          + (f" ({note})" if note else ""))
