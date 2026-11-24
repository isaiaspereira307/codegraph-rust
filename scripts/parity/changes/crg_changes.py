# ABOUTME: Runs code-review-graph detect_changes, get_review_context and raw impact radius on one repo.
# ABOUTME: Run inside the CRG uv env; writes JSON for compare_changes.py. Args: root base out.json [depth].
import json
import sys

from code_review_graph.tools._common import _get_store
from code_review_graph.tools.review import detect_changes_func, get_review_context

root, base, out = sys.argv[1], sys.argv[2], sys.argv[3]
dc = detect_changes_func(base=base, repo_root=root, max_results=100000, max_flows=100000)
rc = get_review_context(repo_root=root, base=base, include_source=False,
                        max_results=100000, max_files=100000)
files = [f if f.startswith("/") else f"{root}/{f}" for f in dc.get("changed_files", [])]
store, _ = _get_store(root)
try:
    imp = store.get_impact_radius(files, max_depth=int(sys.argv[4]) if len(sys.argv) > 4 else 2, max_nodes=1000)
    impact = {
        "changed_nodes": [n.qualified_name for n in imp["changed_nodes"]],
        "impacted_nodes": [n.qualified_name for n in imp["impacted_nodes"]],
        "impacted_files": sorted(imp["impacted_files"]),
        "truncated": imp.get("truncated"),
        "total_impacted": imp.get("total_impacted"),
    }
finally:
    store.close()
with open(out, "w") as f:
    json.dump({"detect_changes": dc, "review_context": rc, "impact": impact}, f,
              indent=1, default=str)
