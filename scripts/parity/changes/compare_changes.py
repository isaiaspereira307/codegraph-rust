# ABOUTME: Compares the Rust change analysis (detect_changes, impact, affected flows) with a CRG dump.
# ABOUTME: Args: crg.json rust_repo crg_repo base codegraph [depth]; prints a Markdown table of Jaccards.
import json
import os
import sqlite3
import subprocess
import sys

crg_json, rust_dir, crg_dir, base, cg = sys.argv[1:6]
# Resolve before run() switches cwd to rust_dir: a relative cg would be looked up from there,
# and node paths in both DBs are absolute, so the roots must be too for strip().
rust_dir, crg_dir, cg = (os.path.abspath(p) for p in (rust_dir, crg_dir, cg))
DEPTH = int(sys.argv[6]) if len(sys.argv) > 6 else 2
crg = json.load(open(crg_json))


def run(tool, params):
    out = subprocess.run([cg, "tool", "run", tool, "--params", json.dumps(params)],
                         cwd=rust_dir, capture_output=True, text=True, check=True).stdout
    return json.loads(out)["result"]


db = sqlite3.connect(f"{rust_dir}/.code-review-graph/graph.db")
qn_by_loc = {}
for qn, fp, ls, kind in db.execute("SELECT qualified_name, file_path, line_start, kind FROM nodes"):
    qn_by_loc.setdefault((fp, ls, kind), qn)


def rqn(n):
    loc = n["location"]
    qn = qn_by_loc.get((loc["file_path"], loc["start_line"], n["kind"]))
    return strip(qn or f"?{loc['file_path']}:{loc['start_line']}:{n['name']}", rust_dir)


def strip(s, root):
    return s.replace(root + "/", "")


def jacc(a, b):
    a, b = set(a), set(b)
    u = a | b
    return (len(a & b) / len(u) if u else 1.0), len(a), len(b), len(a & b), sorted(a - b), sorted(b - a)


def show(label, a, b, extra=""):
    j, na, nb, both, only_a, only_b = jacc(a, b)
    print(f"| {label} | {na} | {nb} | {both} | {j:.3f} |{extra}")
    return label, only_a, only_b


rows = []
print(f"\n### base {base}\n")
print("| set | CRG | Rust | both | Jaccard |")
print("|---|---|---|---|---|")
dc = crg["detect_changes"]
rdc = run("detect_changes", {"base_ref": base, "include_churn": True})  # CRG MCP always adds churn
crg_files = [f for f in dc["changed_files"]]
rust_files = [f["path"] for f in rdc["changed_files"]]
rows.append(show("changed files", crg_files, rust_files))
crg_fn = [strip(f["qualified_name"], crg_dir) for f in dc["changed_functions"]]
rust_fn = [rqn(n) for n in rdc["changed_nodes"] if n["kind"] != "File"]
rows.append(show("changed functions (non-File)", crg_fn, rust_fn))

imp = crg["impact"]
code_files = sorted({n.split("::")[0] for n in imp["changed_nodes"]})
rel_code_files = [strip(f, crg_dir) for f in code_files]
rimp = run("get_impact_radius", {"files": rel_code_files, "depth": DEPTH, "max_nodes": 1000})
crg_imp = [strip(q, crg_dir) for q in imp["impacted_nodes"]]
# Rust `impacted` is the CRG-equivalent set; its separate `tests` list adds one TESTED_BY hop.
rust_imp = [rqn(n) for n in rimp["impacted"]]
rust_imp_tests = rust_imp + [rqn(n) for n in rimp["tests"]]
trunc = f" CRG truncated={imp['truncated']} total={imp['total_impacted']}; Rust truncated={rimp['truncated']}"
rows.append(show(f"impacted nodes (depth {DEPTH}, ≤1000)", crg_imp, rust_imp, trunc))
rows.append(show("impacted nodes + Rust tests list", crg_imp, rust_imp_tests))
crg_if = [strip(f, crg_dir) for f in imp["impacted_files"]]
rust_if = [strip(f, rust_dir) for f in rimp["affected_files"]]
changed = set(rel_code_files)


def outside(qns):
    return [q for q in qns if q.split("::")[0] not in changed]


# Rust `affected_files` also lists the changed files themselves; CRG does not.
rows.append(show("impacted files outside changed files", outside(crg_if), outside(rust_if)))

rfl = run("get_affected_flows", {"base_ref": base, "detail_level": "minimal", "max_flows": 500})
crg_flows = [f["name"] for f in dc["affected_flows"]]
rust_flows = [f["name"] for f in rfl["affected_flows"]]
rows.append(show("affected flows (entry name)", crg_flows, rust_flows))

crg_gaps = [strip(g.get("qualified_name", g.get("name", "")), crg_dir) for g in dc["test_gaps"]]
print(f"\nCRG test_gaps={dc['test_gaps_total']}  CRG risk={dc['risk_score']}  Rust risk={rdc['risk_score']}")
for label, only_crg, only_rust in rows:
    if only_crg or only_rust:
        print(f"\n**{label}** only CRG ({len(only_crg)}): {only_crg[:12]}")
        print(f"only Rust ({len(only_rust)}): {only_rust[:12]}")
