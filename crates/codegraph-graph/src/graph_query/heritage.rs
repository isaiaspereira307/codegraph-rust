// ABOUTME: Binds text INHERITS/IMPLEMENTS targets (`Base`, `unittest.TestCase`) to type nodes.
// ABOUTME: Pure resolution by bare name with a same-file, imported-file, unique-repo tie-break.

use std::collections::BTreeMap;

pub(crate) struct HeritageNode<'a> {
    pub qualified: &'a str,
    pub name: &'a str,
    pub kind: Option<&'a str>,
    pub file_path: &'a str,
    pub language: Option<&'a str>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct HeritageResolution {
    /// Source node index → base node index.
    pub bound: Vec<(usize, usize)>,
    /// Source node index and the heritage text that named more than one base.
    pub ambiguous: Vec<(usize, String)>,
}

/// Kinds a base may have (code-review-graph's `inheritors_of` list).
const TYPE_KINDS: [&str; 6] = ["Class", "Interface", "Type", "Struct", "Enum", "Trait"];

pub(crate) fn is_heritage(kind: &str) -> bool {
    kind == "INHERITS" || kind == "IMPLEMENTS"
}

/// The bare name a heritage text names: its last segment split on `.` and `::`.
pub(crate) fn last_segment(text: &str) -> &str {
    let tail = text.rsplit("::").next().unwrap_or(text);
    tail.rsplit('.').next().unwrap_or(tail)
}

/// Languages that may share a bare base name: JS/TS/TSX and C/C++; others only themselves.
fn family(language: Option<&str>) -> Option<String> {
    let lang = language?.to_ascii_lowercase();
    Some(match lang.as_str() {
        "javascript" | "typescript" | "tsx" => "javascript".to_string(),
        "c" | "cpp" => "c".to_string(),
        _ => lang,
    })
}

/// Binds each `(source, text)` heritage edge to one type node named by the text's last
/// segment, in a compatible language. Tiers, first non-empty wins: candidates in the source's
/// file, then in files it imports, then the whole repository; a tier with one candidate binds,
/// a tier with several is ambiguous. No candidate (external base) leaves the edge unbound.
pub(crate) fn resolve_heritage(
    nodes: &[HeritageNode<'_>],
    heritage: &[(usize, String)],
    imported_files: &dyn Fn(usize) -> Vec<String>,
) -> HeritageResolution {
    let mut by_name: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (ix, node) in nodes.iter().enumerate() {
        if node.kind.is_some_and(|k| TYPE_KINDS.contains(&k)) {
            by_name.entry(node.name).or_default().push(ix);
        }
    }
    for candidates in by_name.values_mut() {
        candidates.sort_by_key(|&ix| (nodes[ix].file_path, nodes[ix].qualified));
    }
    let mut out = HeritageResolution::default();
    for (source, text) in heritage {
        let name = last_segment(text);
        if name.is_empty() {
            continue;
        }
        let Some(all) = by_name.get(name) else {
            continue;
        };
        let src = &nodes[*source];
        let src_family = family(src.language);
        let candidates: Vec<usize> = all
            .iter()
            .copied()
            .filter(|&ix| ix != *source && family(nodes[ix].language) == src_family)
            .collect();
        if candidates.is_empty() {
            continue;
        }
        let same_file: Vec<usize> = candidates
            .iter()
            .copied()
            .filter(|&ix| nodes[ix].file_path == src.file_path)
            .collect();
        let tier = if !same_file.is_empty() {
            same_file
        } else {
            let imported = imported_files(*source);
            let in_imports: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|&ix| imported.iter().any(|f| f == nodes[ix].file_path))
                .collect();
            if in_imports.is_empty() {
                candidates
            } else {
                in_imports
            }
        };
        if let [base] = tier[..] {
            out.bound.push((*source, base));
        } else {
            out.ambiguous.push((*source, text.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n<'a>(qualified: &'a str, kind: &'a str, file: &'a str, lang: &'a str) -> HeritageNode<'a> {
        let name = qualified.rsplit("::").next().unwrap();
        HeritageNode {
            qualified,
            name,
            kind: Some(kind),
            file_path: file,
            language: Some(lang),
        }
    }

    fn no_imports(_: usize) -> Vec<String> {
        Vec::new()
    }

    fn edge(source: usize, text: &str) -> Vec<(usize, String)> {
        vec![(source, text.to_string())]
    }

    #[test]
    fn last_segment_of_dotted_and_scoped_names() {
        assert_eq!(last_segment("Base"), "Base");
        assert_eq!(last_segment("unittest.TestCase"), "TestCase");
        assert_eq!(last_segment("a.b.Base"), "Base");
        assert_eq!(last_segment("crate::model::Base"), "Base");
        assert_eq!(last_segment("ns::pkg.Base"), "Base");
        assert_eq!(last_segment(""), "");
    }

    #[test]
    fn same_file_candidate_wins() {
        let nodes = [
            n("a.py::Child", "Class", "a.py", "python"),
            n("a.py::Base", "Class", "a.py", "python"),
            n("b.py::Base", "Class", "b.py", "python"),
        ];
        let r = resolve_heritage(&nodes, &edge(0, "Base"), &no_imports);
        assert_eq!(r.bound, vec![(0, 1)]);
        assert!(r.ambiguous.is_empty());
    }

    #[test]
    fn imported_file_candidate_wins_over_repo_wide_duplicates() {
        let nodes = [
            n("a.py::Child", "Class", "a.py", "python"),
            n("b.py::Base", "Class", "b.py", "python"),
            n("c.py::Base", "Class", "c.py", "python"),
        ];
        let imports = |ix: usize| {
            assert_eq!(ix, 0);
            vec!["c.py".to_string()]
        };
        let r = resolve_heritage(&nodes, &edge(0, "pkg.Base"), &imports);
        assert_eq!(r.bound, vec![(0, 2)]);
        assert!(r.ambiguous.is_empty());
    }

    #[test]
    fn unique_repo_candidate_binds() {
        let nodes = [
            n("a.ts::Child", "Class", "a.ts", "typescript"),
            n("b.ts::Shape", "Interface", "b.ts", "typescript"),
        ];
        let r = resolve_heritage(&nodes, &edge(0, "Shape"), &no_imports);
        assert_eq!(r.bound, vec![(0, 1)]);
        assert!(r.ambiguous.is_empty());
    }

    #[test]
    fn two_candidates_without_a_tie_break_are_ambiguous() {
        let nodes = [
            n("a.py::Child", "Class", "a.py", "python"),
            n("b.py::Base", "Class", "b.py", "python"),
            n("c.py::Base", "Class", "c.py", "python"),
        ];
        let r = resolve_heritage(&nodes, &edge(0, "Base"), &no_imports);
        assert!(r.bound.is_empty());
        assert_eq!(r.ambiguous, vec![(0, "Base".to_string())]);
    }

    #[test]
    fn external_base_stays_unbound() {
        let nodes = [n("a.py::Child", "Class", "a.py", "python")];
        let heritage = vec![
            (0, "unittest.TestCase".to_string()),
            (0, "object".to_string()),
            (0, String::new()),
        ];
        let r = resolve_heritage(&nodes, &heritage, &no_imports);
        assert_eq!(r, HeritageResolution::default());
    }

    #[test]
    fn function_named_like_the_base_is_not_a_candidate() {
        let nodes = [
            n("a.py::Child", "Class", "a.py", "python"),
            n("a.py::Base", "Function", "a.py", "python"),
            n("b.py::Base", "Class", "b.py", "python"),
        ];
        let r = resolve_heritage(&nodes, &edge(0, "Base"), &no_imports);
        assert_eq!(r.bound, vec![(0, 2)]);
        assert!(r.ambiguous.is_empty());
    }

    #[test]
    fn several_same_file_candidates_are_ambiguous_even_with_one_imported() {
        // A nested `Outer.Base` next to `Base`, or TS `interface Foo` merged with `class Foo`:
        // the local declarations shadow the import, so the import must not win.
        let nodes = [
            n("a.py::Child", "Class", "a.py", "python"),
            n("a.py::Base", "Class", "a.py", "python"),
            HeritageNode {
                name: "Base",
                ..n("a.py::Outer.Base", "Class", "a.py", "python")
            },
            n("b.py::Base", "Class", "b.py", "python"),
        ];
        let imports = |_: usize| vec!["b.py".to_string()];
        let r = resolve_heritage(&nodes, &edge(0, "Base"), &imports);
        assert!(r.bound.is_empty());
        assert_eq!(r.ambiguous, vec![(0, "Base".to_string())]);
    }

    #[test]
    fn several_imported_candidates_are_ambiguous_even_with_one_elsewhere() {
        let nodes = [
            n("a.py::Child", "Class", "a.py", "python"),
            n("b.py::Base", "Class", "b.py", "python"),
            n("c.py::Base", "Class", "c.py", "python"),
            n("d.py::Base", "Class", "d.py", "python"),
        ];
        let imports = |_: usize| vec!["b.py".to_string(), "c.py".to_string()];
        let r = resolve_heritage(&nodes, &edge(0, "Base"), &imports);
        assert!(r.bound.is_empty());
        assert_eq!(r.ambiguous, vec![(0, "Base".to_string())]);
    }

    #[test]
    fn self_edge_is_dropped() {
        let nodes = [n("a.py::A", "Class", "a.py", "python")];
        let r = resolve_heritage(&nodes, &edge(0, "A"), &no_imports);
        assert_eq!(r, HeritageResolution::default());
    }

    #[test]
    fn js_and_ts_share_a_family_but_python_does_not() {
        let nodes = [
            n("a.js::Child", "Class", "a.js", "javascript"),
            n("b.ts::Base", "Class", "b.ts", "typescript"),
            n("c.py::Base", "Class", "c.py", "python"),
            n("d.py::PyChild", "Class", "d.py", "python"),
            n("e.tsx::Widget", "Class", "e.tsx", "tsx"),
            n("f.py::Widget", "Class", "f.py", "python"),
        ];
        let heritage = vec![
            (0, "Base".to_string()),
            (3, "Widget".to_string()),
            (0, "Widget".to_string()),
        ];
        let r = resolve_heritage(&nodes, &heritage, &no_imports);
        // JS child binds to the TS base (one family); the Python child binds only to the
        // Python Widget; the JS child binds to the TSX Widget, never the Python one.
        assert_eq!(r.bound, vec![(0, 1), (3, 5), (0, 4)]);
        assert!(r.ambiguous.is_empty());
    }
}
