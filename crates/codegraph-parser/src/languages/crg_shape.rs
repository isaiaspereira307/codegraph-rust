// ABOUTME: Reshapes an extraction into code-review-graph's graph shape for every language.
// ABOUTME: Adds the File node, class-qualified names and CONTAINS edges from file/class to members.

use codegraph_core::{
    CodeNode, EdgeRelationship, EdgeType, ExtractionResult, Language, Location, NodeType,
};
use std::collections::HashMap;

/// Set on every node whose `qualified_name` this pass rewrote.
pub const SHAPED_ATTR: &str = "crg_shaped";
/// The `qualified_name` a rewritten node had before this pass, when it had one.
pub const PRE_SHAPE_QN_ATTR: &str = "pre_shape_qualified_name";

/// The `qualified_name` the indexer must use when deriving node ids: the value from before
/// this pass, so node ids stay byte-identical to the unshaped extraction.
pub fn id_qualified_name(node: &CodeNode) -> Option<&String> {
    let attributes = &node.metadata.attributes;
    if attributes.contains_key(SHAPED_ATTR) {
        attributes.get(PRE_SHAPE_QN_ATTR)
    } else {
        attributes.get("qualified_name")
    }
}

fn is_container(kind: &NodeType) -> bool {
    matches!(
        kind,
        NodeType::Class | NodeType::Struct | NodeType::Trait | NodeType::Interface | NodeType::Enum
    )
}

fn is_member(kind: &NodeType) -> bool {
    is_container(kind) || matches!(kind, NodeType::Function | NodeType::Type)
}

/// `(start, end)` positions as `(line, column)` pairs.
fn extent(node: &CodeNode) -> ((u32, u32), (u32, u32)) {
    let loc = &node.location;
    (
        (loc.line, loc.column),
        (
            loc.end_line.unwrap_or(loc.line),
            loc.end_column.unwrap_or(loc.column),
        ),
    )
}

/// Adds the `File` node, rewrites `qualified_name`/`parent_name` of functions and types to
/// `<file>::<Outer.Inner>.<name>` and links each to its file or innermost enclosing class with
/// `CONTAINS`, as code-review-graph does for every language.
pub fn shape_for_crg(
    result: &mut ExtractionResult,
    file_path: &str,
    language: Language,
    line_count: u32,
) {
    let members: Vec<usize> = (0..result.nodes.len())
        .filter(|&i| result.nodes[i].node_type.as_ref().is_some_and(is_member))
        .collect();
    let containers: Vec<usize> = members
        .iter()
        .copied()
        .filter(|&i| result.nodes[i].node_type.as_ref().is_some_and(is_container))
        .collect();

    // Innermost enclosing container of each member, by span.
    let parent_of: HashMap<usize, usize> = members
        .iter()
        .filter_map(|&child| {
            let (start, end) = extent(&result.nodes[child]);
            containers
                .iter()
                .copied()
                .filter(|&c| c != child)
                .filter(|&c| {
                    let (c_start, c_end) = extent(&result.nodes[c]);
                    c_start <= start && end <= c_end && (c_start, c_end) != (start, end)
                })
                .max_by_key(|&c| {
                    let (c_start, c_end) = extent(&result.nodes[c]);
                    (c_start, std::cmp::Reverse(c_end))
                })
                .map(|c| (child, c))
        })
        .collect();

    // Dotted class path (`Outer.Inner`) of each member, outermost first.
    let mut paths: HashMap<usize, String> = HashMap::new();
    fn path_of(
        i: usize,
        nodes: &[CodeNode],
        parent_of: &HashMap<usize, usize>,
        paths: &mut HashMap<usize, String>,
        depth: usize,
    ) -> String {
        if let Some(path) = paths.get(&i) {
            return path.clone();
        }
        let name = nodes[i].name.to_string();
        let path = match parent_of.get(&i) {
            Some(&p) if depth < 64 => {
                format!("{}.{name}", path_of(p, nodes, parent_of, paths, depth + 1))
            }
            _ => name,
        };
        paths.insert(i, path.clone());
        path
    }
    for &i in &members {
        path_of(i, &result.nodes, &parent_of, &mut paths, 0);
    }

    let mut file_node = CodeNode::new(
        file_path.to_string(),
        Some(NodeType::Other("File".into())),
        Some(language),
        Location {
            file_path: file_path.to_string(),
            line: 1,
            column: 0,
            end_line: Some(line_count),
            end_column: Some(0),
        },
    );
    file_node
        .metadata
        .attributes
        .insert("qualified_name".into(), file_path.to_string());

    for &i in &members {
        let qualified = format!("{file_path}::{}", paths[&i]);
        let parent = parent_of.get(&i).map(|&p| paths[&p].clone());
        let from = parent_of
            .get(&i)
            .map_or(file_node.id, |&p| result.nodes[p].id);
        let node = &mut result.nodes[i];
        let attributes = &mut node.metadata.attributes;
        if let Some(original) = attributes.insert("qualified_name".into(), qualified.clone()) {
            attributes.insert(PRE_SHAPE_QN_ATTR.into(), original);
        }
        attributes.insert(SHAPED_ATTR.into(), "true".into());
        if let Some(parent) = parent {
            attributes.insert("parent_name".into(), parent);
        }
        result.edges.push(EdgeRelationship {
            from,
            to: qualified,
            edge_type: EdgeType::Contains,
            metadata: HashMap::from([
                ("source_file".into(), file_path.to_string()),
                ("target_node_id".into(), node.id.to_string()),
                ("line".into(), node.location.line.to_string()),
                ("analyzer".into(), "crg_shape".into()),
            ]),
            span: None,
        });
    }

    result.nodes.insert(0, file_node);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::extract_for_language;
    use std::collections::BTreeSet;

    fn extract_python(source: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        extract_for_language(&Language::Python, &tree, source, "m.py").unwrap()
    }

    fn qualified_names(result: &ExtractionResult) -> BTreeSet<String> {
        result
            .nodes
            .iter()
            .filter_map(|n| n.metadata.attributes.get("qualified_name").cloned())
            .collect()
    }

    fn contains_edges(result: &ExtractionResult) -> BTreeSet<(String, String)> {
        result
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Contains)
            .map(|e| {
                let from = result.nodes.iter().find(|n| n.id == e.from).unwrap();
                (
                    from.metadata.attributes["qualified_name"].clone(),
                    e.to.clone(),
                )
            })
            .collect()
    }

    fn set(items: &[(&str, &str)]) -> BTreeSet<(String, String)> {
        items
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn methods_get_class_prefix_and_contains() {
        let r = extract_python(
            "class A:\n    def run(self):\n        pass\n\n\
             class B:\n    def run(self):\n        pass\n\n\
             def top():\n    pass\n",
        );
        let qns = qualified_names(&r);
        assert!(qns.contains("m.py::A.run") && qns.contains("m.py::B.run"));
        assert_eq!(
            contains_edges(&r),
            set(&[
                ("m.py", "m.py::A"),
                ("m.py", "m.py::B"),
                ("m.py", "m.py::top"),
                ("m.py::A", "m.py::A.run"),
                ("m.py::B", "m.py::B.run"),
            ])
        );
        let run_a = r
            .nodes
            .iter()
            .find(|n| {
                n.metadata
                    .attributes
                    .get("qualified_name")
                    .map(String::as_str)
                    == Some("m.py::A.run")
            })
            .unwrap();
        assert_eq!(run_a.name.as_str(), "run");
        assert_eq!(run_a.metadata.attributes["parent_name"], "A");
    }

    #[test]
    fn nested_classes_are_dotted() {
        let r = extract_python(
            "class Outer:\n    class Inner:\n        def go(self):\n            pass\n",
        );
        let qns = qualified_names(&r);
        assert!(qns.contains("m.py::Outer.Inner"));
        assert!(qns.contains("m.py::Outer.Inner.go"));
        assert!(contains_edges(&r).contains(&(
            "m.py::Outer.Inner".to_string(),
            "m.py::Outer.Inner.go".to_string()
        )));
    }

    #[test]
    fn nested_function_is_child_of_file() {
        let r = extract_python("def outer():\n    def inner():\n        pass\n    inner()\n");
        let qns = qualified_names(&r);
        assert!(qns.contains("m.py::outer") && qns.contains("m.py::inner"));
        assert_eq!(
            contains_edges(&r),
            set(&[("m.py", "m.py::outer"), ("m.py", "m.py::inner")])
        );
    }

    #[test]
    fn empty_file_still_has_file_node() {
        let r = extract_python("");
        let files: Vec<_> = r
            .nodes
            .iter()
            .filter(|n| n.node_type == Some(NodeType::Other("File".into())))
            .collect();
        assert_eq!(files.len(), 1);
        let file = files[0];
        assert_eq!(file.name.as_str(), "m.py");
        assert_eq!(file.metadata.attributes["qualified_name"], "m.py");
        assert_eq!((file.location.line, file.location.end_line), (1, Some(1)));
    }

    #[test]
    fn file_node_spans_every_line() {
        let r = extract_python("x = 1\ny = 2\nz = 3\n");
        let file = r
            .nodes
            .iter()
            .find(|n| n.node_type == Some(NodeType::Other("File".into())))
            .unwrap();
        assert_eq!((file.location.line, file.location.end_line), (1, Some(4)));
    }

    #[test]
    fn contains_edges_carry_the_child_id() {
        let r = extract_python("class A:\n    def run(self):\n        pass\n");
        for edge in r.edges.iter().filter(|e| e.edge_type == EdgeType::Contains) {
            let target = edge.metadata["target_node_id"].clone();
            let child = r.nodes.iter().find(|n| n.id.to_string() == target).unwrap();
            assert_eq!(child.metadata.attributes["qualified_name"], edge.to);
        }
    }

    #[test]
    fn id_qualified_name_is_the_pre_shape_value() {
        let r = extract_python("class A:\n    def run(self):\n        pass\n");
        let run = r.nodes.iter().find(|n| n.name.as_str() == "run").unwrap();
        assert_eq!(run.metadata.attributes["qualified_name"], "m.py::A.run");
        assert_eq!(id_qualified_name(run), None);

        let source = "struct S;\nimpl S {\n    fn go(&self) {}\n}\n";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let r = extract_for_language(&Language::Rust, &tree, source, "m.rs").unwrap();
        let go = r.nodes.iter().find(|n| n.name.as_str() == "go").unwrap();
        assert_eq!(go.metadata.attributes["qualified_name"], "m.rs::go");
        assert_eq!(
            id_qualified_name(go).map(String::as_str),
            Some("m.rs::impl<S>::go")
        );
    }
}
