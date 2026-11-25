# ABOUTME: Unit tests for the parity graph.db comparator, using synthetic SQLite databases.
# ABOUTME: Run with: python3 -m unittest discover -s scripts/parity -v
import json
import shutil
import sqlite3
import tempfile
import unittest
from dataclasses import asdict
from pathlib import Path

import subprocess

from changes.risk import equal_risk_count
from check_p1 import collect_metrics, evaluate, parse_report
from parity import (
    GIT, Graph, adjusted_rand_index, check_crg_commit, compare, load_graph, main, prepare_copies,
    render_markdown, run_corpus,
)


def make_db(path, root, nodes, edges=()):
    """nodes: (kind, qn, language, line_start, line_end, is_test); edges: (kind, src, tgt)."""
    con = sqlite3.connect(path)
    con.execute(
        "CREATE TABLE nodes(kind, name, qualified_name, file_path, line_start, line_end, "
        "language, is_test)"
    )
    con.execute(
        "CREATE TABLE edges(kind, source_qualified, target_qualified, file_path, line)"
    )
    for kind, qn, lang, ls, le, t in nodes:
        fp = qn.split("::")[0]
        con.execute(
            "INSERT INTO nodes VALUES (?,?,?,?,?,?,?,?)",
            (kind, qn.split("::")[-1], qn, fp, ls, le, lang, t),
        )
    for kind, s, t in edges:
        con.execute("INSERT INTO edges VALUES (?,?,?,?,?)", (kind, s, t, s.split("::")[0], 1))
    con.commit()
    con.close()
    return Path(path)


def add_analysis(path, flows=(), communities=None, detector=None, community_rows=0):
    """flows: (entry_qn, [member_qn], criticality); communities: {qn: community_id}."""
    con = sqlite3.connect(path)
    con.execute("ALTER TABLE nodes ADD COLUMN id INTEGER")
    con.execute("ALTER TABLE nodes ADD COLUMN community_id INTEGER")
    con.execute("UPDATE nodes SET id = rowid")
    ids = dict(con.execute("SELECT qualified_name, id FROM nodes"))
    con.execute("CREATE TABLE flows(id INTEGER PRIMARY KEY, entry_point_id, criticality)")
    con.execute("CREATE TABLE flow_memberships(flow_id, node_id, position)")
    con.execute("CREATE TABLE communities(id INTEGER PRIMARY KEY)")
    con.execute("CREATE TABLE metadata(key, value)")
    for i, (entry, members, crit) in enumerate(flows, start=1):
        con.execute("INSERT INTO flows VALUES (?,?,?)", (i, ids[entry], crit))
        for pos, m in enumerate(members):
            con.execute("INSERT INTO flow_memberships VALUES (?,?,?)", (i, ids[m], pos))
    for qn, cid in (communities or {}).items():
        con.execute("UPDATE nodes SET community_id = ? WHERE qualified_name = ?", (cid, qn))
    for _ in range(community_rows):
        con.execute("INSERT INTO communities DEFAULT VALUES")
    if detector:
        con.execute("INSERT INTO metadata VALUES ('community_detector', ?)", (detector,))
    con.commit()
    con.close()


class AriTest(unittest.TestCase):
    def test_identical_partitions_up_to_relabeling(self):
        self.assertEqual(adjusted_rand_index([1, 1, 2, 2, 3], [7, 7, 5, 5, 9]), 1.0)

    def test_known_value(self):
        # Classic example (sklearn docs): ARI([0,0,1,1], [0,0,1,2]) = 0.571428...
        self.assertAlmostEqual(adjusted_rand_index([0, 0, 1, 1], [0, 0, 1, 2]), 4 / 7)

    def test_independent_split_is_negative_or_zero(self):
        self.assertAlmostEqual(adjusted_rand_index([0, 0, 1, 1], [0, 1, 0, 1]), -0.5)

    def test_degenerate_cases(self):
        self.assertIsNone(adjusted_rand_index([1], [2]))
        self.assertEqual(adjusted_rand_index([1, 1, 1], [4, 4, 4]), 1.0)
        self.assertEqual(adjusted_rand_index([1, 2, 3], [4, 5, 6]), 1.0)
        self.assertEqual(adjusted_rand_index([1, 1, 1], [1, 2, 3]), 0.0)


class ParityTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.d = Path(self.tmp.name)

    def pair(self, crg_nodes, rust_nodes, crg_edges=(), rust_edges=()):
        crg = make_db(self.d / "crg.db", "/a/r1", crg_nodes, crg_edges)
        rust = make_db(self.d / "rust.db", "/b/r2", rust_nodes, rust_edges)
        return compare(load_graph(crg, "/a/r1"), load_graph(rust, "/b/r2"))

    def test_relative_keys_match_across_roots(self):
        rep = self.pair(
            [("Function", "/a/r1/m.py::f", "python", 1, 2, 0)],
            [("Function", "/b/r2/m.py::f", "python", 1, 2, 0)],
        )
        self.assertEqual(rep.nodes_by_language["python"]["jaccard"], 1.0)

    def test_line_mismatch_is_divergent_not_missing(self):
        rep = self.pair(
            [("Function", "/a/r1/m.py::f", "python", 1, 2, 0)],
            [("Function", "/b/r2/m.py::f", "python", 1, 3, 0)],
        )
        py = rep.nodes_by_language["python"]
        self.assertEqual(py["both"], 1)
        self.assertEqual(len(py["divergent"]), 1)
        self.assertEqual(py["divergent"][0]["field"], "line_end")
        self.assertEqual((py["divergent"][0]["crg"], py["divergent"][0]["rust"]), (2, 3))

    def test_unresolved_targets_counted_separately(self):
        node = ("Function", "/a/r1/m.py::f", "python", 1, 2, 0)
        rnode = ("Function", "/b/r2/m.py::f", "python", 1, 2, 0)
        rep = self.pair(
            [node], [rnode], [("CALLS", "/a/r1/m.py::f", "print")], []
        )
        e = rep.edges_by_kind["CALLS"]
        self.assertEqual((e["crg"], e["rust"], e["jaccard"]), (1, 0, 0.0))
        self.assertEqual(e["resolved_jaccard"], 1.0)

    def test_resolved_edges_compared_and_prefix_stripped(self):
        nodes = lambda r: [
            ("Function", f"{r}/m.py::f", "python", 1, 2, 0),
            ("Function", f"{r}/m.py::g", "python", 3, 4, 0),
        ]
        rep = self.pair(
            nodes("/a/r1"),
            nodes("/b/r2"),
            [("CALLS", "/a/r1/m.py::f", "/a/r1/m.py::g")],
            [("CALLS", "/b/r2/m.py::f", "/b/r2/m.py::g"), ("CALLS", "/b/r2/m.py::g", "/b/r2/m.py::f")],
        )
        e = rep.edges_by_kind["CALLS"]
        self.assertEqual((e["both"], e["jaccard"], e["resolved_jaccard"]), (1, 0.5, 0.5))
        self.assertEqual(e["only_rust"], [["CALLS", "m.py::g", "m.py::f"]])

    def test_only_sides_and_language_split(self):
        rep = self.pair(
            [("Function", "/a/r1/m.py::f", "python", 1, 2, 0)],
            [("Function", "/b/r2/x.rs::h", "rust", 1, 2, 0)],
        )
        self.assertEqual(rep.nodes_by_language["python"]["only_crg"], ["m.py::f"])
        self.assertEqual(rep.nodes_by_language["rust"]["only_rust"], ["x.rs::h"])
        self.assertEqual(rep.nodes_by_language["rust"]["jaccard"], 0.0)

    def test_language_mismatch_bucketed_under_crg_language(self):
        rep = self.pair(
            [("Function", "/a/r1/m.py::f", "python", 1, 2, 0)],
            [("Function", "/b/r2/m.py::f", "unknown", 1, 2, 0)],
        )
        self.assertEqual(list(rep.nodes_by_language), ["python"])
        py = rep.nodes_by_language["python"]
        self.assertEqual((py["both"], py["only_crg"], py["only_rust"]), (1, [], []))
        self.assertEqual(
            py["divergent"],
            [{"qn": "m.py::f", "field": "language", "crg": "python", "rust": "unknown"}],
        )

    def test_flows_and_communities_compared(self):
        def nodes(r):
            return [("Function", f"{r}/m.py::{n}", "python", 1, 2, 0) for n in "abcde"]

        crg = make_db(self.d / "crg.db", "/a/r1", nodes("/a/r1"))
        rust = make_db(self.d / "rust.db", "/b/r2", nodes("/b/r2"))
        q = lambda r, n: f"{r}/m.py::{n}"
        add_analysis(
            crg,
            [(q("/a/r1", "a"), [q("/a/r1", x) for x in "abc"], 0.5),
             (q("/a/r1", "d"), [q("/a/r1", x) for x in "de"], 0.2)],
            {q("/a/r1", x): c for x, c in zip("abcde", [1, 1, 2, 2, 3])},
            "leiden", 3,
        )
        add_analysis(
            rust,
            [(q("/b/r2", "a"), [q("/b/r2", x) for x in "ab"], 0.4),
             (q("/b/r2", "e"), [q("/b/r2", "e")], 0.1)],
            {q("/b/r2", x): c for x, c in zip("abcde", [9, 9, 8, 8, 7])},
            "leiden-rust", 3,
        )
        rep = compare(load_graph(crg, "/a/r1"), load_graph(rust, "/b/r2"))
        f = rep.flows_by_language["python"]
        self.assertEqual((f["crg"], f["rust"], f["both"]), (2, 2, 1))
        self.assertAlmostEqual(f["entry_jaccard"], 1 / 3)
        self.assertAlmostEqual(f["member_jaccard_mean"], 2 / 3)
        self.assertAlmostEqual(f["criticality_delta_max"], 0.1)
        self.assertEqual((f["only_crg"], f["only_rust"]), (["m.py::d"], ["m.py::e"]))
        c = rep.communities
        self.assertEqual((c["crg"], c["rust"], c["common_nodes"], c["ari"]), (3, 3, 5, 1.0))
        self.assertEqual((c["crg_detector"], c["rust_detector"]), ("leiden", "leiden-rust"))
        md = render_markdown(rep)
        self.assertIn("## Flows by language", md)
        self.assertIn("| 3 | 3 | leiden | leiden-rust | 5 | 1.000 |", md)
        json.dumps(asdict(rep), default=sorted)

    def test_missing_analysis_tables_load_empty(self):
        rep = self.pair(
            [("Function", "/a/r1/m.py::f", "python", 1, 2, 0)],
            [("Function", "/b/r2/m.py::f", "python", 1, 2, 0)],
        )
        self.assertEqual(rep.flows_by_language, {})
        self.assertEqual(rep.communities["ari"], None)
        self.assertEqual(rep.communities["common_nodes"], 0)

    def test_load_graph_path_with_special_chars(self):
        d = self.d / "a b#c"
        d.mkdir()
        db = make_db(d / "g.db", "/a/r1", [("Function", "/a/r1/m.py::f", "python", 1, 2, 0)])
        self.assertIn("m.py::f", load_graph(db, "/a/r1").nodes)

    def test_prepare_copies_drops_existing_index(self):
        src = self.d / "src"
        (src / ".code-review-graph").mkdir(parents=True)
        (src / ".code-review-graph" / "graph.db").write_text("x")
        (src / ".git").mkdir()
        (src / "target").mkdir()
        (src / "m.py").write_text("x = 1\n")
        crg, rust = prepare_copies(src, self.d / "work")
        self.assertFalse((crg / ".code-review-graph").exists())
        self.assertTrue((rust / ".git").is_dir())
        self.assertFalse((rust / "target").exists())
        self.assertEqual(sorted(p.name for p in crg.iterdir() if p.name != ".git"), ["m.py"])

    def test_empty_union_is_full_agreement(self):
        self.assertEqual(compare(Graph(), Graph()).edges_by_kind, {})
        rep = self.pair([], [])
        self.assertEqual(rep.nodes_by_language, {})

    def test_report_serializable_and_markdown(self):
        rep = self.pair(
            [("Function", "/a/r1/m.py::f", "python", 1, 2, 0)],
            [("Function", "/b/r2/m.py::f", "python", 1, 3, 0)],
        )
        json.dumps(asdict(rep))
        self.assertIn("| python |", render_markdown(rep))

    def test_cli_compare(self):
        make_db(self.d / "c.db", "/a/r1", [("Function", "/a/r1/m.py::f", "python", 1, 2, 0)])
        make_db(self.d / "r.db", "/b/r2", [("Function", "/b/r2/m.py::f", "python", 1, 2, 0)])
        out = self.d / "o.json"
        main(["compare", str(self.d / "c.db"), "/a/r1", str(self.d / "r.db"), "/b/r2",
              "--json", str(out), "--md", str(self.d / "o.md")])
        self.assertEqual(json.loads(out.read_text())["nodes_by_language"]["python"]["both"], 1)

    def test_git_prefix_never_signs_or_runs_hooks(self):
        self.assertIn("commit.gpgsign=false", GIT)
        self.assertIn("core.hooksPath=/dev/null", GIT)

    def _repo(self):
        repo = self.d / "crg"
        repo.mkdir()
        for args in (["init", "-q"], ["commit", "-q", "--allow-empty", "-m", "x"]):
            subprocess.run(GIT + args, cwd=repo, check=True, capture_output=True)
        head = subprocess.run(["git", "-C", str(repo), "rev-parse", "HEAD"], check=True,
                              capture_output=True, text=True).stdout.strip()
        return repo, head

    def test_crg_commit_matches_checkout_by_prefix(self):
        repo, head = self._repo()
        check_crg_commit(repo, head[:7])
        check_crg_commit(repo, head)
        with self.assertRaises(SystemExit) as e:
            check_crg_commit(repo, "0000000")
        self.assertIn("0000000", str(e.exception))
        self.assertIn(head[:7], str(e.exception))

    def test_corpus_aborts_when_checkout_differs(self):
        repo, _ = self._repo()
        corpus = self.d / "corpus.json"
        corpus.write_text(json.dumps({"crg_commit": "0000000", "languages": {"python": ["x.py"]}}))
        with self.assertRaises(SystemExit):
            run_corpus(corpus, repo, self.d / "work", repo / "fixtures", "codegraph")
        self.assertFalse((self.d / "work").exists())


class P1RegressionTest(unittest.TestCase):
    """The P1 baseline check parses the raw harness output, not re-indexes anything."""

    REPORT = """# Parity report

### Nodes by language

| language | crg | rust | both | jaccard | divergent |
|---|---|---|---|---|---|
| python | 31 | 31 | 31 | 1.000 | 0 |

#### Edges by kind

| kind | crg | rust | both | jaccard | resolved jaccard | line agreement |
|---|---|---|---|---|---|---|
| CALLS | 25 | 25 | 24 | 0.923 | 0.833 | 1.000 |
| IMPORTS_FROM | 5 | 5 | 5 | 1.000 | 1.000 | 1.000 |

### Flows by language

| language | crg | rust | both | entry jaccard | members jaccard (mean/min) | criticality delta (mean/max) |
|---|---|---|---|---|---|---|
| python | 3 | 4 | 3 | 0.750 | 0.833 / 0.500 | 0.020 / 0.020 |

### Communities

| crg | rust | crg detector | rust detector | common nodes | ARI |
|---|---|---|---|---|---|
| 8 | 5 | leiden | leiden | 16 | 0.403 |
"""

    def setUp(self):
        self.d = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.d, ignore_errors=True)
        (self.d / "corpus.md").write_text(
            "## Language corpus: python\n\n" + "\n".join(
                "#" + line if line.startswith("#") else line
                for line in self.REPORT.splitlines()
            ) + "\n"
        )
        (self.d / "probes").mkdir()
        (self.d / "probes" / "rust-imports.md").write_text(self.REPORT)
        (self.d / "inheritance").mkdir()
        (self.d / "inheritance" / "compare.md").write_text(
            "| language | set | compared | identical | Σ∩ | Σ∪ | Jaccard |\n"
            "|---|---|---|---|---|---|---|\n"
            "| py | inheritors_of | 7 | 4 | 1 | 4 | 0.250 |\n"
        )
        (self.d / "changes").mkdir()
        (self.d / "changes" / "rust-head3").mkdir()
        (self.d / "changes" / "rust-head3" / "compare.md").write_text(
            "| set | CRG | Rust | both | Jaccard |\n"
            "|---|---|---|---|---|\n"
            "| changed files | 15 | 15 | 15 | 1.000 |\n"
            "| impacted nodes (depth 2, ≤1000) | 186 | 192 | 184 | 0.948 |\n"
            "\nRISK_EQUAL 31/35\n"
        )
        (self.d / "timing.json").write_text(json.dumps({"rust_update_s_debug": 70.0}))

    def test_parse_report_reads_four_tables(self):
        r = parse_report(self.REPORT)
        self.assertEqual(r["nodes"]["python"]["jaccard"], 1.0)
        self.assertEqual(r["edges"]["CALLS"]["jaccard"], 0.923)
        self.assertEqual(r["edges"]["CALLS"]["resolved jaccard"], 0.833)
        self.assertEqual(r["edges"]["CALLS"]["line agreement"], 1.0)
        self.assertEqual(r["flows"]["python"]["entry jaccard"], 0.750)
        self.assertEqual(r["flows"]["python"]["members jaccard (mean/min)"], "0.833 / 0.500")
        self.assertEqual(r["ari"], 0.403)

    def test_collect_metrics_accepts_a_probe_only_directory(self):
        (self.d / "corpus.md").unlink()
        shutil.rmtree(self.d / "changes")
        shutil.rmtree(self.d / "inheritance")
        m = collect_metrics(self.d)
        self.assertEqual(m["probes.rust-imports.IMPORTS_FROM"], 1.0)
        self.assertNotIn("corpus.python.CALLS", m)

    def test_collect_metrics_from_snapshot_dir(self):
        m = collect_metrics(self.d)
        self.assertEqual(m["corpus.python.nodes"], 1.0)
        self.assertEqual(m["corpus.python.CALLS"], 0.923)
        self.assertEqual(m["probes.rust-imports.IMPORTS_FROM"], 1.0)
        self.assertEqual(m["inheritance.py.inheritors_of"], 0.250)
        self.assertEqual(m["changes.rust-head3.impacted"], 0.948)
        self.assertEqual(m["changes.rust-head3.risk_equal"], 31 / 35)
        self.assertEqual(m["timing.rust_update_s_debug"], 70.0)

    def test_evaluate_fails_only_on_a_drop(self):
        limits = {"corpus.python.CALLS": 0.923, "changes.rust-head3.impacted": 0.948}
        failures = evaluate(
            {"corpus.python.CALLS": 0.923, "changes.rust-head3.impacted": 1.0}, limits
        )
        self.assertEqual(failures, [])
        failures = evaluate(
            {"corpus.python.CALLS": 0.922, "changes.rust-head3.impacted": 0.947}, limits
        )
        self.assertEqual(len(failures), 2)
        self.assertTrue(any("corpus.python.CALLS" in f for f in failures))


class RiskEqualityTest(unittest.TestCase):
    def test_counts_equal_scores_within_tolerance(self):
        equal, common = equal_risk_count(
            {"a": 0.5, "b": 0.2}, {"a": 0.5005, "b": 0.3}, 0.001
        )
        self.assertEqual((equal, common), (1, 2))

    def test_missing_function_is_not_a_match(self):
        equal, common = equal_risk_count({"a": 0.5, "b": 0.2}, {"a": 0.5}, 0.001)
        self.assertEqual((equal, common), (1, 1))


if __name__ == "__main__":
    unittest.main()
