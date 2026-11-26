# ABOUTME: Checks a P1 parity snapshot against the frozen baseline; exits 0/1.
# ABOUTME: Usage: python3 scripts/parity/check_p1.py [--print] <snapshot-dir>
"""Regression detector for the frozen P1 baseline.

Reads the raw Markdown/JSON emitted by p1_snapshot.sh; it never re-indexes a repository.
Every limit is a floor: an improvement passes, a drop fails.
"""
import json
import re
import sys
from pathlib import Path

# Frozen in docs/parity/p1-baseline.md. Values are floors, not exact expectations.
LIMITS = {
    "corpus.python.nodes": 1.000,
    "corpus.python.CALLS": 0.923,
    "corpus.python.CALLS.resolved": 0.833,
    "corpus.python.CALLS.line": 1.000,
    "corpus.python.CONTAINS": 1.000,
    "corpus.python.IMPORTS_FROM": 1.000,
    "corpus.python.INHERITS": 1.000,
    "corpus.python.TESTED_BY": 1.000,
    "corpus.python.flows.entry": 0.750,
    "corpus.python.flows.members_mean": 0.833,
    "corpus.python.flows.members_min": 0.500,
    "corpus.python.ari": 0.403,
    "corpus.typescript.nodes": 1.000,
    "corpus.typescript.CALLS": 1.000,
    "corpus.typescript.CONTAINS": 1.000,
    "corpus.typescript.IMPORTS_FROM": 1.000,
    "corpus.typescript.INHERITS": 1.000,
    "corpus.typescript.TESTED_BY": 1.000,
    "corpus.typescript.flows.entry": 1.000,
    "corpus.typescript.flows.members_mean": 1.000,
    "corpus.typescript.ari": 1.000,
    "corpus.go.nodes": 1.000,
    "corpus.go.CALLS": 1.000,
    "corpus.go.CONTAINS": 1.000,
    "corpus.go.IMPORTS_FROM": 1.000,
    "corpus.go.flows.entry": 1.000,
    "corpus.go.flows.members_mean": 1.000,
    "corpus.go.ari": 1.000,
    "corpus.rust.nodes": 1.000,
    "corpus.rust.CALLS": 1.000,
    "corpus.rust.CONTAINS": 1.000,
    "corpus.rust.IMPLEMENTS": 1.000,
    "corpus.rust.IMPORTS_FROM": 1.000,
    "corpus.rust.TESTED_BY": 1.000,
    "corpus.rust.flows.entry": 1.000,
    "corpus.rust.flows.members_mean": 1.000,
    "corpus.rust.ari": 1.000,
    "corpus.java.nodes": 1.000,
    "corpus.java.CALLS": 1.000,
    "corpus.java.CONTAINS": 1.000,
    "corpus.java.IMPORTS_FROM": 1.000,
    "corpus.java.INHERITS": 1.000,
    "corpus.java.INJECTS": 1.000,
    "corpus.java.flows.entry": 1.000,
    "corpus.java.flows.members_mean": 1.000,
    "corpus.java.ari": 1.000,
    "probes.python.nodes": 1.000,
    "probes.python.CALLS": 1.000,
    "probes.python.CONTAINS": 0.902,
    "probes.python.IMPORTS_FROM": 1.000,
    "probes.python.INHERITS": 1.000,
    "probes.python.TESTED_BY": 1.000,
    "probes.python.flows.entry": 1.000,
    "probes.python-imports.nodes": 1.000,
    "probes.python-imports.CALLS": 0.684,
    "probes.python-imports.CALLS.resolved": 0.786,
    "probes.python-imports.CONTAINS": 0.909,
    "probes.python-imports.IMPORTS_FROM": 1.000,
    "probes.python-imports.flows.entry": 1.000,
    "probes.rust-imports.nodes": 1.000,
    "probes.rust-imports.CALLS": 1.000,
    "probes.rust-imports.CONTAINS": 0.737,
    "probes.rust-imports.IMPORTS_FROM": 1.000,
    "probes.rust-imports.IMPLEMENTS": 1.000,
    "probes.typescript-js.nodes": 1.000,
    "probes.typescript-js.CALLS": 1.000,
    "probes.typescript-js.CONTAINS": 1.000,
    "probes.typescript-js.IMPORTS_FROM": 1.000,
    "probes.go-package.nodes": 1.000,
    "probes.go-package.CALLS": 1.000,
    "probes.go-package.CONTAINS": 1.000,
    "probes.go-package.IMPORTS_FROM": 1.000,
    "probes.go-package.TESTED_BY": 1.000,
    "probes.java-cross-file.nodes": 1.000,
    "probes.java-cross-file.CALLS": 1.000,
    "probes.java-cross-file.CONSUMES": 1.000,
    "probes.java-cross-file.CONTAINS": 1.000,
    "probes.java-cross-file.IMPORTS_FROM": 1.000,
    "probes.java-cross-file.INHERITS": 1.000,
    "probes.java-cross-file.INJECTS": 1.000,
    "probes.java-cross-file.flows.entry": 1.000,
    "probes.java-cross-file.ari": 1.000,
    "probes.java-framework.nodes": 1.000,
    "probes.java-framework.CALLS": 1.000,
    "probes.java-framework.CONTAINS": 1.000,
    "probes.java-framework.DEPENDS_ON_CONFIG": 1.000,
    "probes.java-framework.HANDLES": 1.000,
    "probes.java-framework.IMPORTS_FROM": 1.000,
    "probes.java-framework.PUBLISHES": 1.000,
    "probes.java-framework.TEMPORAL_STUB": 1.000,
    "probes.java-framework.TRIGGERS": 1.000,
    "probes.java-framework.flows.entry": 1.000,
    "probes.java-framework.ari": 1.000,
    "probes.swift-imports.nodes": 1.000,
    "probes.swift-imports.CALLS": 1.000,
    "probes.swift-imports.CONTAINS": 1.000,
    "probes.swift-imports.IMPORTS_FROM": 1.000,
    "probes.swift-imports.INHERITS": 1.000,
    "probes.swift-imports.TESTED_BY": 1.000,
    "probes.cpp-imports.nodes": 1.000,
    "probes.cpp-imports.CALLS": 1.000,
    "probes.cpp-imports.CONTAINS": 1.000,
    "probes.cpp-imports.IMPORTS_FROM": 1.000,
    "probes.cpp-imports.TESTED_BY": 1.000,
    "probes.csharp-imports.nodes": 1.000,
    "probes.csharp-imports.CALLS": 1.000,
    "probes.csharp-imports.CONTAINS": 1.000,
    "probes.csharp-imports.IMPORTS_FROM": 1.000,
    "probes.csharp-imports.TESTED_BY": 1.000,
    "probes.ruby-imports.nodes": 1.000,
    "probes.ruby-imports.CALLS": 1.000,
    "probes.ruby-imports.CONTAINS": 1.000,
    "probes.ruby-imports.IMPORTS_FROM": 1.000,
    "probes.ruby-imports.TESTED_BY": 1.000,
    "probes.ruby-imports.flows.entry": 1.000,
    "probes.ruby-imports.ari": 1.000,
    "probes.php-imports.nodes": 1.000,
    "probes.php-imports.CALLS": 1.000,
    "probes.php-imports.CONTAINS": 1.000,
    "probes.php-imports.IMPORTS_FROM": 1.000,
    "probes.php-imports.TESTED_BY": 1.000,
    "probes.php-imports.ari": 1.000,
    "probes.kotlin-imports.nodes": 1.000,
    "probes.kotlin-imports.CALLS": 1.000,
    "probes.kotlin-imports.CONTAINS": 1.000,
    "probes.kotlin-imports.IMPORTS_FROM": 1.000,
    "probes.kotlin-imports.TESTED_BY": 1.000,
    "probes.kotlin-imports.flows.entry": 1.000,
    "probes.kotlin-imports.ari": 1.000,
    "probes.dart-imports.nodes": 1.000,
    "probes.dart-imports.CALLS": 1.000,
    "probes.dart-imports.IMPORTS_FROM": 1.000,
    "probes.dart-imports.INHERITS": 1.000,
    "inheritance.go.inheritors_of": 1.000,
    "inheritance.go.impact_d1": 0.000,
    "inheritance.java.inheritors_of": 0.250,
    "inheritance.py.inheritors_of": 0.250,
    "inheritance.py.impact_d1": 0.500,
    "inheritance.ts.inheritors_of": 0.500,
    "inheritance.ts.impact_d1": 0.500,
    "changes.rust-head3.changed_files": 1.000,
    "changes.rust-head3.changed_functions": 1.000,
    "changes.rust-head3.impacted": 0.948,
    "changes.rust-head3.impacted_files": 0.976,
    "changes.rust-head3.risk_equal": 31 / 35,
    "changes.crg-synthetic-d1.changed_files": 1.000,
    "changes.crg-synthetic-d1.changed_functions": 1.000,
    "changes.crg-synthetic-d1.impacted": 0.995,
    "changes.crg-synthetic-d2.changed_files": 1.000,
    "changes.crg-synthetic-d2.changed_functions": 1.000,
    # Task 11: 0.960 -> 0.957. Tests reach tools/build.py::run_postprocess (only Rust reaches it:
    # CRG leaves a function-local `from M import f as _f; _f()` as text) through a local
    # `import ... as m; m.run_postprocess()` receiver that CRG binds too. See docs/parity/changes.md.
    "changes.crg-synthetic-d2.impacted": 0.957,
}

TOLERANCE = 0.0006  # Markdown rounds to three decimals.
_SECTIONS = {
    "nodes by language": "nodes",
    "edges by kind": "edges",
    "flows by language": "flows",
    "communities": "communities",
}


def _num(value):
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def parse_report(text):
    """Parse one parity report's Markdown tables into a small dictionary."""
    out = {"nodes": {}, "edges": {}, "flows": {}, "ari": None}
    section, header = None, None
    for raw in text.splitlines():
        line = raw.strip()
        if line.startswith("#"):
            section = _SECTIONS.get(line.lstrip("#").strip().lower())
            header = None
            continue
        if not line.startswith("|") or section is None:
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if all(set(c) <= set("-: ") for c in cells):
            continue
        if header is None:
            header = cells
            continue
        row = {}
        for key, value in zip(header, cells):
            number = _num(value)
            row[key] = number if number is not None else value
        if section == "nodes":
            out["nodes"][row["language"]] = row
        elif section == "edges":
            out["edges"][row["kind"]] = row
        elif section == "flows":
            out["flows"][row["language"]] = row
        elif section == "communities":
            out["ari"] = _num(row.get("ARI"))
    return out


def _primary(report):
    """Language row with the largest CRG node set (ignores Rust-only Directory rows)."""
    rows = report["nodes"].values()
    return max(rows, key=lambda r: int(r.get("crg") or 0), default=None)


def _add_report(metrics, prefix, report):
    nodes = _primary(report)
    if nodes:
        metrics[f"{prefix}.nodes"] = _num(nodes.get("jaccard"))
    for kind, row in report["edges"].items():
        metrics[f"{prefix}.{kind}"] = _num(row.get("jaccard"))
        metrics[f"{prefix}.{kind}.resolved"] = _num(row.get("resolved jaccard"))
        metrics[f"{prefix}.{kind}.line"] = _num(row.get("line agreement"))
    for lang, row in report["flows"].items():
        mean_min = row.get("members jaccard (mean/min)", "").split("/")
        metrics[f"{prefix}.flows.entry"] = _num(row.get("entry jaccard"))
        metrics[f"{prefix}.flows.members_mean"] = _num(mean_min[0])
        if len(mean_min) > 1:
            metrics[f"{prefix}.flows.members_min"] = _num(mean_min[1])
    metrics[f"{prefix}.ari"] = report["ari"]


_LABELS = {
    "impacted_nodes": "impacted",
    "changed_functions_non_file": "changed_functions",
    "affected_flows_entry_name": "affected_flows",
    "impacted_files_outside_changed_files": "impacted_files",
}


def _label(value):
    value = value.lower()
    value = re.sub(r"\(depth \d+.*?\)", "", value)
    label = re.sub(r"[^a-z0-9]+", "_", value).strip("_")
    return _LABELS.get(label, label)


def collect_metrics(snapshot):
    """Collect comparable numbers from a p1_snapshot.sh output directory."""
    snapshot = Path(snapshot)
    metrics = {}

    corpus_file = snapshot / "corpus.md"
    if corpus_file.exists():
        chunks = re.split(
            r"^## Language corpus: ([^\n]+)\s*$", corpus_file.read_text(), flags=re.MULTILINE
        )
        for lang, text in zip(chunks[1::2], chunks[2::2]):
            _add_report(metrics, f"corpus.{lang.strip()}", parse_report(text))

    for md in sorted((snapshot / "probes").glob("*.md")):
        _add_report(metrics, f"probes.{md.stem}", parse_report(md.read_text()))

    inherits = snapshot / "inheritance" / "compare.md"
    if inherits.exists():
        for line in inherits.read_text().splitlines():
            if not line.startswith("|"):
                continue
            cells = [c.strip() for c in line.strip("|").split("|")]
            if len(cells) == 7 and cells[0] not in ("language", "---"):
                metrics[f"inheritance.{cells[0]}.{_label(cells[1])}"] = _num(cells[6])

    for compare in sorted((snapshot / "changes").glob("*/compare.md")):
        scenario = compare.parent
        text = compare.read_text()
        for line in text.splitlines():
            if line.startswith("RISK_EQUAL"):
                equal, _, common = line.split()[1].partition("/")
                metrics[f"changes.{scenario.name}.risk_equal"] = (
                    int(equal) / int(common) if int(common) else None
                )
                continue
            if not line.startswith("|"):
                continue
            cells = [c.strip() for c in line.strip("|").split("|")]
            if len(cells) >= 5 and cells[0] not in ("set", "---"):
                metrics[f"changes.{scenario.name}.{_label(cells[0])}"] = _num(cells[4])

    timing = snapshot / "timing.json"
    if timing.exists():
        for key, value in json.loads(timing.read_text()).items():
            if isinstance(value, (int, float)):
                metrics[f"timing.{key}"] = float(value)
    return metrics


def evaluate(metrics, limits=LIMITS):
    """Return one message per failed floor; an empty list means no regression."""
    failures = []
    for key, floor in sorted(limits.items()):
        measured = metrics.get(key)
        if measured is None:
            failures.append(f"{key}: missing (floor {floor:.3f})")
        elif measured + TOLERANCE < floor:
            failures.append(f"{key}: {measured:.3f} < {floor:.3f}")
    return failures


def main(argv=None):
    args = sys.argv[1:] if argv is None else argv
    dump = "--print" in args
    args = [a for a in args if a != "--print"]
    if len(args) != 1:
        print("usage: check_p1.py [--print] <snapshot-dir>", file=sys.stderr)
        return 2
    metrics = collect_metrics(args[0])
    if dump:
        print(json.dumps(metrics, indent=2, sort_keys=True))
        return 0
    failures = evaluate(metrics)
    if failures:
        print(f"P1 regression: {len(failures)} metric(s) below baseline")
        for failure in failures:
            print(f"  {failure}")
        return 1
    print(f"P1 baseline OK: {len(LIMITS)} floors, no metric dropped")
    return 0


if __name__ == "__main__":
    sys.exit(main())
