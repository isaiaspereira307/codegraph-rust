# ABOUTME: Parity harness comparing code-review-graph and Rust graph.db files (schema v15).
# ABOUTME: Loads nodes/edges with root-relative keys and reports per-language/per-kind Jaccard.
import argparse
import json
import shutil
import sqlite3
import subprocess
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
    con = sqlite3.connect(Path(db).resolve().as_uri() + "?mode=ro", uri=True)
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
    lang_mismatch = {}
    for qn, n in rust.nodes.items():
        lang = n.language
        if qn in crg.nodes:
            lang = crg.nodes[qn].language
            if lang != n.language:
                lang_mismatch[qn] = n.language
        langs.setdefault(lang, [set(), set()])[1].add(qn)
    for lang in sorted(langs):
        a, b = langs[lang]
        both = a & b
        divergent = []
        for qn in sorted(both):
            ca, ra = crg.nodes[qn], rust.nodes[qn]
            if qn in lang_mismatch:
                divergent.append(
                    {"qn": qn, "field": "language", "crg": ca.language, "rust": lang_mismatch[qn]}
                )
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


IGNORED = (".git", "target", "node_modules", ".code-review-graph")
GIT = ["git", "-c", "user.name=parity", "-c", "user.email=parity@localhost"]
TIMEOUT = 600


def prepare_copies(src, work):
    """Copy src to work/crg and work/rust (no VCS/build/index dirs), each a fresh git repo."""
    src, work = Path(src), Path(work)
    out = []
    for name in ("crg", "rust"):
        dest = work / name
        if dest.exists():
            shutil.rmtree(dest)
        shutil.copytree(src, dest, ignore=shutil.ignore_patterns(*IGNORED))
        for args in (["init", "-q"], ["add", "-A"], ["commit", "-q", "--allow-empty", "-m", "parity"]):
            subprocess.run(GIT + args, cwd=dest, check=True, capture_output=True)
        out.append(dest)
    return tuple(out)


def _run_tool(cmd, cwd, label):
    try:
        r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=TIMEOUT)
    except subprocess.TimeoutExpired:
        raise SystemExit(f"{label} timed out after {TIMEOUT}s")
    if r.returncode != 0:
        raise SystemExit(f"{label} failed ({r.returncode}):\n{r.stderr}")


def _check_crg_schema(db):
    con = sqlite3.connect(Path(db).resolve().as_uri() + "?mode=ro", uri=True)
    try:
        row = con.execute("SELECT value FROM metadata WHERE key='schema_version'").fetchone()
    finally:
        con.close()
    if not row or str(row[0]) != "15":
        raise SystemExit(
            f"CRG graph.db schema_version is {row[0] if row else None}, expected 15: the "
            "installed code-review-graph 2.3.7 (schema 9) was probably used; run CRG from source "
            "with `uv run --frozen --project <crg_repo> code-review-graph`."
        )


def run_pair(src, work, crg_repo, codegraph):
    """Index src with CRG and Rust in separate copies; return the comparison Report."""
    crg_dir, rust_dir = prepare_copies(src, work)
    crg_dir, rust_dir = crg_dir.resolve(), rust_dir.resolve()
    _run_tool(
        ["uv", "run", "--frozen", "--project", str(Path(crg_repo).resolve()),
         "code-review-graph", "build"],
        crg_dir, "code-review-graph build",
    )
    _run_tool([str(Path(codegraph).resolve()), "index", ".", "-r"], rust_dir, "codegraph index")
    crg_db = crg_dir / ".code-review-graph" / "graph.db"
    rust_db = rust_dir / ".code-review-graph" / "graph.db"
    _check_crg_schema(crg_db)
    return compare(load_graph(crg_db, crg_dir), load_graph(rust_db, rust_dir))


def _emit(rep, md, js):
    if js:
        Path(js).write_text(json.dumps(asdict(rep), indent=2))
    text = render_markdown(rep)
    if md:
        Path(md).write_text(text)
    else:
        sys.stdout.write(text)


def run_corpus(corpus_file, crg_repo, work, crg_repo_fixtures, codegraph):
    corpus = json.loads(Path(corpus_file).read_text())
    fixtures = Path(crg_repo_fixtures)
    parts = [f"# Parity baseline\n\nCRG commit `{corpus['crg_commit']}`; corpus: fixtures from "
             "`tests/fixtures`, one repository per language.\n"]
    for lang, files in corpus["languages"].items():
        src = Path(work) / "src" / lang
        if src.exists():
            shutil.rmtree(src)
        src.mkdir(parents=True)
        for f in files:
            shutil.copy(fixtures / f, src / Path(f).name)
        rep = run_pair(src, Path(work) / lang, crg_repo, codegraph)
        lines = render_markdown(rep).splitlines()[1:]
        demoted = ["#" + ln if ln.startswith("#") else ln for ln in lines]
        parts.append(f"## Language corpus: {lang}\n" + "\n".join(demoted) + "\n")
    return "\n".join(parts)


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
    r = sub.add_parser("run", help="index a directory with both tools and compare")
    r.add_argument("src_dir", type=Path)
    r.add_argument("--work", type=Path, required=True)
    r.add_argument("--crg-repo", type=Path, default=Path("../code-review-graph"))
    r.add_argument("--codegraph", type=Path, default=Path("target/debug/codegraph"))
    r.add_argument("--json", type=Path)
    r.add_argument("--md", type=Path)
    k = sub.add_parser("corpus", help="run the per-language fixture corpus")
    k.add_argument("--work", type=Path, required=True)
    k.add_argument("--crg-repo", type=Path, default=Path("../code-review-graph"))
    k.add_argument("--codegraph", type=Path, default=Path("target/debug/codegraph"))
    k.add_argument("--corpus", type=Path, default=Path(__file__).with_name("corpus.json"))
    k.add_argument("--md", type=Path)
    args = p.parse_args(argv)
    if args.cmd == "run":
        _emit(run_pair(args.src_dir, args.work, args.crg_repo, args.codegraph), args.md, args.json)
        return 0
    if args.cmd == "corpus":
        text = run_corpus(args.corpus, args.crg_repo, args.work,
                          args.crg_repo / "tests" / "fixtures", args.codegraph)
        if args.md:
            args.md.parent.mkdir(parents=True, exist_ok=True)
            args.md.write_text(text)
        else:
            sys.stdout.write(text)
        return 0
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
