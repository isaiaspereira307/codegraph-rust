# ABOUTME: Unit tests for the parity graph.db comparator, using synthetic SQLite databases.
# ABOUTME: Run with: python3 -m unittest discover -s scripts/parity -v
import json
import sqlite3
import tempfile
import unittest
from dataclasses import asdict
from pathlib import Path

from parity import Graph, compare, load_graph, main, prepare_copies, render_markdown


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


if __name__ == "__main__":
    unittest.main()
