# ABOUTME: Parity harness comparing code-review-graph and Rust graph.db files (schema v15).
# ABOUTME: Loads nodes/edges with root-relative keys and reports per-language/per-kind Jaccard.
import argparse
import json
import sqlite3
import sys
from dataclasses import asdict, dataclass, field
from pathlib import Path

LIST_CAP = 50
DIVERGENT_FIELDS = ("kind", "line_start", "line_end", "is_test")

EdgeKey = tuple  # (kind, source, target), all root-relative


@dataclass(frozen=True)
class NodeRow:
    kind: str
    file: str
    line_start: int
    line_end: int
    is_test: int
    language: str


@dataclass
class Graph:
    nodes: dict = field(default_factory=dict)  # qn (relative) -> NodeRow
    edges: set = field(default_factory=set)  # EdgeKey


@dataclass
class Report:
    nodes_by_language: dict = field(default_factory=dict)
    edges_by_kind: dict = field(default_factory=dict)


def _strip(value, prefix):
    if value is not None and prefix and value.startswith(prefix):
        return value[len(prefix):]
    return value


def load_graph(db, root):
    prefix = str(root).rstrip("/") + "/"
    graph = Graph()
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    try:
        for kind, qn, fp, ls, le, lang, is_test in con.execute(
            "SELECT kind, qualified_name, file_path, line_start, line_end, language, is_test "
            "FROM nodes"
        ):
            graph.nodes[_strip(qn, prefix)] = NodeRow(
                kind, _strip(fp, prefix), ls, le, int(is_test or 0), lang or "unknown"
            )
        for kind, src, tgt in con.execute(
            "SELECT kind, source_qualified, target_qualified FROM edges"
        ):
            graph.edges.add((kind, _strip(src, prefix), _strip(tgt, prefix)))
    finally:
        con.close()
    return graph


def _jaccard(both, union):
    return 1.0 if union == 0 else both / union


def _cap(items):
    return sorted(items)[:LIST_CAP]


def compare(crg, rust):
    report = Report()

    langs = {}
    for qn, n in crg.nodes.items():
        langs.setdefault(n.language, [set(), set()])[0].add(qn)
    for qn, n in rust.nodes.items():
        langs.setdefault(n.language, [set(), set()])[1].add(qn)
    for lang in sorted(langs):
        a, b = langs[lang]
        both = a & b
        divergent = []
        for qn in sorted(both):
            ca, ra = crg.nodes[qn], rust.nodes[qn]
            for f in DIVERGENT_FIELDS:
                if getattr(ca, f) != getattr(ra, f):
                    divergent.append(
                        {"qn": qn, "field": f, "crg": getattr(ca, f), "rust": getattr(ra, f)}
                    )
        report.nodes_by_language[lang] = {
            "crg": len(a),
            "rust": len(b),
            "both": len(both),
            "jaccard": _jaccard(len(both), len(a | b)),
            "only_crg": _cap(a - b),
            "only_rust": _cap(b - a),
            "divergent": divergent[:LIST_CAP],
        }

    def by_kind(g):
        out = {}
        for e in g.edges:
            out.setdefault(e[0], set()).add(e)
        return out

    ck, rk = by_kind(crg), by_kind(rust)
    for kind in sorted(set(ck) | set(rk)):
        a, b = ck.get(kind, set()), rk.get(kind, set())
        both = a & b
        ra = {e for e in a if e[2] in crg.nodes}
        rb = {e for e in b if e[2] in rust.nodes}
        report.edges_by_kind[kind] = {
            "crg": len(a),
            "rust": len(b),
            "both": len(both),
            "jaccard": _jaccard(len(both), len(a | b)),
            "only_crg": [list(e) for e in _cap(a - b)],
            "only_rust": [list(e) for e in _cap(b - a)],
            "resolved_jaccard": _jaccard(len(ra & rb), len(ra | rb)),
        }
    return report


def render_markdown(report):
    lines = ["# Parity report", "", "## Nodes by language", ""]
    lines += ["| language | crg | rust | both | jaccard | divergent |", "|---|---|---|---|---|---|"]
    for lang, r in report.nodes_by_language.items():
        lines.append(
            f"| {lang} | {r['crg']} | {r['rust']} | {r['both']} | {r['jaccard']:.3f} "
            f"| {len(r['divergent'])} |"
        )
    lines += ["", "## Edges by kind", ""]
    lines += [
        "| kind | crg | rust | both | jaccard | resolved jaccard |",
        "|---|---|---|---|---|---|",
    ]
    for kind, r in report.edges_by_kind.items():
        lines.append(
            f"| {kind} | {r['crg']} | {r['rust']} | {r['both']} | {r['jaccard']:.3f} "
            f"| {r['resolved_jaccard']:.3f} |"
        )
    for lang, r in report.nodes_by_language.items():
        for title, key in (("only in CRG", "only_crg"), ("only in Rust", "only_rust")):
            if r[key]:
                lines += ["", f"### {lang}: nodes {title}", ""] + [f"- `{q}`" for q in r[key]]
        if r["divergent"]:
            lines += ["", f"### {lang}: divergent nodes", ""]
            lines += [
                f"- `{d['qn']}` {d['field']}: crg={d['crg']} rust={d['rust']}"
                for d in r["divergent"]
            ]
    for kind, r in report.edges_by_kind.items():
        for title, key in (("only in CRG", "only_crg"), ("only in Rust", "only_rust")):
            if r[key]:
                lines += ["", f"### {kind}: edges {title}", ""]
                lines += [f"- `{s}` -> `{t}`" for _, s, t in r[key]]
    return "\n".join(lines) + "\n"


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("compare", help="compare two graph.db files")
    c.add_argument("crg_db", type=Path)
    c.add_argument("crg_root")
    c.add_argument("rust_db", type=Path)
    c.add_argument("rust_root")
    c.add_argument("--json", type=Path)
    c.add_argument("--md", type=Path)
    args = p.parse_args(argv)
    if args.cmd == "compare":
        rep = compare(load_graph(args.crg_db, args.crg_root), load_graph(args.rust_db, args.rust_root))
        if args.json:
            args.json.write_text(json.dumps(asdict(rep), indent=2))
        md = render_markdown(rep)
        if args.md:
            args.md.write_text(md)
        else:
            sys.stdout.write(md)
    return 0


if __name__ == "__main__":
    sys.exit(main())
