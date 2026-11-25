# ABOUTME: Dumps code-review-graph inheritors_of per type node and file impact radius (depth 1, 2).
# ABOUTME: Run inside the CRG uv env; writes JSON for compare_inherits.py. Args: root out.json.
import json
import os
import sys

from code_review_graph.tools._common import _get_store
from code_review_graph.tools.query import query_graph

TYPE_KINDS = ("Class", "Interface", "Type", "Struct", "Enum", "Trait")

root, out = os.path.abspath(sys.argv[1]), sys.argv[2]
store, _ = _get_store(root)
try:
    rows = store._conn.execute(
        "SELECT qualified_name, kind FROM nodes ORDER BY qualified_name"
    ).fetchall()
    types = [qn for qn, kind in rows if kind in TYPE_KINDS]
    files = sorted({qn for qn, kind in rows if kind == "File"})
    inheritors = {}
    for qn in types:
        r = query_graph("inheritors_of", qn, repo_root=root, max_results=1000)
        inheritors[qn] = [
            {"qualified_name": n["qualified_name"], "inferred_by": n.get("inferred_by")}
            for n in r.get("results", [])
        ]
    impact = {}
    for depth in (1, 2):
        for f in files:
            imp = store.get_impact_radius([f], max_depth=depth, max_nodes=1000)
            impact[f"{depth}:{f}"] = sorted(n.qualified_name for n in imp["impacted_nodes"])
finally:
    store.close()
with open(out, "w") as fh:
    json.dump({"inheritors": inheritors, "impact": impact}, fh, indent=1, sort_keys=True)
